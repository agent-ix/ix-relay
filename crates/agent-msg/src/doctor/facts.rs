//! What `doctor` looks at, gathered without changing anything.
//!
//! [`gather`] reads Redis (a bounded set of read-only commands), the local state
//! file and spool, herdr and the service manager, and returns them as plain data
//! for [`super::checks::evaluate`]. Every read failing is data too: a Redis that
//! cannot be reached is [`RedisFacts::Down`], not an error, and the local
//! facts are still gathered.
//!
//! # Time
//!
//! `doctor` answers within [`REDIS_BUDGET`] plus [`PROBE_BUDGET`] (about four
//! seconds) however the machine and the bus are failing. Redis is read first, its
//! connect and every read armed with what is left of the budget, and a read that
//! finds it spent reports that instead of waiting. The child processes (herdr's
//! check and listing, the service manager) run after it, in parallel with each
//! other, each call killed at its own deadline.
//!
//! The two are not overlapped, and this is measured, not a guess: with Redis read
//! in parallel with the children, a child's exit (`SIGCHLD`) interrupted the
//! Redis socket read (`EINTR`, "Interrupted system call") in about 1 run in 60
//! (7 of 400 and 6 of 600, at connect, `SCAN`, `HGETALL` and pipelines);
//! with `SIGCHLD` blocked in the Redis thread, or with Redis read first as here,
//! 0 of 600 and 0 of 400. The program installs no signal handler; the mechanism
//! is (not proven) that glibc blocks signals in the thread that spawns, so the
//! child's `SIGCHLD` is queued for the process and wakes the other thread, whose
//! `recv` on a socket with a receive timeout returns `EINTR` instead of being
//! restarted, and redis-rs does not retry it. `relay-tui` (a Redis poll thread
//! beside a thread that spawns `herdr` and `linear`) has the same exposure and
//! takes the other way out: its poll only reads on a fresh connection, so it
//! repeats an interrupted read ([`relay_core::Error::is_interrupted`], AGE-2027).

use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use herdr_cli::{ProcessRunner, RealProcessRunner};
use relay_core::{
    AgentName, Backlog, Heartbeats, InboxOverview, LeaseState, LinkFault, MachineName, Moves,
    PaneId, Registration, Relay, config::DeliveryMode, health::WatcherFile,
    socket_status::SocketStatus,
};
use relay_ops::herdr_check::{self, HerdrCheck};

use crate::spool::SpoolRecord;

/// The systemd user unit `deploy/relay-watcher.service` installs.
pub(crate) const SYSTEMD_UNIT: &str = "relay-watcher.service";

/// The launchd label `deploy/com.agent-ix.relay-watcher.plist` installs.
pub(crate) const LAUNCHD_LABEL: &str = "com.agent-ix.relay-watcher";

/// How long the whole Redis section may take: the connect and every read share it.
pub(crate) const REDIS_BUDGET: Duration = Duration::from_secs(2);

/// The longest one Redis read or connect may wait.
const REDIS_PER_CALL: Duration = Duration::from_secs(1);

/// How long each herdr or service-manager call may take (a whole second: the
/// timeout is reported in seconds).
const PROBE_TIMEOUT: Duration = Duration::from_secs(1);

/// How long herdr's whole version and shape check may take.
pub(crate) const PROBE_BUDGET: Duration = Duration::from_secs(2);

/// The most hook failure records read from the bus (the stream is capped near
/// this).
const MAX_HOOK_RECORDS: usize = 1000;

/// The most recent moves `doctor` reads. The read is by time (the last 24 hours),
/// so a busier stream is reported as "at least this many", never cut silently.
const MAX_MOVES: usize = 200;

/// The most bytes of the spool read: twice its own bound, so a spool that grew
/// past it is still read, not slurped.
const MAX_SPOOL_READ: u64 = 512 * 1024;

/// Where `doctor` looks and how it reaches the bus.
#[derive(Debug, Clone)]
pub(crate) struct Env {
    /// This machine's bus name.
    pub machine: MachineName,
    /// The Redis URL, with its credentials.
    pub url: String,
    /// The bus key prefix.
    pub prefix: String,
    /// `state/watcher.json`, when a config location is known.
    pub state_file: Option<PathBuf>,
    /// `spool/hook.jsonl`, when a config location is known.
    pub spool_file: Option<PathBuf>,
    /// The time the Redis section may take.
    pub redis_budget: Duration,
    /// `[watcher] delivery` from `config.toml`, or why the watcher would refuse it.
    pub delivery: Result<DeliveryMode, String>,
    /// The status directory (`state/socket`) holding `socket-<pid>.json`, when a config location is
    /// known.
    pub socket_status_dir: Option<PathBuf>,
}

/// The machine's own state that is not on the bus.
pub(crate) trait Probes: Sync {
    /// The herdr version and shape check.
    fn herdr_check(&self) -> HerdrCheck;
    /// The panes herdr has an agent in, or why they could not be listed.
    fn herdr_panes(&self) -> Result<HashSet<PaneId>, String>;
    /// The watcher unit's state.
    fn service(&self) -> ServiceState;
}

/// The watcher unit as its service manager reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ServiceState {
    /// Running.
    Active,
    /// Installed, not running (the manager's own word for it).
    Inactive(String),
    /// Not installed.
    NotInstalled,
    /// It could not be told, and why.
    Unknown(String),
}

/// What the local `watcher.json` holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FileState {
    /// No config location, so no state directory.
    NoPath,
    /// The file is not there.
    Missing,
    /// It could not be read or decoded.
    Unreadable(String),
    /// It decoded; `pid_alive` says whether that process runs, when it can be told.
    Read {
        file: WatcherFile,
        pid_alive: Option<bool>,
    },
}

/// Claude Code socket delivery, as `doctor` finds it (FR-044).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SocketFacts {
    /// `[watcher] delivery` is herdr (the default): sockets are not used.
    Herdr,
    /// `config.toml` did not load, so the mode is not known.
    ConfigFailed(String),
    /// It is `socket`: what each watcher last reported (one entry per status
    /// file, or one entry saying there is none).
    Socket(Vec<SocketState>),
}

/// What a watcher's `socket-<pid>.json` holds.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(
    clippy::large_enum_variant,
    reason = "one value per watcher, a handful at most; boxing the status would touch every construction site for nothing"
)]
pub(crate) enum SocketState {
    /// No config location, so no state directory.
    NoPath,
    /// No watcher has written one.
    Missing,
    /// It could not be read or decoded.
    Unreadable(String),
    /// The status directory (or file) is not one the watcher trusts (not a real directory of
    /// this user closed to others, a symlink, another user's file), so it is not read.
    Untrusted(String),
    /// It decoded; `pid_alive` says whether the watcher that wrote it runs.
    Read {
        status: SocketStatus,
        pid_alive: Option<bool>,
    },
}

/// Reads the status files when delivery is by socket.
fn read_socket_facts(
    delivery: &Result<DeliveryMode, String>,
    config_error: Option<&str>,
    dir: Option<&Path>,
) -> SocketFacts {
    if let Some(error) = config_error {
        return SocketFacts::ConfigFailed(error.to_owned());
    }
    let delivery = match delivery {
        Ok(mode) => *mode,
        Err(why) => return SocketFacts::ConfigFailed(why.clone()),
    };
    if delivery == DeliveryMode::Herdr {
        return SocketFacts::Herdr;
    }
    let Some(dir) = dir else {
        return SocketFacts::Socket(vec![SocketState::NoPath]);
    };
    let states: Vec<SocketState> = relay_core::socket_status::read_all(dir)
        .into_iter()
        .map(|(_, read)| match read {
            Ok(status) => SocketState::Read {
                pid_alive: watcher_alive(&status),
                status,
            },
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                SocketState::Untrusted(e.to_string())
            }
            Err(e) => SocketState::Unreadable(e.to_string()),
        })
        .collect();
    SocketFacts::Socket(if states.is_empty() {
        vec![SocketState::Missing]
    } else {
        states
    })
}

/// What the local hook spool holds.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SpoolState {
    /// No config location.
    NoPath,
    /// No spool file, or an empty one.
    Empty,
    /// It could not be read.
    Unreadable(String),
    /// Failures waiting to be shipped.
    Pending {
        bytes: u64,
        records: Vec<SpoolRecord>,
        undecodable: usize,
        /// When the file was last written, Unix milliseconds: the age of a spool
        /// with nothing in it that decodes.
        modified_ms: Option<i64>,
    },
}

/// One hook failure record on the bus.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BusFailure {
    /// The machine that spooled it.
    pub machine: MachineName,
    /// The record, when it decodes.
    pub record: Option<SpoolRecord>,
}

/// Reads of the bus, each of which can fail on its own.
#[derive(Debug, Clone)]
pub(crate) struct RedisUp {
    /// Connect and `PING` time.
    pub rtt: Duration,
    /// Decodable registrations, and how many records were skipped.
    pub registrations: Result<(Vec<Registration>, usize), String>,
    /// This machine's, every machine with a registered lead's, and any other
    /// machine's heartbeat key.
    pub heartbeats: Result<Heartbeats, String>,
    /// Each registered lead's inbox.
    pub inboxes: Vec<(AgentName, Result<InboxOverview, String>)>,
    /// Every lead with in-flight entries or dead letters, found by the one
    /// bounded key scan `inbox --in-flight` uses.
    pub backlog: Result<Backlog, String>,
    /// The lease state of each lead that has entries in flight.
    pub leases: Result<Vec<LeaseState>, String>,
    /// Hook failures recorded on the bus.
    pub hook_failures: Result<Vec<BusFailure>, String>,
    /// Health-stream entries the read could not use (a missing field, a machine
    /// that is not a machine name).
    pub hook_unreadable: usize,
    /// The relinks and renames the watchers recorded in the last 24 hours (FR-043).
    pub moves: Result<MovesRead, String>,
}

/// What reading the recent moves found.
#[derive(Debug, Clone, Default)]
pub(crate) struct MovesRead {
    /// The newest events, at most [`MAX_MOVES`].
    pub moves: Moves,
    /// Whether the last 24 hours hold more than were read.
    pub more: bool,
}

/// Whether the bus could be read.
#[derive(Debug, Clone)]
pub(crate) enum RedisFacts {
    /// `config.toml` did not load, so no bus was tried.
    NotAttempted,
    /// It could not be reached: why, and the error text scrubbed of
    /// credentials.
    Down(LinkFault, String),
    /// It answered.
    Up(Box<RedisUp>),
}

/// Everything the checks are made from.
#[derive(Debug, Clone)]
pub(crate) struct Facts {
    /// Now, Unix milliseconds.
    pub now_ms: i64,
    /// This machine.
    pub machine: MachineName,
    /// The Redis URL with credentials redacted.
    pub redis_url: String,
    /// Why `config.toml` did not load, if it did not.
    pub config_error: Option<String>,
    /// Why the watcher would refuse the file's OTLP keys, when it would.
    pub watcher_config_problem: Option<String>,
    /// The bus.
    pub redis: RedisFacts,
    /// The local state file.
    pub watcher_file: FileState,
    /// The local spool.
    pub spool: SpoolState,
    /// The herdr check.
    pub herdr: HerdrCheck,
    /// herdr's panes.
    pub panes: Result<HashSet<PaneId>, String>,
    /// The watcher unit.
    pub service: ServiceState,
    /// Claude Code socket delivery.
    pub socket: SocketFacts,
}

/// `text` with the URL's credentials removed: every place the URL, or its
/// password, appears is replaced.
pub(crate) fn scrub(text: &str, url: &str) -> String {
    let mut out = text.replace(url, &relay_core::config::redact_url(url));
    if let Some((_, rest)) = url.split_once("://")
        && let Some((userinfo, _)) = rest.rsplit_once('@')
    {
        let password = userinfo.split_once(':').map_or(userinfo, |(_, p)| p);
        if !password.is_empty() {
            out = out.replace(password, "***");
        }
    }
    out
}

/// Gathers the facts. `now_ms` is when the report is made.
pub(crate) fn gather(
    env: &Env,
    config_error: Option<String>,
    probes: &dyn Probes,
    now_ms: i64,
) -> Facts {
    // Redis first, alone; then the child processes together (see the module docs).
    let redis = if config_error.is_some() {
        RedisFacts::NotAttempted
    } else {
        gather_redis(env)
    };
    let (herdr, panes, service) = thread::scope(|scope| {
        let herdr = scope.spawn(|| probes.herdr_check());
        let panes = scope.spawn(|| probes.herdr_panes());
        let service = scope.spawn(|| probes.service());
        let herdr = herdr.join().unwrap_or_else(|_| HerdrCheck::Failed {
            reason: "the herdr check panicked".to_owned(),
        });
        let panes = panes
            .join()
            .unwrap_or_else(|_| Err("listing herdr's panes panicked".to_owned()));
        let service = service
            .join()
            .unwrap_or_else(|_| ServiceState::Unknown("the unit check panicked".to_owned()));
        (herdr, panes, service)
    });
    let socket = read_socket_facts(
        &env.delivery,
        config_error.as_deref(),
        env.socket_status_dir.as_deref(),
    );
    Facts {
        now_ms,
        machine: env.machine.clone(),
        redis_url: relay_core::config::redact_url(&env.url),
        watcher_config_problem: if config_error.is_none() {
            relay_core::config::otlp::file_problem()
        } else {
            None
        },
        config_error,
        redis,
        watcher_file: read_watcher_file(env.state_file.as_deref()),
        spool: read_spool(env.spool_file.as_deref()),
        herdr,
        panes,
        service,
        socket,
    }
}

/// The time left of a Redis section's budget: arms each read, and refuses one
/// when it is spent.
struct Budget {
    started: Instant,
    total: Duration,
}

impl Budget {
    fn left(&self) -> Duration {
        self.total.saturating_sub(self.started.elapsed())
    }

    /// Arms `relay`'s timeout for the next read: what is left, at most
    /// [`REDIS_PER_CALL`]. `Err` when nothing is left.
    fn arm(&self, relay: &Relay) -> Result<(), String> {
        let left = self.left();
        if left.is_zero() {
            return Err("doctor's time budget for Redis was spent".to_owned());
        }
        relay
            .set_timeout(left.min(REDIS_PER_CALL))
            .map_err(|e| e.to_string())
    }
}

/// Connects and reads the bus. Any failure to connect or `PING` is
/// [`RedisFacts::Down`]; a failing read after that is that read's own error.
fn gather_redis(env: &Env) -> RedisFacts {
    let budget = Budget {
        started: Instant::now(),
        total: env.redis_budget,
    };
    let scrubbed = |e: relay_core::Error| {
        if e.is_timeout() {
            "timed out".to_owned()
        } else {
            scrub(&e.to_string(), &env.url)
        }
    };
    let down = |e: relay_core::Error| RedisFacts::Down(e.link_fault(), scrubbed(e));
    let mut relay = match Relay::connect(&env.url, env.redis_budget.min(REDIS_PER_CALL)) {
        Ok(relay) => relay.with_prefix(env.prefix.clone()),
        Err(e) => return down(e),
    };
    if let Err(e) = budget.arm(&relay) {
        return RedisFacts::Down(LinkFault::Other, e.to_string());
    }
    if let Err(e) = relay.ping() {
        return down(e);
    }
    let rtt = budget.started.elapsed();
    // One deadline for the whole section, however many calls a scan makes.
    relay.set_scan_deadline(Some(budget.started + budget.total));
    let reader = Reader {
        budget: &budget,
        scrub: &scrubbed,
    };
    let registrations = reader.run(&mut relay, |r| {
        r.agents().map(|list| {
            let skipped = list.skipped.len();
            (list.registrations, skipped)
        })
    });
    // Heartbeats are read for this machine and for every machine with a lead, by
    // name, and scanned for the rest.
    let mut named = vec![env.machine.clone()];
    if let Ok((regs, _)) = &registrations {
        named.extend(regs.iter().map(|r| r.machine.clone()));
    }
    let heartbeats = reader.run(&mut relay, |r| r.heartbeats(&named));
    let inboxes = match &registrations {
        Ok((regs, _)) => {
            let names: Vec<&AgentName> = regs.iter().map(|r| &r.name).collect();
            match reader.run(&mut relay, |r| r.inbox_overviews(&names)) {
                Ok(overviews) => names
                    .into_iter()
                    .cloned()
                    .zip(overviews.into_iter().map(|o| o.map_err(scrubbed)))
                    .collect(),
                Err(text) => names
                    .into_iter()
                    .map(|n| (n.clone(), Err(text.clone())))
                    .collect(),
            }
        }
        Err(_) => Vec::new(),
    };
    let backlog = reader.run(&mut relay, |r| r.backlog());
    let leases = match &backlog {
        Ok(found) => {
            // A key whose suffix is not a bus name is nobody's in-flight list.
            let leads: Vec<AgentName> = found
                .leads
                .iter()
                .filter(|l| l.in_flight > 0)
                .filter_map(|l| l.lead.parse().ok())
                .collect();
            reader.run(&mut relay, |r| r.overdue_leases(&leads))
        }
        Err(e) => Err(e.clone()),
    };
    let day_ago = wall_ms().saturating_sub(24 * 60 * 60 * 1000);
    let moves = reader
        .run(&mut relay, |r| r.moves_since(day_ago, MAX_MOVES))
        .map(|(moves, more)| MovesRead { moves, more });
    let health = reader.run(&mut relay, |r| r.hook_health(MAX_HOOK_RECORDS));
    let hook_unreadable = health.as_ref().map_or(0, |h| h.unreadable);
    let hook_failures = health.map(|h| {
        h.entries
            .into_iter()
            .map(|e| BusFailure {
                machine: e.machine,
                record: serde_json::from_str(&e.record).ok(),
            })
            .collect()
    });
    RedisFacts::Up(Box::new(RedisUp {
        rtt,
        registrations,
        heartbeats,
        inboxes,
        backlog,
        leases,
        hook_failures,
        hook_unreadable,
        moves,
    }))
}

/// One Redis read: armed with what is left of the budget, its error scrubbed of
/// the URL's credentials.
struct Reader<'a> {
    budget: &'a Budget,
    scrub: &'a dyn Fn(relay_core::Error) -> String,
}

impl Reader<'_> {
    fn run<T>(
        &self,
        relay: &mut Relay,
        f: impl FnOnce(&mut Relay) -> relay_core::Result<T>,
    ) -> Result<T, String> {
        self.budget.arm(relay)?;
        f(relay).map_err(self.scrub)
    }
}

/// Whether process `pid` runs, where that can be told without a system call this
/// crate does not have (`/proc`, Linux only).
fn pid_alive(pid: u32) -> Option<bool> {
    let proc = Path::new("/proc");
    proc.join("self")
        .exists()
        .then(|| proc.join(pid.to_string()).exists())
}

/// Whether the watcher that wrote `status` runs: its pid exists and, when the status
/// records its start time, the process there started then (a pid that a later process
/// reuses is another process). `None` where that cannot be told (no `/proc`).
fn watcher_alive(status: &SocketStatus) -> Option<bool> {
    let exists = pid_alive(status.pid)?;
    if !exists {
        return Some(false);
    }
    match (
        &status.start_time,
        relay_core::socket_status::proc_start_time(status.pid),
    ) {
        (Some(then), Some(now)) => Some(*then == now),
        (Some(_), None) => Some(false),
        (None, _) => Some(true),
    }
}

/// Reads `watcher.json`.
fn read_watcher_file(path: Option<&Path>) -> FileState {
    let Some(path) = path else {
        return FileState::NoPath;
    };
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return FileState::Missing,
        Err(e) => return FileState::Unreadable(e.to_string()),
    };
    match serde_json::from_slice::<WatcherFile>(&bytes) {
        Ok(file) => FileState::Read {
            pid_alive: pid_alive(file.pid),
            file,
        },
        Err(e) => FileState::Unreadable(e.to_string()),
    }
}

/// Reads the spool without following a symlink and without holding its lock: a
/// snapshot, at most [`MAX_SPOOL_READ`] bytes.
fn read_spool(path: Option<&Path>) -> SpoolState {
    let Some(path) = path else {
        return SpoolState::NoPath;
    };
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return SpoolState::Empty,
        Err(e) => return SpoolState::Unreadable(e.to_string()),
    };
    if !meta.is_file() {
        return SpoolState::Unreadable("the spool path is not a regular file".to_owned());
    }
    if meta.len() == 0 {
        return SpoolState::Empty;
    }
    let mut bytes = Vec::new();
    let read = fs::File::open(path).and_then(|f| f.take(MAX_SPOOL_READ).read_to_end(&mut bytes));
    if let Err(e) = read {
        return SpoolState::Unreadable(e.to_string());
    }
    let mut records = Vec::new();
    let mut undecodable = 0;
    for line in bytes.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
        match serde_json::from_slice(line) {
            Ok(record) => records.push(record),
            Err(_) => undecodable += 1,
        }
    }
    SpoolState::Pending {
        bytes: meta.len(),
        records,
        undecodable,
        modified_ms: meta.modified().ok().and_then(|t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .ok()
                .and_then(|d| i64::try_from(d.as_millis()).ok())
        }),
    }
}

/// The real herdr and service manager on this machine.
pub(crate) struct RealProbes;

impl Probes for RealProbes {
    fn herdr_check(&self) -> HerdrCheck {
        herdr_check::check_within(&RealProcessRunner, PROBE_TIMEOUT, PROBE_BUDGET)
    }

    fn herdr_panes(&self) -> Result<HashSet<PaneId>, String> {
        use relay_ops::find::{FindHerdr, RealFindHerdr};
        let listed = RealFindHerdr::new()
            .agent_list(None, PROBE_TIMEOUT)
            .map_err(|e| e.to_string())?;
        // A dropped entry may be a registered pane: with any dropped, the set is
        // not the panes herdr has, and no registration is judged against it.
        if listed.dropped > 0 {
            return Err(format!(
                "{} herdr agent entr{} could not be decoded",
                listed.dropped,
                if listed.dropped == 1 { "y" } else { "ies" }
            ));
        }
        Ok(listed.entries.into_iter().map(|a| a.pane_id).collect())
    }

    fn service(&self) -> ServiceState {
        service_state(&RealProcessRunner)
    }
}

/// Asks systemd (`--user`), or launchd where there is no systemctl.
pub(crate) fn service_state(runner: &impl ProcessRunner) -> ServiceState {
    match runner.run(
        "systemctl",
        &["--user", "is-active", SYSTEMD_UNIT],
        PROBE_TIMEOUT,
    ) {
        Ok(out) if out.success => ServiceState::Active,
        Ok(out) => {
            let word = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            let stderr = String::from_utf8_lossy(&out.stderr);
            if stderr.contains("Failed to connect") || word.is_empty() {
                return ServiceState::Unknown(format!(
                    "systemctl --user could not be asked: {}",
                    stderr.trim()
                ));
            }
            match runner.run("systemctl", &["--user", "cat", SYSTEMD_UNIT], PROBE_TIMEOUT) {
                Ok(cat) if cat.success => ServiceState::Inactive(word),
                Ok(_) => ServiceState::NotInstalled,
                Err(e) => ServiceState::Unknown(e.to_string()),
            }
        }
        Err(herdr_cli::ProcessError::Spawn { .. }) => launchd_state(runner),
        Err(e) => ServiceState::Unknown(e.to_string()),
    }
}

/// launchd's view of [`LAUNCHD_LABEL`].
fn launchd_state(runner: &impl ProcessRunner) -> ServiceState {
    match runner.run("launchctl", &["list", LAUNCHD_LABEL], PROBE_TIMEOUT) {
        Ok(out) if out.success => {
            if String::from_utf8_lossy(&out.stdout).contains("\"PID\"") {
                ServiceState::Active
            } else {
                ServiceState::Inactive("loaded, not running".to_owned())
            }
        }
        Ok(_) => ServiceState::NotInstalled,
        Err(herdr_cli::ProcessError::Spawn { .. }) => {
            ServiceState::Unknown("neither systemctl nor launchctl is available".to_owned())
        }
        Err(e) => ServiceState::Unknown(e.to_string()),
    }
}

/// Unix milliseconds now, for reading a stream by time.
fn wall_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use herdr_cli::{Output, ProcessError};

    use super::*;

    /// Answers by `program arg arg ..`; a program with no entry cannot be spawned.
    struct Runner(HashMap<&'static str, (i32, &'static str, &'static str)>);

    impl ProcessRunner for Runner {
        fn run(&self, program: &str, args: &[&str], _: Duration) -> Result<Output, ProcessError> {
            let key = std::iter::once(program)
                .chain(args.iter().copied())
                .collect::<Vec<_>>()
                .join(" ");
            match self.0.get(key.as_str()) {
                Some((code, out, err)) => Ok(Output {
                    status_code: *code,
                    success: *code == 0,
                    stdout: out.as_bytes().to_vec(),
                    stderr: err.as_bytes().to_vec(),
                }),
                None => Err(ProcessError::Spawn {
                    program: program.to_owned(),
                    reason: "not found".into(),
                }),
            }
        }
    }

    const IS_ACTIVE: &str = "systemctl --user is-active relay-watcher.service";
    const CAT: &str = "systemctl --user cat relay-watcher.service";
    const LIST: &str = "launchctl list com.agent-ix.relay-watcher";

    #[test]
    fn the_service_state_is_read_from_systemd_or_launchd_and_unknown_when_neither_answers() {
        let state = |entries: &[(&'static str, (i32, &'static str, &'static str))]| {
            service_state(&Runner(entries.iter().copied().collect()))
        };
        assert_eq!(
            state(&[(IS_ACTIVE, (0, "active\n", ""))]),
            ServiceState::Active
        );
        assert_eq!(
            state(&[(IS_ACTIVE, (3, "inactive\n", "")), (CAT, (0, "[Unit]", ""))]),
            ServiceState::Inactive("inactive".into())
        );
        assert_eq!(
            state(&[
                (IS_ACTIVE, (3, "inactive\n", "")),
                (CAT, (1, "", "No files found"))
            ]),
            ServiceState::NotInstalled
        );
        assert!(matches!(
            state(&[(IS_ACTIVE, (1, "", "Failed to connect to bus"))]),
            ServiceState::Unknown(_)
        ));
        assert_eq!(
            state(&[(LIST, (0, "{\n\t\"PID\" = 12;\n}", ""))]),
            ServiceState::Active
        );
        assert!(matches!(
            state(&[(LIST, (0, "{\n\t\"LastExitStatus\" = 0;\n}", ""))]),
            ServiceState::Inactive(_)
        ));
        assert_eq!(state(&[(LIST, (113, "", ""))]), ServiceState::NotInstalled);
        assert!(matches!(state(&[]), ServiceState::Unknown(_)));
    }

    #[test]
    fn the_url_and_its_password_are_scrubbed_from_any_text() {
        let url = "redis://user:s3cretpw@127.0.0.1:6379";
        let text = format!("failed to open {url}: bad password s3cretpw");
        let clean = scrub(&text, url);
        assert!(!clean.contains("s3cretpw"), "{clean}");
        assert!(clean.contains("redis://***@127.0.0.1:6379"), "{clean}");
        assert_eq!(scrub("no secret", "redis://127.0.0.1:6379"), "no secret");
    }

    #[test]
    fn the_spool_is_read_without_being_changed_and_a_symlink_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let spool = dir.path().join("hook.jsonl");
        assert_eq!(read_spool(Some(&spool)), SpoolState::Empty);
        assert_eq!(read_spool(None), SpoolState::NoPath);
        let record = r#"{"at_ms":1,"machine":"hub","lead":null,"event":null,"session_id":null,"stage":"pop","error_kind":"timeout"}"#;
        let body = format!("{record}\nnot json\n{record}\n");
        fs::write(&spool, &body).unwrap();
        let SpoolState::Pending {
            records,
            undecodable,
            bytes,
            ..
        } = read_spool(Some(&spool))
        else {
            panic!("records are waiting");
        };
        assert_eq!((records.len(), undecodable), (2, 1));
        assert_eq!(bytes, u64::try_from(body.len()).unwrap());
        assert_eq!(fs::read_to_string(&spool).unwrap(), body, "read-only");

        let link = dir.path().join("link.jsonl");
        std::os::unix::fs::symlink(&spool, &link).unwrap();
        assert!(matches!(read_spool(Some(&link)), SpoolState::Unreadable(_)));
    }

    #[test]
    fn the_state_file_reads_or_says_why_not() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("watcher.json");
        assert_eq!(read_watcher_file(Some(&path)), FileState::Missing);
        fs::write(&path, "{}").unwrap();
        assert!(matches!(
            read_watcher_file(Some(&path)),
            FileState::Unreadable(_)
        ));
        let file = WatcherFile {
            pid: std::process::id(),
            started_at_ms: 1,
            last_ok_tick_ms: Some(2),
            last_error: None,
        };
        fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();
        let FileState::Read { file: read, .. } = read_watcher_file(Some(&path)) else {
            panic!("a valid file reads");
        };
        assert_eq!(read, file);
    }
}

#[cfg(test)]
mod socket_tests {
    use super::*;

    /// Tracing: TC-044-09
    /// ACs: FR-044-AC-9
    ///
    /// A status directory that is open to others is reported as untrusted, not as a report
    /// that cannot be read.
    #[test]
    fn tc_044_09_an_open_status_directory_is_untrusted_in_the_facts() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let status = dir.path().join("socket");
        fs::create_dir(&status).unwrap();
        fs::set_permissions(&status, fs::Permissions::from_mode(0o777)).unwrap();
        let facts = read_socket_facts(&Ok(DeliveryMode::Socket), None, Some(&status));
        assert!(
            matches!(&facts, SocketFacts::Socket(v) if matches!(v.as_slice(), [SocketState::Untrusted(_)])),
            "{facts:?}"
        );
    }

    /// Tracing: TC-044-09
    /// ACs: FR-044-AC-9
    ///
    /// A watcher is alive by pid and start time: a pid that a later process reuses (its
    /// start time differs) is a dead watcher, and a status with no start time is judged
    /// by the pid alone.
    #[cfg(target_os = "linux")]
    #[test]
    fn tc_044_09_a_reused_pid_is_not_the_watcher_that_wrote_the_status() {
        let own = std::process::id();
        let start = relay_core::socket_status::proc_start_time(own);
        assert!(start.is_some(), "this needs /proc");
        let status = |pid, start_time: Option<String>| SocketStatus {
            pid,
            start_time,
            ..SocketStatus::default()
        };
        assert_eq!(watcher_alive(&status(own, start.clone())), Some(true));
        assert_eq!(
            watcher_alive(&status(own, Some("1".into()))),
            Some(false),
            "same pid, another start time: reused"
        );
        assert_eq!(watcher_alive(&status(own, None)), Some(true));
        assert_eq!(watcher_alive(&status(u32::MAX - 1, start)), Some(false));
    }
}
