//! `relay-watcher`: one process per machine that wakes idle team leads.
//!
//! `agent-msg`'s hooks deliver messages mid-turn (`PostToolUse`) or at turn
//! end (`Stop`), but a lead that is simply sitting idle between turns never
//! runs a hook, so nothing wakes it up. This watches the leads registered on
//! this machine (see [`relay_core::config`], AGE-2017) and, for any that are
//! idle or done with a non-empty inbox, types the queued messages in through
//! a [`deliver::Deliverer`] (`herdr agent prompt`).
//!
//! # Waking
//!
//! The watcher does not poll on a timer. Every send appends to the message log
//! stream, so [`RedisBus::wait`] blocks on `XREAD BLOCK` for it and a message
//! is looked at as soon as it is sent. Two timers cover what a send cannot:
//! a lead left for its hook (working, blocked, mid-turn) has mail queued and
//! nothing new coming, so it has its own retry timer ([`WatcherState::retry_at`],
//! every `interval_secs`); and a backup sweep (`--sweep-secs`) looks at every
//! lead regardless, so a wake lost to a dropped connection or a restarted Redis
//! costs seconds, not a message. A new connection reads the log's tail *before*
//! its first sweep, so a send in between still wakes the next wait.
//!
//! Working leads are left alone (their hook will deliver); blocked leads are
//! left alone and the operator is notified once per blocked episode via `herdr
//! notification show`. See [`watch::try_deliver`] for the per-lead decision.
//!
//! The process never exits because Redis is unhealthy: [`supervise`] retries
//! forever with capped backoff. The only exits are a command-line error and a
//! configuration error at startup (and `--once`, which is for manual checks), and a
//! clean stop on SIGTERM or SIGINT, noticed at the next turn of the loop (a call the
//! signal interrupts is not logged or recorded as a failure).

mod deliver;
mod episodes;
mod herdr;
mod herdr_gate;
mod last_seen;
mod liveness;
mod otlp;
mod relink;
mod socket;
mod state_file;
mod supervise;
#[cfg(test)]
mod test_support;
mod watch;

use std::{
    cell::OnceCell,
    collections::{HashMap, HashSet},
    num::NonZeroUsize,
    process::ExitCode,
    time::{Duration, Instant},
};

use clap::Parser;
use relay_core::{
    AddressBook, AgentName, Claim, MachineName, Now, PaneId, PlaceRefresh, Popper, Relay, SessionId,
};

use crate::{
    deliver::Deliverer,
    episodes::Episodes,
    herdr::{Herdr, RealHerdr},
    liveness::{HeartbeatConfig, Liveness},
    state_file::StateFile,
    supervise::{Bus, Supervisor, SystemClock, Wake},
    watch::{Inbox, Outcome, PopOutcome, PoppedBatch, try_deliver},
};

/// Most messages popped from an inbox in one poll; a batch is also held to
/// [`MAX_PROMPT_BYTES`] by the pop itself, see [`RelayInbox::pop`].
const BATCH: NonZeroUsize = NonZeroUsize::new(50).expect("50 is non-zero");

/// How large one `herdr agent prompt` payload is allowed to get: the byte
/// budget of [`Relay::pop_inbox_budget`], which counts each message as it
/// renders (signature with the longest sender label, indented continuation
/// lines, the newline joining messages). Messages beyond it stay queued for the
/// next poll rather than being typed in as one giant paste (AGE-2013's PR
/// review, finding 1). Only the first message of a batch can push it past this,
/// since it is always taken; one message is at most `MAX_MESSAGE_BYTES` of text.
const MAX_PROMPT_BYTES: usize = 16 * 1024;

/// Connect/read/write timeout for each Redis call this process makes.
const REDIS_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Parser)]
#[command(
    name = "relay-watcher",
    version,
    about = "Wakes idle team leads on this machine by typing ix-relay messages in through herdr."
)]
struct Cli {
    /// Redis URL of the bus. Defaults come from `~/.ix/relay/config.toml`,
    /// then `IX_RELAY_URL`, then `redis://127.0.0.1:6379`.
    #[arg(long)]
    url: Option<String>,
    /// Redis key prefix, matching `agent-msg`'s. Defaults come from
    /// `~/.ix/relay/config.toml`, then `IX_RELAY_PREFIX`, then
    /// [`relay_core::DEFAULT_PREFIX`].
    #[arg(long)]
    prefix: Option<String>,
    /// How soon to look again at a lead that has messages queued but was left
    /// for its own hook (mid-turn, blocked). New messages wake the watcher at
    /// once whatever this is. Defaults come from `~/.ix/relay/config.toml`'s
    /// `[watcher] interval_secs`, then `IX_RELAY_WATCHER_INTERVAL_SECS`, then 2.
    #[arg(long)]
    interval_secs: Option<u64>,
    /// How long after everything was last looked at the backup sweep looks
    /// again, in seconds. Sends wake the watcher at once, so this only bounds
    /// the cost of a lost wake and the age of the state file.
    #[arg(long, default_value_t = DEFAULT_SWEEP_SECS)]
    sweep_secs: u64,
    /// Run a single sweep and exit, instead of looping forever. For manual
    /// checks and tests; the systemd/launchd units never pass this. Exits
    /// nonzero if that one sweep couldn't even reach Redis.
    #[arg(long)]
    once: bool,
}

/// The decodable registrations of `list`, logging each undecodable record once
/// (tracked in `reported`, by hash field) rather than on every read. A record
/// that decodes again is forgotten, so breaking again logs again.
fn decodable(
    list: relay_core::AgentList,
    reported: &mut HashSet<String>,
) -> (Vec<relay_core::Registration>, Vec<relay_core::Salvage>) {
    reported.retain(|field| list.skipped.iter().any(|s| s.field == *field));
    // What the prune rule needs from the records this binary refuses (AGE-2027).
    let salvaged = list
        .skipped
        .iter()
        .filter_map(relay_core::SkippedRegistration::salvage)
        .collect();
    let regs = list.into_registrations_reporting(|skipped| {
        if reported.insert(skipped.field.clone()) {
            eprintln!("relay-watcher: {skipped}");
        }
    });
    (regs, salvaged)
}

/// Handles a failure while serving one lead. A connection-level error is
/// returned so the tick ends and the connection is dropped (its state is
/// unknown: after a timeout the server's late reply is still on the wire). Any
/// other error is that lead's alone: logged with `context`, and the tick goes
/// on with the next lead.
fn per_lead(err: relay_core::Error, context: impl std::fmt::Display) -> relay_core::Result<()> {
    if err.is_connection() {
        return Err(err);
    }
    eprintln!("relay-watcher: {context}: {err}");
    Ok(())
}

/// State carried across ticks, so "once per episode" (blocked notifications,
/// stale-registration, no-agent and herdr-error log lines) means what it says
/// instead of firing every tick. Nothing in it outlives the registry: each tick
/// keeps only leads it saw ([`Episodes::retain_present`]) and only records
/// still reported.
struct WatcherState {
    /// Ticks so far, for the fair rotation of the order leads are looked at in.
    rotation: usize,
    /// The condition each local lead is currently in, if any.
    episodes: Episodes,
    /// Registry records already reported as undecodable (by hash field), so a
    /// record a newer binary wrote is logged once, not on every tick.
    skipped_reported: HashSet<String>,
    /// Whether the herdr entries this binary could not decode are already
    /// reported for the current episode.
    dropped_reported: bool,
    /// Whether the failure to read herdr's layout is already reported for the
    /// current episode: one line when it starts, none per refresh, one when it ends.
    layout_failed_reported: bool,
    /// Leads whose relink is refused for a lasting reason, reported once (see
    /// [`relink::Reported`]).
    relink_reported: relink::Reported,
    /// When each lead's last rename event was recorded, so a flapping label is one
    /// event per [`RENAME_EVENT_GAP`], not one per refresh.
    last_rename_event: HashMap<AgentName, Instant>,
    /// Leads whose renames inside that gap were left out of the stream, reported
    /// once while it lasts.
    rename_gap_reported: HashSet<AgentName>,
    /// When [`refresh_places`] last read herdr's layout; `None` before the
    /// first tick, so it runs immediately.
    last_place_refresh: Option<Instant>,
    /// Why the running tick runs: what it looks at (see [`Wake`]).
    wake: Wake,
    /// When each lead that had mail queued is next looked at again. A lead with
    /// an empty inbox has no entry: only a send can give it work, and a send
    /// wakes the watcher.
    retry_at: HashMap<AgentName, Instant>,
    /// How long after looking at a lead with mail to look again.
    retry_every: Duration,
    /// When the backup sweep is next due.
    next_sweep: Instant,
    /// The newest message-log entry known to be looked at, where the next wait
    /// listens from. `None` when the log cannot be followed (a key of the wrong
    /// type, an ACL refusing it): the watcher then runs on its timers alone.
    log_id: Option<String>,
    /// Whether the "cannot follow the log" line was printed for this condition.
    degraded_logged: bool,
    /// How long after everything was last looked at the backup sweep looks again.
    sweep_every: Duration,
    /// The registry as last read and when, reused by wakes and retries within
    /// [`REGISTRY_TTL`] so a busy bus does not cost a whole-registry read per send.
    registry: Option<(Instant, Vec<relay_core::Registration>)>,
    /// What can be read of the records the last full read refused, so they are
    /// pruned like any other stale registration (see [`prune_refused`]).
    refused: Vec<relay_core::Salvage>,
    /// When a registry served from the cache is looked at again in full, so a
    /// lead registered meanwhile is not missed until the sweep.
    registry_recheck_at: Option<Instant>,
    /// Each local lead's inbox length as of the last tick that saw it: a wake
    /// looks at a lead only when it has grown (or has no timer).
    last_len: HashMap<AgentName, usize>,
    /// When each lead was last looked at with herdr, capping a wake's spawns.
    last_examined: HashMap<AgentName, Instant>,
    /// When each old registration was last checked for pruning, so a lead idle
    /// for a day costs a herdr call every [`PRUNE_RECHECK_EVERY`], not every tick.
    prune_checked: HashMap<AgentName, Instant>,
    /// Leads whose in-flight reap is failing or stuck, so a persistent trouble is
    /// logged once when it starts and once when it ends, not on every sweep.
    reap_trouble: HashSet<AgentName>,
    /// The heartbeat, the herdr check and the registration refresh (AGE-2033).
    live: Liveness,
    /// The OTLP metrics push (FR-045); disabled when no endpoint is configured.
    otlp: otlp::Publisher,
}

impl WatcherState {
    /// A fresh state that retries a lead with mail every `retry_every` and
    /// sweeps every `sweep_every`.
    fn new(retry_every: Duration, sweep_every: Duration) -> Self {
        Self {
            episodes: Episodes::default(),
            skipped_reported: HashSet::new(),
            dropped_reported: false,
            layout_failed_reported: false,
            relink_reported: relink::Reported::default(),
            last_rename_event: HashMap::new(),
            rename_gap_reported: HashSet::new(),
            last_place_refresh: None,
            wake: Wake::Sweep,
            retry_at: HashMap::new(),
            retry_every,
            next_sweep: Instant::now(),
            log_id: None,
            degraded_logged: false,
            sweep_every,
            registry: None,
            refused: Vec::new(),
            registry_recheck_at: None,
            last_len: HashMap::new(),
            last_examined: HashMap::new(),
            prune_checked: HashMap::new(),
            rotation: 0,
            reap_trouble: HashSet::new(),
            live: Liveness::default(),
            otlp: otlp::Publisher::disabled(),
        }
    }

    /// The wake owed at `now` without waiting for a message: the sweep when it
    /// is due, else a retry when a lead's timer is.
    fn due(&self, now: Instant) -> Option<Wake> {
        if now >= self.next_sweep || self.registry_recheck_at.is_some_and(|at| now >= at) {
            Some(Wake::Sweep)
        } else if self.retry_at.values().any(|at| *at <= now) {
            Some(Wake::Retry)
        } else {
            None
        }
    }

    /// How long until [`WatcherState::due`] is `Some`: zero when it already is.
    fn until_due(&self, now: Instant) -> Duration {
        self.retry_at
            .values()
            .copied()
            .chain([self.next_sweep])
            .chain(self.registry_recheck_at)
            .min()
            .map_or(Duration::ZERO, |at| at.saturating_duration_since(now))
    }

    /// Whether this tick looks at `name`, a lead with `len` messages queued.
    ///
    /// A sweep looks at every lead with mail. A retry looks only at leads whose
    /// timer is due. A message wake (which any send anywhere on the bus causes)
    /// looks only at a lead whose inbox grew or that has no timer yet, and at most
    /// once per [`MIN_EXAMINE_GAP`]: a lead left for its hook does not cost a
    /// herdr spawn per send of other machines' traffic. A lead held back by the
    /// gap gets a timer for when it ends.
    fn examines(&mut self, name: &AgentName, len: usize, now: Instant) -> bool {
        let grew = len > self.last_len.get(name).copied().unwrap_or(0);
        self.last_len.insert(name.clone(), len);
        match self.wake {
            Wake::Sweep => true,
            Wake::Retry => self.retry_at.get(name).is_none_or(|at| *at <= now),
            Wake::Message => {
                if !(grew || !self.retry_at.contains_key(name)) {
                    return false;
                }
                match self.last_examined.get(name) {
                    Some(at) if now.saturating_duration_since(*at) < MIN_EXAMINE_GAP => {
                        self.retry_at
                            .entry(name.clone())
                            .or_insert(*at + MIN_EXAMINE_GAP);
                        false
                    }
                    _ => true,
                }
            }
        }
    }

    /// Whether `name`, an old registration, is due for a pruning check.
    fn prune_due(&self, name: &AgentName, now: Instant) -> bool {
        self.prune_checked
            .get(name)
            .is_none_or(|at| now.saturating_duration_since(*at) >= PRUNE_RECHECK_EVERY)
    }
}

impl Default for WatcherState {
    fn default() -> Self {
        Self::new(
            Duration::from_secs(relay_core::config::DEFAULT_WATCHER_INTERVAL_SECS),
            Duration::from_secs(DEFAULT_SWEEP_SECS),
        )
    }
}

/// The default `--sweep-secs`: how long after everything was last looked at the
/// backup sweep looks again. A send wakes the watcher at once; this only catches
/// a wake that was lost (a dropped connection, a restarted Redis), so it can be
/// slow. It is also the longest the state file goes without a refresh while
/// nothing happens.
const DEFAULT_SWEEP_SECS: u64 = 15;

/// How long the registry read by a tick is reused by the wakes and retries after
/// it. Bounds the whole-registry reads a busy bus causes to one per this.
const REGISTRY_TTL: Duration = Duration::from_secs(2);

/// The fewest time between two herdr looks at one lead that message wakes cause.
const MIN_EXAMINE_GAP: Duration = Duration::from_secs(1);

/// The longest one `XREAD BLOCK` waits, and so the longest a tunnel that died
/// mid-wait goes unnoticed (the block plus the socket's one second of slack).
const WAIT_BLOCK_MAX: Duration = Duration::from_secs(5);

/// How often an old registration is re-checked with herdr for pruning. A lead
/// whose session is still live stays registered, so without this every tick
/// would spawn `herdr agent get` for it forever.
const PRUNE_RECHECK_EVERY: Duration = Duration::from_secs(10 * 60);

/// How often [`refresh_places`] re-reads herdr's layout (three herdr calls).
/// A tab closing or a rename only needs to reach the registry within a poll or
/// two of a person noticing, not on every tick.
const PLACE_REFRESH_EVERY: Duration = Duration::from_secs(30);

/// The least time between two rename events of one lead. A label that flaps is
/// followed (the address stays true) but recorded once per gap, so one workspace
/// cannot fill the shared, capped moves stream.
const RENAME_EVENT_GAP: Duration = Duration::from_secs(5 * 60);

/// Keeps each local lead's registered `server:workspace[:tab[:pane]]` place true to
/// herdr's live layout. Hooks record a place at `SessionStart` and cannot
/// refresh it (they make no herdr calls mid-session); without this a closed
/// tab, which renumbers herdr's unlabelled tabs, or a renamed workspace leaves
/// an address resolving to whoever used to sit there. Also gives a lead that
/// registered before addresses existed its first one. Failures are logged and
/// skipped: address freshness never blocks delivery. Only a connection-level
/// Redis error is returned (see [`per_lead`]).
fn refresh_places(
    relay: &mut Relay,
    herdr: &impl Herdr,
    regs: &mut [relay_core::Registration],
    machine: &MachineName,
    state: &mut WatcherState,
    log: &mut impl FnMut(String),
) -> relay_core::Result<HashSet<AgentName>> {
    if state
        .last_place_refresh
        .is_some_and(|at| at.elapsed() < PLACE_REFRESH_EVERY)
    {
        return Ok(HashSet::new());
    }
    state.last_place_refresh = Some(Instant::now());
    let layout = match herdr.layout(machine) {
        Ok(layout) => {
            if std::mem::take(&mut state.layout_failed_reported) {
                log("herdr's layout is readable again".to_owned());
            }
            layout
        }
        Err(e) => {
            // Nothing is relinked or renamed on an answer that did not come: one
            // line per episode, the addresses stay as registered.
            if !std::mem::replace(&mut state.layout_failed_reported, true) {
                log(format!(
                    "could not read herdr's layout to refresh addresses and follow moved \
                     panes: {e}"
                ));
            }
            return Ok(HashSet::new());
        }
    };
    // Entries of herdr's listings this binary could not decode: reported once per
    // episode (until a listing decodes whole again), not on every refresh.
    let dropped = herdr.dropped_entries();
    if dropped == 0 {
        state.dropped_reported = false;
    } else if !state.dropped_reported {
        state.dropped_reported = true;
        log(format!(
            "{dropped} herdr layout entr{} could not be read and left out: the addresses \
             they would refresh stay as registered",
            if dropped == 1 { "y" } else { "ies" }
        ));
    }
    // A session that moved to another pane is followed first, and the relinked
    // leads are updated in `regs` (which the tick goes on with and caches).
    let relinked = relink::relink_moved(
        relay,
        &layout,
        regs,
        machine,
        now_ms(),
        &mut state.relink_reported,
        log,
    )?;
    // The same herdr answer, joined with the registry as just updated: the pane
    // metrics cost no herdr call of their own (FR-045).
    if let Some(view) = &layout.view {
        state.otlp.set_panes(Instant::now(), view, regs);
    }
    for reg in regs.iter_mut() {
        if reg.machine != *machine {
            continue;
        }
        if !relink::pane_has_current_location(reg, &layout) {
            if reg.place.is_some() {
                match relay.invalidate_place(reg) {
                    Ok(true) => reg.place = None,
                    Ok(false) => {}
                    Err(e) => per_lead(
                        e,
                        format_args!("{}: could not invalidate its address", reg.name),
                    )?,
                }
            }
            continue;
        }
        let Some(place) = reg
            .herdr_pane
            .as_ref()
            .and_then(|pane| layout.places.get(pane))
        else {
            continue;
        };
        if reg.place.as_ref() == Some(place) {
            continue;
        }
        let name = reg.name.clone();
        let relabelled = reg
            .place
            .as_ref()
            .is_some_and(|old| old.workspace != place.workspace || old.tab != place.tab);
        let gap_over = state
            .last_rename_event
            .get(&name)
            .is_none_or(|at| at.elapsed() >= RENAME_EVENT_GAP);
        match relay.refresh_place(&name, place, now_ms(), gap_over) {
            Ok(PlaceRefresh::Unchanged) => {}
            Ok(PlaceRefresh::Renamed(_)) => {
                state.last_rename_event.insert(name.clone(), Instant::now());
                state.rename_gap_reported.remove(&name);
                eprintln!("relay-watcher: {name}: address is now {place:?}");
                reg.place = Some(place.clone());
            }
            Ok(PlaceRefresh::Refreshed) => {
                if relabelled && !gap_over && state.rename_gap_reported.insert(name.clone()) {
                    log(format!(
                        "{name}: its address changed again within {} minutes; the address is \
                         kept true, the change is not recorded as another event",
                        RENAME_EVENT_GAP.as_secs() / 60
                    ));
                }
                reg.place = Some(place.clone());
            }
            Err(e) => per_lead(e, format_args!("{name}: could not refresh its address"))?,
        }
    }
    state
        .last_rename_event
        .retain(|name, _| regs.iter().any(|r| r.name == *name));
    state
        .rename_gap_reported
        .retain(|name| regs.iter().any(|r| r.name == *name));
    Ok(relinked)
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    // `~/.ix/relay/config.toml` (AGE-2017), resolved flag > env > file >
    // default. Unlike `agent-msg hook`, nothing here needs to survive a
    // malformed file: there's no agent turn to protect, so this exits 1 like
    // any other bad configuration.
    let flags = relay_core::config::Flags {
        core: relay_core::config::CoreFlags {
            url: cli.url.clone(),
            prefix: cli.prefix.clone(),
            machine: None,
            human: None,
        },
        watcher_interval_secs: cli.interval_secs,
    };
    let settings = match relay_core::config::load(flags) {
        Ok(settings) => settings,
        Err(e) => {
            eprintln!("relay-watcher: {e}");
            return ExitCode::FAILURE;
        }
    };
    let machine = settings.core.machine.value;
    let interval_secs = settings.watcher_interval_secs.value;
    let sweep_secs = cli.sweep_secs;
    eprintln!(
        "relay-watcher: watching leads on \"{machine}\" via {}, waking on sends, \
         retrying a lead with mail every {interval_secs}s, sweeping every {}s",
        relay_core::config::redact_url(&settings.core.url.value),
        sweep_secs
    );

    // A URL that can never parse would fail every reconnect forever; that is a
    // configuration error, and one of the two things this process exits for.
    if let Err(e) = Relay::validate_url(&settings.core.url.value) {
        eprintln!("relay-watcher: invalid redis url: {e}");
        return ExitCode::FAILURE;
    }
    // `--once` is a manual check: it must not overwrite the running watcher's
    // state file.
    let state_file = if cli.once {
        None
    } else {
        StateFile::default_location()
    };
    if state_file.is_none() && !cli.once {
        eprintln!("relay-watcher: neither HOME nor IX_RELAY_CONFIG is set; keeping no state file");
    }
    let sweep = Duration::from_secs(sweep_secs.max(1));
    let mut state = WatcherState::new(Duration::from_secs(interval_secs.max(1)), sweep);
    // The OTLP push (FR-045): started, and announced, by the module. `--once` sends
    // nothing.
    state.otlp = otlp::Publisher::from_settings(
        &settings.watcher_otlp,
        &machine,
        env!("CARGO_PKG_VERSION"),
        cli.once,
    );
    // `--once` is a manual check: like the state file, it must not publish a
    // heartbeat that a running watcher's would be overwritten by (or hide).
    if !cli.once {
        state.live = Liveness::publishing(HeartbeatConfig {
            machine: machine.clone(),
            pid: std::process::id(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            started_at_ms: now_ms(),
            sweep,
            ttl: relay_core::health::heartbeat_ttl(sweep),
            otlp: state.otlp.status(),
        });
    }
    let herdr = delivery(settings.delivery);
    let bus = RedisBus {
        url: settings.core.url.value,
        prefix: settings.core.prefix.value,
        machine,
        herdr: herdr.clone(),
        state,
    };
    // SIGTERM (a service manager's stop) and SIGINT are noticed at the next turn of
    // the loop, so the reply socket is removed on the way out. `signal_hook::flag`
    // only ever does one atomic store in the handler, so this needs no `unsafe`.
    let stop = install_signal_handlers();
    let mut supervisor = Supervisor::new(
        bus,
        SystemClock::new(),
        herdr,
        state_file,
        std::process::id(),
    )
    .stop_on(std::sync::Arc::clone(&stop));
    if cli.once {
        return if supervisor.step().ok {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }
    supervisor.run_until(|| !stop.load(std::sync::atomic::Ordering::Relaxed));
    eprintln!("relay-watcher: stopping on a signal");
    // Dropping the supervisor and the bus drops the socket delivery, which removes the
    // reply socket (and the status file, unless it holds a held batch to hand on).
    drop(supervisor);
    ExitCode::SUCCESS
}

/// Registers SIGTERM and SIGINT to set the returned flag. A signal that cannot be
/// registered is reported and leaves the default action (an immediate exit that
/// leaves the reply socket for the next start to remove).
fn install_signal_handlers() -> std::sync::Arc<std::sync::atomic::AtomicBool> {
    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    for signal in [signal_hook::consts::SIGTERM, signal_hook::consts::SIGINT] {
        if let Err(e) = signal_hook::flag::register(signal, std::sync::Arc::clone(&flag)) {
            eprintln!("relay-watcher: cannot handle signal {signal}: {e}");
        }
    }
    flag
}

/// The production delivery for `mode`: herdr alone (the default), or herdr
/// behind Claude Code's inbox socket (FR-044). A socket whose locations cannot be
/// told is reported and leaves herdr alone.
fn delivery(mode: relay_core::config::DeliveryMode) -> socket::Routed<RealHerdr> {
    use relay_core::config::DeliveryMode;
    let herdr = RealHerdr::default();
    if mode == DeliveryMode::Herdr {
        return socket::Routed::herdr_only(herdr);
    }
    match socket::deliverer::SocketConfig::from_env() {
        Ok(config) => {
            let socket = socket::deliverer::SocketDeliverer::start(
                config,
                Box::new(socket::session::ProcFs),
            );
            match socket.reply_path() {
                Some(path) => eprintln!(
                    "relay-watcher: delivery = socket: posting to Claude Code inbox sockets, \
                     replies on {}; herdr is the fallback",
                    path.display()
                ),
                None => eprintln!(
                    "relay-watcher: delivery = socket, but the reply socket could not be bound; \
                     every batch goes to herdr (see agent-msg doctor)"
                ),
            }
            socket::Routed::socket_first(herdr, socket)
        }
        Err(why) => {
            eprintln!("relay-watcher: delivery = socket is unavailable ({why}); using herdr");
            socket::Routed::herdr_only(herdr)
        }
    }
}

/// The production [`Bus`]: a Redis connection per [`Relay`], woken by the
/// message log and polled by [`tick`].
struct RedisBus<H: Herdr + Deliverer> {
    url: String,
    prefix: String,
    machine: MachineName,
    herdr: H,
    state: WatcherState,
}

impl<H: Herdr + Deliverer> Bus for RedisBus<H> {
    type Conn = Relay;

    /// Connects, and reads the log's tail before anything else: the first sweep
    /// runs after this, so a message sent in between is a wake the next
    /// [`Bus::wait`] hears rather than one lost. Timers of an earlier
    /// connection are dropped; the first poll is a sweep.
    fn connect(&mut self) -> relay_core::Result<Relay> {
        let mut relay = Relay::connect(&self.url, REDIS_TIMEOUT)?.with_prefix(self.prefix.clone());
        match relay.log_tail_id() {
            Ok(id) => {
                self.state.log_id = Some(id);
                self.state.degraded_logged = false;
            }
            Err(e) if e.is_connection() => return Err(e),
            Err(e) => {
                // A log this watcher cannot follow costs it the wake-ups, not its
                // deliveries: it falls back to its timers and says so once.
                self.state.log_id = None;
                if !self.state.degraded_logged {
                    self.state.degraded_logged = true;
                    eprintln!(
                        "relay-watcher: cannot follow the message log ({e}); \
                         running on the retry timers and the sweep only"
                    );
                }
            }
        }
        self.state.retry_at.clear();
        self.state.registry = None;
        self.state.last_len.clear();
        self.state.next_sweep = Instant::now();
        Ok(relay)
    }

    fn poll(&mut self, conn: &mut Relay, wake: Wake) -> relay_core::Result<()> {
        self.state.wake = wake;
        self.state.live.check_herdr(&self.herdr);
        let ticked = tick(
            conn,
            &self.herdr,
            &self.machine,
            &mut self.state,
            &mut |line| {
                eprintln!("relay-watcher: {line}");
            },
        );
        self.state.live.finish_tick(conn, wake, ticked)
    }

    /// Blocks on the message log until a send, a due retry or the sweep, in
    /// blocks of at most [`WAIT_BLOCK_MAX`] so a dead tunnel is noticed promptly.
    /// Without a followable log it sleeps to the next timer instead.
    fn wait(&mut self, conn: &mut Relay) -> relay_core::Result<Wake> {
        loop {
            let now = Instant::now();
            if let Some(wake) = self.state.due(now) {
                return Ok(wake);
            }
            let block = self.state.until_due(now).min(WAIT_BLOCK_MAX);
            let Some(after) = self.state.log_id.clone() else {
                std::thread::sleep(block);
                continue;
            };
            if conn.wait_for_log(&after, block)? {
                // Jump to the tail so the tick that follows covers everything
                // sent so far and only a later send wakes the next wait.
                self.state.log_id = Some(conn.log_tail_id()?);
                return Ok(Wake::Message);
            }
        }
    }
}

/// One tick: for every lead registered on `machine` with a non-empty inbox,
/// decide whether to deliver via [`try_deliver`] and log the outcome. What it
/// looks at depends on why it runs ([`WatcherState::wake`]): a message or the
/// sweep looks at every lead with mail; a retry only at the leads whose timer
/// is due.
///
/// One request reads the registry and one pipelined request every local
/// inbox's length; the registry read is reused for sender addresses when
/// something is delivered.
///
/// A failure to list registrations at all aborts the tick, and so does any
/// connection-level error ([`per_lead`]); the caller drops the connection and
/// reconnects. A data error reading or popping one lead's inbox is logged and
/// that lead is skipped, so one bad lead never blocks the rest (AGE-2013's PR
/// review, finding 8).
fn tick<H: Herdr + Deliverer>(
    relay: &mut Relay,
    herdr: &H,
    machine: &MachineName,
    state: &mut WatcherState,
    log: &mut impl FnMut(String),
) -> relay_core::Result<()> {
    let started = Instant::now();
    // The registry is read by a sweep, and by a wake or retry once the copy is
    // older than REGISTRY_TTL; within it the copy is reused and a full look is
    // scheduled for when it expires, so a lead registered meanwhile is not lost.
    let cached = state.registry.take().filter(|(at, _)| {
        state.wake != Wake::Sweep && started.saturating_duration_since(*at) < REGISTRY_TTL
    });
    let (read_at, mut regs) = match cached {
        Some((at, regs)) => {
            state.registry_recheck_at.get_or_insert(at + REGISTRY_TTL);
            (at, regs)
        }
        None => {
            state.registry_recheck_at = None;
            let (regs, refused) = decodable(relay.agents()?, &mut state.skipped_reported);
            state.refused = refused;
            (started, regs)
        }
    };
    apply_settlements(relay, herdr, log)?;
    // A lead relinked by this refresh is left out of the rest of the tick (delivery,
    // refresh, prune) and looked at from the next one, from `regs` as updated: the
    // tick never works from the pane the lead just left.
    let relinked = refresh_places(relay, herdr, &mut regs, machine, state, log)?;
    // A pane is only meaningful with its machine (FR-009): a registration is this
    // watcher's to act on when its `PaneRef` is on this machine.
    let local: Vec<(&relay_core::Registration, &PaneId)> = regs
        .iter()
        .filter(|r| !relinked.contains(&r.name))
        .filter_map(|r| Some((r, r.pane_on(machine)?)))
        .collect();
    state.live.leads_watched = u32::try_from(local.len()).unwrap_or(u32::MAX);
    let names: Vec<&AgentName> = local.iter().map(|(reg, _)| &reg.name).collect();
    let lens = if state.wake == Wake::Sweep {
        // A sweep, and so the first poll after a (re)connect, first returns every
        // lead's expired in-flight batches to its inbox: a batch popped by a
        // consumer that died before it acknowledged (FR-020). Same request as
        // the lengths, so the lengths include what came back.
        let everyone: Vec<&AgentName> = regs.iter().map(|r| &r.name).collect();
        let sweep = relay.reap_and_inbox_lens(&everyone, &names, Now::Server)?;
        for (name, reaped) in everyone.iter().copied().zip(sweep.reaped) {
            match reaped {
                Ok(r) => {
                    state.otlp.reaped(r.requeued, r.dead);
                    if r.requeued + r.dead + r.evicted > 0 {
                        log(format!(
                            "{name}: returned {} expired in-flight message(s) to the inbox, \
                             dead-lettered {}, evicted {} old dead letter(s)",
                            r.requeued, r.dead, r.evicted
                        ));
                    }
                    if r.stuck > 0 {
                        if state.reap_trouble.insert(name.clone()) {
                            log(format!(
                                "{name}: {} in-flight message(s) have spent their redeliveries \
                                 but cannot be dead-lettered (its deadletter key holds the wrong \
                                 type); left in flight",
                                r.stuck
                            ));
                        }
                    } else if state.reap_trouble.remove(name) {
                        log(format!("{name}: in-flight reap is healthy again"));
                    }
                }
                Err(e) if e.is_connection() => return Err(e),
                // Once per episode: the same lead fails the same way every sweep.
                Err(e) => {
                    if state.reap_trouble.insert(name.clone()) {
                        log(format!("{name}: reaping in-flight failed: {e}"));
                    }
                }
            }
        }
        state.reap_trouble.retain(|name| everyone.contains(&name));
        sweep.lens
    } else {
        relay.inbox_lens(&names)?
    };
    let now = Instant::now();
    if state.wake != Wake::Retry {
        state.next_sweep = now + state.sweep_every;
    }
    // Built at most once, when something is delivered.
    let book: OnceCell<AddressBook> = OnceCell::new();
    let mut seen = HashSet::new();
    // A transport whose time per tick is limited looks at the leads in a rotating order,
    // so a lead behind slow ones is reached within as many ticks as there are leads.
    let (mut local, mut lens) = (local, lens);
    if herdr.rotates() && !local.is_empty() {
        let by = state.rotation % local.len();
        local.rotate_left(by);
        lens.rotate_left(by);
    }
    state.rotation = state.rotation.wrapping_add(1);
    for ((reg, pane), len) in local.into_iter().zip(lens) {
        let name = &reg.name;
        seen.insert(name.clone());
        let len = match len {
            Ok(len) => len,
            Err(e) => {
                per_lead(e, format_args!("{name}: inbox_len failed"))?;
                // Unreadable is not empty: try it again on its timer, and never
                // leave a due timer behind for the wait to spin on.
                state.retry_at.insert(name.clone(), now + state.retry_every);
                continue;
            }
        };
        if len == 0 {
            state.retry_at.remove(name);
            state.last_len.remove(name);
            let ended = state.episodes.observe(name, None);
            if let Some(kind) = ended.ended {
                log(format!("{name}: {} ended", kind.describe()));
            }
            // A pane herdr still shows with the registered session is alive: its
            // stamp moves forward instead of aging toward the prune rule.
            let refreshed = !state.live.delivery_blocked()
                && state.live.touch_due(reg, now_ms(), now)
                && last_seen::refresh_if_live(
                    relay,
                    herdr,
                    &mut state.live,
                    reg,
                    pane,
                    now_ms(),
                    now,
                )?;
            if !refreshed && old_enough_to_prune(reg.last_seen_ms) && state.prune_due(name, now) {
                state.prune_checked.insert(name.clone(), now);
                maybe_prune(
                    relay,
                    herdr,
                    name,
                    pane,
                    reg.session_id.as_ref(),
                    reg.last_seen_ms,
                )?;
            }
            continue;
        }
        if !state.examines(name, len, now) {
            continue;
        }
        state.last_examined.insert(name.clone(), now);
        // Mail is queued and nothing new will announce it: come back.
        state.retry_at.insert(name.clone(), now + state.retry_every);
        // herdr's output is in a shape ix-relay cannot read (FR-036): type into
        // no pane. The mail stays queued for the lead's own hook, and the check
        // that blocks this is re-run until it passes.
        if state.live.delivery_blocked() {
            continue;
        }
        let notice = state.episodes.notice(name, now);
        let mut inbox = RelayInbox {
            relay,
            lead: name,
            registry: &regs,
            book: &book,
            lost: None,
        };
        let outcome = try_deliver(
            herdr,
            name.as_str(),
            pane,
            reg.session_id.as_ref(),
            notice,
            &mut inbox,
        );
        let lost = inbox.lost.take();
        match &outcome {
            Outcome::Delivered { messages } => state.otlp.delivered(*messages),
            Outcome::PromptRejected { requeued, .. } => state.otlp.requeued(*requeued),
            _ => {}
        }
        if matches!(outcome, Outcome::Delivered { .. }) {
            // What is left is unknown: any later mail counts as growth.
            state.last_len.remove(name);
        }
        // Record what happened before anything can end the tick: a delivery
        // that typed messages into a pane must always leave its journal line.
        if matches!(outcome, Outcome::NotifyFailed(_)) {
            state.episodes.notify_failed(name, Instant::now());
        }
        let transition = observe_outcome(&mut state.episodes, name, &outcome);
        for line in episode_lines(name, pane, &outcome, transition) {
            log(line);
        }
        if let Some(lost) = lost {
            return Err(lost);
        }
    }
    state.retry_at.retain(|name, _| seen.contains(name));
    state.last_len.retain(|name, _| seen.contains(name));
    state.last_examined.retain(|name, _| seen.contains(name));
    state.otlp.tick_took(started.elapsed());
    state.registry = Some((read_at, regs));
    prune_refused(relay, herdr, machine, state)?;
    state
        .prune_checked
        .retain(|name, _| seen.contains(name) || state.refused.iter().any(|r| r.name == *name));
    state.live.retain_touched(&seen);
    for (name, kind) in state.episodes.retain_present(&seen) {
        log(format!(
            "{name}: no longer watched here; {} ended",
            kind.describe()
        ));
    }
    // Last, after the deliveries, the registry cache and the prune, and answering to
    // nobody: its store reads can neither delay nor skip any of them (FR-045).
    if let Some((_, regs)) = &state.registry {
        state
            .otlp
            .publish_if_due(relay, regs, Instant::now(), now_ms(), log);
    }
    Ok(())
}

/// Shows the notifications the transport asks for (a decision it cannot write).
fn show_alerts<H: Herdr + Deliverer>(herdr: &H, log: &mut impl FnMut(String)) {
    for (title, body) in herdr.alerts() {
        if let Err(e) = herdr.notify(&title, &body) {
            log(format!("the notification failed: {e}"));
        }
    }
}

/// Applies what the transport settled since the last tick (a receipt that came
/// late): acknowledges, puts back or dead-letters the batch it is about. A batch
/// that has meanwhile been reaped or acknowledged is simply not found.
fn apply_settlements<H: Herdr + Deliverer>(
    relay: &mut Relay,
    herdr: &H,
    log: &mut impl FnMut(String),
) -> relay_core::Result<()> {
    use deliver::SettleAction;
    use relay_core::Popper;
    for s in herdr.settle() {
        let Ok(lead) = s.lead.parse::<AgentName>() else {
            continue;
        };
        let result = match s.action {
            SettleAction::Ack => relay.ack(&lead, s.batch, Popper::Watcher).map(usize::from),
            SettleAction::Requeue => relay.requeue_batch(&lead, s.batch, Popper::Watcher),
            SettleAction::DeadLetter => relay.dead_letter_batch(&lead, s.batch, Popper::Watcher),
            SettleAction::Renew(lease) => relay.renew_batch(
                &lead,
                s.batch,
                lease,
                relay_core::Now::Server,
                Popper::Watcher,
            ),
        };
        // The transport forgets a decision only on a confirmed write, and returns it again
        // on the next tick otherwise; so a failure here loses nothing, and a connection
        // error can end the tick with the rest still owed.
        herdr.applied(
            &s,
            &result.as_ref().map(|n| *n).map_err(ToString::to_string),
        );
        match result {
            Ok(moved) => {
                if moved > 0
                    && let Some((title, body)) = &s.notice
                    && let Err(e) = herdr.notify(title, body)
                {
                    log(format!("{lead}: the notification failed: {e}"));
                }
                if !matches!(s.action, SettleAction::Renew(_)) {
                    log(format!(
                        "{lead}: batch {} {:?} ({}); {}",
                        s.batch.get(),
                        s.action,
                        s.why,
                        if moved == 0 {
                            "it was no longer in flight"
                        } else {
                            "done"
                        }
                    ));
                } else if moved == 0 {
                    log(format!(
                        "{lead}: held batch {} is no longer in flight; no longer renewed",
                        s.batch.get()
                    ));
                }
            }
            Err(e) if e.is_connection() => {
                show_alerts(herdr, log);
                return Err(e);
            }
            // Logged once per episode by the transport, which retries.
            Err(_) => {}
        }
    }
    show_alerts(herdr, log);
    Ok(())
}

/// The log lines for one lead's tick: the end of the episode `transition`
/// reports, if any, then the `outcome` itself. An event outcome (a delivery, a
/// lead mid-turn) is logged every time; a condition ([`Outcome::episode`]) only
/// when it begins.
/// Records what one lead's tick found in its episodes. A postponement (the transport's time
/// this tick was spent) says nothing about the lead, so it leaves the episode state
/// completely untouched: it neither ends an episode nor clears who was notified.
fn observe_outcome(
    episodes: &mut episodes::Episodes,
    name: &AgentName,
    outcome: &Outcome,
) -> episodes::Transition {
    if matches!(outcome, Outcome::SocketPostponed { .. }) {
        return episodes::Transition::default();
    }
    episodes.observe(name, outcome.episode())
}

fn episode_lines(
    name: &AgentName,
    pane: &PaneId,
    outcome: &Outcome,
    transition: episodes::Transition,
) -> Vec<String> {
    let mut lines = Vec::new();
    if matches!(outcome, Outcome::SocketPostponed { .. }) {
        return lines;
    }
    if let Some(kind) = transition.ended {
        lines.push(format!("{name}: {} ended", kind.describe()));
    }
    // A failed notification attempt is logged every time it is made (retries
    // are rate-limited, see `NOTIFY_RETRY_EVERY`), so a broken herdr stays visible.
    if outcome.episode().is_none()
        || transition.started.is_some()
        || matches!(outcome, Outcome::NotifyFailed(_))
    {
        lines.push(outcome.log_line(name.as_str(), pane));
    }
    lines
}

/// A registration is eligible for pruning once it's been this long since it
/// last registered (AGE-2018 PR review, finding 7). Deliberately long: a
/// lead sitting quietly idle between turns keeps `last_seen_ms` fresh, at every
/// `SessionStart`/`UserPromptSubmit` and, while its pane is alive, through
/// [`last_seen::refresh_if_live`] (FR-042), so only a genuinely abandoned
/// registration — one whose session hasn't touched the bus in a full day and
/// whose pane herdr no longer vouches for — is ever a candidate.
const PRUNE_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// How long a record whose pane id this binary refuses is kept when herdr cannot
/// confirm it: a week (see [`prune_refused`]).
const REFUSED_PANE_PRUNE_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Whether a registration last seen at `last_seen_ms` has reached [`PRUNE_AGE`].
fn old_enough_to_prune(last_seen_ms: i64) -> bool {
    let age_ms = now_ms().saturating_sub(last_seen_ms);
    age_ms >= i64::try_from(PRUNE_AGE.as_millis()).unwrap_or(i64::MAX)
}

/// Whether the session in a pane is provably not the registered one: both
/// ids known and different.
fn positively_replaced(registered: Option<&SessionId>, live: Option<&SessionId>) -> bool {
    matches!((registered, live), (Some(r), Some(l)) if r != l)
}

/// Removes `name`'s registration once it's old enough (see [`PRUNE_AGE`])
/// and its herdr pane no longer hosts the session it registered under
/// (AGE-2018 PR review, finding 7) — checked live via `herdr`, the same way
/// [`try_deliver`] already tells a genuinely stale registration apart from a
/// live one. Only ever called with an empty inbox (`tick`'s own caller
/// already checked): a registration whose inbox still has something queued
/// is kept regardless of age, so a message queued for a lead that's been
/// away for a while still has somewhere to land when it comes back.
///
/// A herdr failure here is logged and treated as "not stale" — the same
/// fail-safe default every other herdr call in this process uses: an
/// uncertain answer never deletes something that might still be live.
fn maybe_prune(
    relay: &mut Relay,
    herdr: &impl Herdr,
    name: &AgentName,
    pane: &PaneId,
    session_id: Option<&SessionId>,
    last_seen_ms: i64,
) -> relay_core::Result<()> {
    if !old_enough_to_prune(last_seen_ms) {
        return Ok(());
    }
    let stale = match herdr.get(pane) {
        // herdr reports nobody in the pane at all: positive evidence it is gone.
        Ok(None) => true,
        // Someone is there: only a *different* session id proves the
        // registered one left. An id missing on either side proves nothing
        // (a lead registered without one is still live), and unlike
        // `session_matches` — where "unknown" means "do not type into it" —
        // here "unknown" must mean "do not delete it".
        Ok(Some(info)) => positively_replaced(session_id, info.session_id.as_ref()),
        Err(e) => {
            eprintln!("relay-watcher: {name}: prune liveness check failed: {e}");
            false
        }
    };
    if !stale {
        return Ok(());
    }
    prune_stale(relay, name, pane, last_seen_ms)
}

/// Removes the registration the caller has judged stale, and says so.
fn prune_stale(
    relay: &mut Relay,
    name: &AgentName,
    pane: &dyn std::fmt::Display,
    last_seen_ms: i64,
) -> relay_core::Result<()> {
    let age_ms = now_ms().saturating_sub(last_seen_ms);
    match relay.prune(name, last_seen_ms) {
        Ok(None) => {
            eprintln!("relay-watcher: {name}: registration changed since the snapshot; not pruned")
        }
        Ok(Some(aliases)) => {
            let aka = if aliases.is_empty() {
                "none".to_owned()
            } else {
                aliases
                    .iter()
                    .map(AgentName::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            eprintln!(
                "relay-watcher: pruned stale registration for {name} (pane {pane}, empty inbox, \
                 last seen {}h ago); removed alias(es): {aka}",
                age_ms / (60 * 60 * 1000)
            );
        }
        Err(e) => per_lead(e, format_args!("{name}: prune failed"))?,
    }
    Ok(())
}

/// [`maybe_prune`] for the records this binary refuses to decode (a session,
/// pane or machine an older binary wrote outside its type's rule): read tolerantly
/// from the raw JSON, so a stale one is pruned as a new watcher would have pruned
/// it before those ids were typed, not left to be reported on every read forever.
///
/// A pane that is not a [`PaneId`] cannot be asked of herdr: it is pruned on age
/// alone only after [`REFUSED_PANE_PRUNE_AGE`]; a session that is not a [`SessionId`] cannot be the one herdr
/// reports, so any live session there replaces it. A record with no pane was
/// never this watcher's, as before.
fn prune_refused(
    relay: &mut Relay,
    herdr: &impl Herdr,
    machine: &MachineName,
    state: &mut WatcherState,
) -> relay_core::Result<()> {
    let now = Instant::now();
    for salvage in state.refused.clone() {
        let name = &salvage.name;
        if salvage.machine != *machine
            || !old_enough_to_prune(salvage.last_seen_ms)
            || !state.prune_due(name, now)
        {
            continue;
        }
        match relay.inbox_len(name) {
            Ok(0) => {}
            Ok(_) => continue,
            Err(e) => {
                per_lead(e, format_args!("{name}: inbox_len failed"))?;
                continue;
            }
        }
        state.prune_checked.insert(name.clone(), now);
        let stale = match &salvage.pane {
            relay_core::Salvaged::Absent => false,
            // herdr cannot be asked about a pane id this binary refuses, and a newer
            // binary with a wider rule may have written one for a live lead that
            // no watcher refreshes (FR-042 serves decodable records only): age
            // alone prunes it only after REFUSED_PANE_PRUNE_AGE.
            relay_core::Salvaged::Refused => {
                now_ms().saturating_sub(salvage.last_seen_ms)
                    >= i64::try_from(REFUSED_PANE_PRUNE_AGE.as_millis()).unwrap_or(i64::MAX)
            }
            relay_core::Salvaged::Valid(pane) => match herdr.get(pane) {
                Ok(None) => true,
                Ok(Some(info)) => match &salvage.session {
                    relay_core::Salvaged::Refused => info.session_id.is_some(),
                    relay_core::Salvaged::Valid(registered) => {
                        positively_replaced(Some(registered), info.session_id.as_ref())
                    }
                    relay_core::Salvaged::Absent => false,
                },
                Err(e) => {
                    eprintln!("relay-watcher: {name}: prune liveness check failed: {e}");
                    false
                }
            },
        };
        if stale {
            prune_stale(relay, name, &"unreadable", salvage.last_seen_ms)?;
        }
    }
    Ok(())
}

/// Unix milliseconds, now.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// The [`Inbox`] `try_deliver` uses in production: pops and requeues through
/// a real `Relay`. Borrows it for the lifetime of one `try_deliver` call, so
/// the pop and the (conditional) requeue share one `&mut Relay` instead of
/// needing two separate closures to each capture it.
struct RelayInbox<'a> {
    relay: &'a mut Relay,
    lead: &'a AgentName,
    /// The registry this tick already read: sender addresses come from it, not
    /// from a second read per delivery.
    registry: &'a [relay_core::Registration],
    /// The address book over [`Self::registry`], built by the first delivery of
    /// the tick and shared by the rest.
    book: &'a OnceCell<AddressBook>,
    /// The connection-level error that hit a pop, an address read or a
    /// requeue, if one did. `try_deliver` only reports the outcome as text, so
    /// `tick` reads this afterwards to end the tick and drop the connection.
    lost: Option<relay_core::Error>,
}

impl RelayInbox<'_> {
    /// Takes ownership of a failed call's error: keeps it in [`Self::lost`]
    /// when it is a connection loss (the first one wins), and returns its
    /// display text for the caller's own log or outcome.
    fn record(&mut self, err: relay_core::Error) -> String {
        let text = err.to_string();
        if err.is_connection() && self.lost.is_none() {
            self.lost = Some(err);
        }
        text
    }
}

impl Inbox for RelayInbox<'_> {
    /// Pops `lead`'s inbox, atomically and within [`MAX_PROMPT_BYTES`] of
    /// rendered text (see [`Relay::pop_inbox_budget`]), and renders what came
    /// back for `herdr agent prompt`. The batch is recorded in flight until
    /// [`Inbox::ack`]; anything over the budget is never popped, so there is no
    /// tail to put back.
    fn pop(&mut self) -> PopOutcome {
        let popped = match self.relay.pop_inbox_budget(
            self.lead,
            BATCH,
            MAX_PROMPT_BYTES,
            Claim::new(Popper::Watcher),
        ) {
            Ok(p) => p,
            Err(e) => return PopOutcome::Failed(self.record(e)),
        };
        for e in &popped.rejected {
            eprintln!(
                "relay-watcher: dropped one inbox entry for {}: {e}",
                self.lead
            );
        }
        let Some(batch) = popped.batch else {
            return if popped.at_cap {
                PopOutcome::AtCap
            } else {
                PopOutcome::Empty
            };
        };
        if popped.messages.is_empty() {
            // Only entries that do not decode: nothing to deliver, and
            // nothing to redeliver either.
            self.ack_batch(batch);
            return PopOutcome::Empty;
        }

        // Senders are shown by `server:workspace[:tab[:pane]]` address, from the
        // registry the tick already read.
        let book = self
            .book
            .get_or_init(|| AddressBook::from_registrations(self.registry));
        let rendered = popped
            .messages
            .iter()
            .zip(&popped.redelivered)
            .map(|(m, again)| book.render_marked(m, *again))
            .collect::<Vec<_>>()
            .join("\n");
        let mut senders: Vec<String> = Vec::new();
        for message in &popped.messages {
            let label = book.label(&message.from);
            if !senders.contains(&label) {
                senders.push(label);
            }
        }
        PopOutcome::Popped(PoppedBatch {
            batch,
            messages: popped.messages,
            rendered,
            senders: senders.join(", "),
        })
    }

    fn ack(&mut self, batch: &PoppedBatch) {
        self.ack_batch(batch.batch);
    }

    fn requeue(&mut self, batch: &PoppedBatch) {
        if let Err(e) =
            self.relay
                .requeue_batch(self.lead, batch.batch, relay_core::Popper::Watcher)
        {
            // Still in flight: its lease returns it.
            let text = self.record(e);
            eprintln!(
                "relay-watcher: {}: failed to requeue batch {}: {text}; left in flight",
                self.lead, batch.batch
            );
        }
    }

    fn dead_letter(&mut self, batch: &PoppedBatch) {
        if let Err(e) =
            self.relay
                .dead_letter_batch(self.lead, batch.batch, relay_core::Popper::Watcher)
        {
            // Still in flight: its lease returns it, and it would be shown again.
            let text = self.record(e);
            eprintln!(
                "relay-watcher: {}: failed to dead-letter batch {}: {text}; left in flight",
                self.lead, batch.batch
            );
        }
    }
}

impl RelayInbox<'_> {
    /// Acknowledges `batch`. A failure leaves it in flight, so it is delivered
    /// again, marked, when its lease expires: logged, because the lead may see
    /// it twice.
    fn ack_batch(&mut self, batch: relay_core::BatchId) {
        match self
            .relay
            .ack(self.lead, batch, relay_core::Popper::Watcher)
        {
            Ok(true) => {}
            Ok(false) => eprintln!(
                "relay-watcher: {}: batch {batch} was no longer in flight when acknowledged",
                self.lead
            ),
            Err(e) => {
                let text = self.record(e);
                eprintln!(
                    "relay-watcher: {}: failed to acknowledge batch {batch}: {text}; \
                     it will be redelivered",
                    self.lead
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use relay_core::{Message, Target};

    use super::*;
    use crate::{
        deliver::DeliverError,
        episodes::EpisodeKind,
        herdr::{AgentGet, AgentStatus, ProcessError},
        test_support::{Fault, FaultProxy, raw_command},
    };
    use relay_core::{MachineName, PaneId};

    // ---- tick ---------------------------------------------------------

    /// A `Herdr` that always reports one idle agent with the given session
    /// id, and records every prompt.
    struct StubHerdr {
        session_id: Option<&'static str>,
        prompts: std::cell::RefCell<Vec<(String, String)>>,
    }

    impl Herdr for StubHerdr {
        fn get(&self, _pane: &PaneId) -> Result<Option<AgentGet>, ProcessError> {
            Ok(Some(AgentGet {
                status: AgentStatus::Idle,
                state_change_seq: 1,
                session_id: self.session_id.map(|s| s.parse().unwrap()),
            }))
        }

        fn notify(&self, _title: &str, _body: &str) -> Result<(), ProcessError> {
            Ok(())
        }
    }

    impl Deliverer for StubHerdr {
        fn deliver(&self, pane: &PaneId, text: &str) -> Result<(), DeliverError> {
            self.prompts
                .borrow_mut()
                .push((pane.to_string(), text.to_owned()));
            Ok(())
        }
    }

    /// A `Herdr` that reports no places and a settable count of dropped entries.
    struct DroppingHerdr(std::cell::Cell<usize>);

    impl Herdr for DroppingHerdr {
        fn get(&self, _pane: &PaneId) -> Result<Option<AgentGet>, ProcessError> {
            Ok(None)
        }
        fn notify(&self, _title: &str, _body: &str) -> Result<(), ProcessError> {
            Ok(())
        }
        fn dropped_entries(&self) -> usize {
            self.0.get()
        }
    }

    impl Deliverer for DroppingHerdr {
        fn deliver(&self, _pane: &PaneId, _text: &str) -> Result<(), DeliverError> {
            Ok(())
        }
    }

    /// Herdr entries this binary could not decode are reported once per episode:
    /// not on every place refresh, and again after a clean listing in between.
    #[test]
    fn dropped_herdr_entries_are_reported_once_per_episode() {
        let Some(mut relay) = test_relay("dropped-entries") else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        let herdr = DroppingHerdr(std::cell::Cell::new(2));
        let mut state = WatcherState::default();
        let mut lines = Vec::new();
        let machine: MachineName = "node-a".parse().unwrap();
        for dropped in [2, 2, 2, 0, 1] {
            herdr.0.set(dropped);
            state.last_place_refresh = None;
            tick(&mut relay, &herdr, &machine, &mut state, &mut |l| {
                lines.push(l)
            })
            .unwrap();
        }
        let reports: Vec<&String> = lines
            .iter()
            .filter(|l| l.contains("herdr layout"))
            .collect();
        assert_eq!(reports.len(), 2, "one per episode: {lines:?}");
        assert!(
            reports[0].starts_with("2 herdr layout entries"),
            "{reports:?}"
        );
        assert!(
            reports[1].starts_with("1 herdr layout entry"),
            "{reports:?}"
        );
    }

    /// A `Herdr` whose only job is to report where panes sit.
    struct LayoutHerdr(
        std::collections::HashMap<PaneId, relay_core::Place>,
        std::collections::HashMap<PaneId, Option<SessionId>>,
    );

    fn layout_herdr(
        mut places: std::collections::HashMap<PaneId, relay_core::Place>,
        sessions: &[(&str, &str)],
    ) -> LayoutHerdr {
        for (pane, place) in &mut places {
            if let Some((_, session)) = sessions
                .iter()
                .find(|(listed_pane, _)| listed_pane.parse::<PaneId>().ok().as_ref() == Some(pane))
            {
                place.herdr_ids = Some(relay_core::HerdrPlaceIds {
                    workspace: "test-workspace".parse().unwrap(),
                    tab: "test-tab".parse().unwrap(),
                    pane: pane.clone(),
                    session_id: session.parse().unwrap(),
                    verified_at_ms: now_ms(),
                });
            }
        }
        LayoutHerdr(
            places,
            sessions
                .iter()
                .map(|(pane, session)| (pane.parse().unwrap(), Some(session.parse().unwrap())))
                .collect(),
        )
    }

    impl Herdr for LayoutHerdr {
        fn get(&self, _pane: &PaneId) -> Result<Option<AgentGet>, ProcessError> {
            Ok(None)
        }
        fn notify(&self, _title: &str, _body: &str) -> Result<(), ProcessError> {
            Ok(())
        }
        fn layout(&self, _machine: &MachineName) -> Result<crate::herdr::Layout, ProcessError> {
            Ok(crate::herdr::Layout {
                places: self.0.clone(),
                sessions: self.1.clone(),
                view: None,
            })
        }
    }

    impl Deliverer for LayoutHerdr {
        fn deliver(&self, _pane: &PaneId, _text: &str) -> Result<(), DeliverError> {
            Ok(())
        }
    }

    /// A herdr whose layout is `Some(view)` or, when `None`, a failed read.
    struct ViewHerdr(Option<relay_ops::find::MachineView>);

    impl Herdr for ViewHerdr {
        fn get(&self, _pane: &PaneId) -> Result<Option<AgentGet>, ProcessError> {
            Ok(None)
        }
        fn notify(&self, _title: &str, _body: &str) -> Result<(), ProcessError> {
            Ok(())
        }
        fn layout(&self, _machine: &MachineName) -> Result<crate::herdr::Layout, ProcessError> {
            match &self.0 {
                Some(view) => Ok(crate::herdr::Layout {
                    view: Some(view.clone()),
                    ..crate::herdr::Layout::default()
                }),
                None => Err(ProcessError::Timeout {
                    program: "herdr".into(),
                    secs: 10,
                }),
            }
        }
    }

    impl Deliverer for ViewHerdr {
        fn deliver(&self, _pane: &PaneId, _text: &str) -> Result<(), DeliverError> {
            Ok(())
        }
    }

    /// A publisher that hands each request body it is asked to send to `bodies`,
    /// with a zero interval so every tick pushes.
    fn recording_publisher(
        here: &MachineName,
        interval: Duration,
    ) -> (otlp::Publisher, std::sync::mpsc::Receiver<Vec<u8>>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let publisher = otlp::Publisher::with_transport(
            otlp::encode::Resource::new("svc", None, here.as_str(), "0.0.0", &[]),
            "collector:4318".into(),
            interval,
            here,
            Box::new(otlp::test_collector::Recording(tx)),
            |_| {},
        )
        .unwrap();
        (publisher, rx)
    }

    /// The tick's layout refresh hands the herdr answer it read to the OTLP push,
    /// joined with the registry, and the push carries the panes with their
    /// registered names; a tick whose layout cannot be read pushes no panes once
    /// they are too old, and never fails (FR-045-AC-5). Deleting the tick's
    /// publish or panes call fails this. Redis lane.
    ///
    /// Tracing: TC-045-05
    /// ACs: FR-045-AC-5
    #[test]
    fn tc_045_05_a_tick_pushes_the_layout_it_read() {
        let Some(mut relay) = test_relay("otlp-wiring") else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        register_at(
            &mut relay,
            "here-lead",
            "here",
            Some("wM:p1"),
            Some("s1"),
            now_ms(),
        );
        let view = relay_ops::find::MachineView {
            machine: "here".parse().unwrap(),
            agents: vec![relay_ops::find::HerdrAgent {
                pane_id: "wM:p1".parse().unwrap(),
                tab_id: "wM:t1".parse().unwrap(),
                workspace_id: "wM".parse().unwrap(),
                status: "idle".into(),
                session_name: None,
                session_id: Some("s1".parse().unwrap()),
            }],
            workspaces: vec![relay_ops::find::HerdrWorkspace {
                workspace_id: "wM".parse().unwrap(),
                label: "EA".into(),
                tab_count: 1,
            }],
            tabs: vec![relay_ops::find::HerdrTab {
                tab_id: "wM:t1".parse().unwrap(),
                label: "code".into(),
            }],
            dropped: 0,
        };
        let here: MachineName = "here".parse().unwrap();
        let (publisher, bodies) = recording_publisher(&here, Duration::ZERO);
        let mut state = WatcherState {
            otlp: publisher,
            ..WatcherState::default()
        };

        tick(
            &mut relay,
            &ViewHerdr(Some(view)),
            &here,
            &mut state,
            &mut |_| {},
        )
        .unwrap();
        let body = String::from_utf8(bodies.recv().unwrap()).unwrap();
        assert!(
            body.contains("\"workspace_label\",\"value\":{\"stringValue\":\"EA\"}")
                && body.contains("\"stringValue\":\"here-lead\""),
            "the push carries the pane with its registered name: {body}"
        );
        assert!(
            body.contains("ix.relay.inbox.depth"),
            "and the relay's health: {body}"
        );
        assert!(
            body.contains("ix.relay.watcher.tick.duration"),
            "and the tick's own duration: {body}"
        );

        // The next refresh (due again) cannot read herdr: the tick still succeeds
        // and still pushes; the panes stay while they are fresh.
        state.last_place_refresh = None;
        tick(&mut relay, &ViewHerdr(None), &here, &mut state, &mut |_| {}).unwrap();
        let body = String::from_utf8(bodies.recv().unwrap()).unwrap();
        assert!(
            body.contains("here-lead"),
            "last panes stay while fresh: {body}"
        );
    }

    fn place(workspace: &str, tab: &str, _tabs: u32) -> relay_core::Place {
        relay_core::Place {
            workspace: workspace.into(),
            tab: tab.into(),
            herdr_ids: None,
        }
    }

    #[test]
    fn tick_refreshes_a_stale_place_from_herdrs_live_layout_for_local_leads_only() {
        let Some(mut relay) = test_relay("refresh-places") else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        // Registered before addresses existed (no place), and one with a stale one.
        register_at(
            &mut relay,
            "here-old",
            "here",
            Some("wB:p1"),
            Some("s1"),
            now_ms(),
        );
        register_at(
            &mut relay,
            "here-stale",
            "here",
            Some("wB:p2"),
            Some("s2"),
            now_ms(),
        );
        register_at(
            &mut relay,
            "there-lead",
            "there",
            Some("wB:p1"),
            Some("s3"),
            now_ms(),
        );
        let stale = relay_core::Registration {
            place: Some(place("EA", "2", 3)),
            ..relay
                .agent(&"here-stale".parse().unwrap())
                .unwrap()
                .unwrap()
        };
        relay.register(&stale).unwrap();

        // herdr now: p1 is EA/code, p2 is what used to be tab "3", now "2".
        let herdr = layout_herdr(
            [
                ("wB:p1".parse().unwrap(), place("EA", "code", 2)),
                ("wB:p2".parse().unwrap(), place("EA", "2", 2)),
            ]
            .into_iter()
            .collect(),
            &[("wB:p1", "s1"), ("wB:p2", "s2")],
        );
        let mut state = WatcherState::default();
        tick(
            &mut relay,
            &herdr,
            &"here".parse().unwrap(),
            &mut state,
            &mut |_| {},
        )
        .unwrap();

        let place_of = |relay: &mut Relay, name: &str| {
            relay.agent(&name.parse().unwrap()).unwrap().unwrap().place
        };
        let old = place_of(&mut relay, "here-old").unwrap();
        assert_eq!((old.workspace.as_str(), old.tab.as_str()), ("EA", "code"));
        assert!(
            old.herdr_ids.is_some(),
            "the refreshed place carries Herdr proof"
        );
        let stale = place_of(&mut relay, "here-stale").unwrap();
        assert_eq!((stale.workspace.as_str(), stale.tab.as_str()), ("EA", "2"));
        assert!(
            stale.herdr_ids.is_some(),
            "the refreshed place carries Herdr proof"
        );
        assert_eq!(
            place_of(&mut relay, "there-lead"),
            None,
            "a lead on another machine is not this watcher's to update"
        );
    }

    #[test]
    fn place_refresh_is_throttled_between_ticks() {
        let Some(mut relay) = test_relay("refresh-places-throttle") else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        register_at(
            &mut relay,
            "here-lead",
            "here",
            Some("wB:p1"),
            Some("s1"),
            now_ms(),
        );
        let herdr = layout_herdr(
            [("wB:p1".parse().unwrap(), place("EA", "code", 2))]
                .into_iter()
                .collect(),
            &[("wB:p1", "s1")],
        );
        let mut state = WatcherState::default();
        tick(
            &mut relay,
            &herdr,
            &"here".parse().unwrap(),
            &mut state,
            &mut |_| {},
        )
        .unwrap();
        // herdr's layout changes, but the very next tick must not re-read it.
        let moved = layout_herdr(
            [("wB:p1".parse().unwrap(), place("EA", "plan", 2))]
                .into_iter()
                .collect(),
            &[("wB:p1", "s1")],
        );
        tick(
            &mut relay,
            &moved,
            &"here".parse().unwrap(),
            &mut state,
            &mut |_| {},
        )
        .unwrap();
        let got = relay
            .agent(&"here-lead".parse().unwrap())
            .unwrap()
            .unwrap()
            .place;
        let got = got.unwrap();
        assert_eq!((got.workspace.as_str(), got.tab.as_str()), ("EA", "code"));
        assert!(
            got.herdr_ids.is_some(),
            "the refreshed place carries Herdr proof"
        );
    }

    fn test_relay(test: &str) -> Option<Relay> {
        let url = std::env::var("IX_RELAY_TEST_URL").ok()?;
        let relay = Relay::connect(&url, Duration::from_secs(2)).expect("connect to test redis");
        Some(relay.with_prefix(test_prefix(test)))
    }

    fn test_prefix(test: &str) -> String {
        format!("test:{}:{test}:", std::process::id())
    }

    /// A `Relay` for `test`'s keyspace that reaches Redis through `proxy`.
    fn relay_via(proxy: &FaultProxy, test: &str) -> Relay {
        Relay::connect(&proxy.url, Duration::from_millis(400))
            .expect("connect through the proxy")
            .with_prefix(test_prefix(test))
    }

    fn queue(relay: &mut Relay, name: &str) {
        relay
            .send(&Message {
                from: "sender".parse().unwrap(),
                to: Target::Agent(lead(name)),
                text: "hi".into(),
                sent_at_ms: 0,
            })
            .unwrap();
    }

    fn idle_herdr(session: &'static str) -> StubHerdr {
        StubHerdr {
            session_id: Some(session),
            prompts: std::cell::RefCell::default(),
        }
    }

    fn register(
        relay: &mut Relay,
        name: &str,
        machine: &str,
        pane: Option<&str>,
        session_id: Option<&str>,
    ) {
        register_at(relay, name, machine, pane, session_id, 1);
    }

    /// Like [`register`], but with a caller-controlled `last_seen_ms` — the
    /// pruning tests need registrations that read as either fresh or a full
    /// day-plus stale.
    fn register_at(
        relay: &mut Relay,
        name: &str,
        machine: &str,
        pane: Option<&str>,
        session_id: Option<&str>,
        last_seen_ms: i64,
    ) {
        relay
            .register(&relay_core::Registration {
                name: name.parse().unwrap(),
                machine: machine.parse().unwrap(),
                kind: "claude".into(),
                session_id: session_id.map(|s| s.parse().unwrap()),
                cwd: None,
                herdr_pane: pane.map(|s| s.parse().unwrap()),
                place: None,
                last_seen_ms,
            })
            .unwrap();
    }

    #[test]
    fn tick_only_touches_leads_on_this_machine() {
        let Some(mut relay) = test_relay("machine-filter") else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        register(&mut relay, "here-lead", "node-a", Some("w1:p1"), Some("s1"));
        register(&mut relay, "there-lead", "hub", Some("w1:p1"), Some("s1"));
        relay
            .send(&Message {
                from: "sender".parse().unwrap(),
                to: Target::Agent("here-lead".parse().unwrap()),
                text: "for here".into(),
                sent_at_ms: 0,
            })
            .unwrap();
        relay
            .send(&Message {
                from: "sender".parse().unwrap(),
                to: Target::Agent("there-lead".parse().unwrap()),
                text: "for there".into(),
                sent_at_ms: 0,
            })
            .unwrap();

        let herdr = StubHerdr {
            session_id: Some("s1"),
            prompts: std::cell::RefCell::default(),
        };
        let mut state = WatcherState::default();
        tick(
            &mut relay,
            &herdr,
            &"node-a".parse().unwrap(),
            &mut state,
            &mut |_| {},
        )
        .unwrap();

        assert_eq!(
            herdr.prompts.borrow().len(),
            1,
            "only the node-a lead is prompted"
        );
        assert_eq!(relay.inbox_len(&"here-lead".parse().unwrap()).unwrap(), 0);
        assert_eq!(
            relay.inbox_len(&"there-lead".parse().unwrap()).unwrap(),
            1,
            "the hub lead's message is untouched"
        );
    }

    #[test]
    fn tick_matches_a_registration_stored_with_uppercase_or_domain_suffixed_machine() {
        let Some(mut relay) = test_relay("machine-case-normalized") else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        // Stored exactly as a raw, un-normalized hostname would read — e.g.
        // `gethostname` on node-b reporting "Node-B.local" — planted as JSON, since
        // the typed `MachineName` would normalise it on the way in. Decoding
        // normalises it (`MachineName`, the one place), so it must still match
        // the already-normalized machine `relay_core::config` resolves.
        let url = std::env::var("IX_RELAY_TEST_URL").expect("set: test_relay returned Some");
        let mut raw = redis::Client::open(url)
            .and_then(|c| c.get_connection())
            .expect("raw connection to test redis");
        redis::cmd("HSET")
            .arg(format!("{}agents", test_prefix("machine-case-normalized")))
            .arg("node-b-lead")
            .arg(r#"{"name":"node-b-lead","machine":"Node-B.local","kind":"claude","session_id":"s1","cwd":null,"herdr_pane":"w1:p1","place":null,"last_seen_ms":1}"#)
            .exec(&mut raw)
            .expect("plant the raw registration");
        relay
            .send(&Message {
                from: "sender".parse().unwrap(),
                to: Target::Agent("node-b-lead".parse().unwrap()),
                text: "for node-b".into(),
                sent_at_ms: 0,
            })
            .unwrap();

        let herdr = StubHerdr {
            session_id: Some("s1"),
            prompts: std::cell::RefCell::default(),
        };
        let mut state = WatcherState::default();
        // "node-b": what `main` would pass, already normalized by
        // `relay_core::config`'s `CoreSettings::machine`.
        tick(
            &mut relay,
            &herdr,
            &"node-b".parse().unwrap(),
            &mut state,
            &mut |_| {},
        )
        .unwrap();

        assert_eq!(
            herdr.prompts.borrow().len(),
            1,
            "the differently-cased registration is still matched"
        );
        assert_eq!(relay.inbox_len(&"node-b-lead".parse().unwrap()).unwrap(), 0);
    }

    #[test]
    fn tick_skips_leads_with_no_herdr_pane() {
        let Some(mut relay) = test_relay("no-pane") else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        register(&mut relay, "no-pane-lead", "node-a", None, Some("s1"));
        relay
            .send(&Message {
                from: "sender".parse().unwrap(),
                to: Target::Agent("no-pane-lead".parse().unwrap()),
                text: "hi".into(),
                sent_at_ms: 0,
            })
            .unwrap();

        let herdr = StubHerdr {
            session_id: Some("s1"),
            prompts: std::cell::RefCell::default(),
        };
        let mut state = WatcherState::default();
        tick(
            &mut relay,
            &herdr,
            &"node-a".parse().unwrap(),
            &mut state,
            &mut |_| {},
        )
        .unwrap();

        assert!(herdr.prompts.borrow().is_empty());
        assert_eq!(
            relay.inbox_len(&"no-pane-lead".parse().unwrap()).unwrap(),
            1,
            "left queued: there's no pane to type into"
        );
    }

    #[test]
    fn tick_tracks_the_blocked_set_across_calls() {
        let Some(mut relay) = test_relay("blocked-lifecycle") else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        register(&mut relay, "tl-lead", "node-a", Some("w1:p1"), Some("s1"));
        relay
            .send(&Message {
                from: "sender".parse().unwrap(),
                to: Target::Agent("tl-lead".parse().unwrap()),
                text: "hi".into(),
                sent_at_ms: 0,
            })
            .unwrap();

        struct BlockedHerdr;
        impl Herdr for BlockedHerdr {
            fn get(&self, _pane: &PaneId) -> Result<Option<AgentGet>, ProcessError> {
                Ok(Some(AgentGet {
                    status: AgentStatus::Blocked,
                    state_change_seq: 1,
                    session_id: Some("s1".parse().unwrap()),
                }))
            }
            fn notify(&self, _title: &str, _body: &str) -> Result<(), ProcessError> {
                Ok(())
            }
        }

        impl Deliverer for BlockedHerdr {
            fn deliver(&self, _pane: &PaneId, _text: &str) -> Result<(), DeliverError> {
                panic!("must not prompt a blocked lead")
            }
        }

        let mut state = WatcherState::default();
        tick(
            &mut relay,
            &BlockedHerdr,
            &"node-a".parse().unwrap(),
            &mut state,
            &mut |_| {},
        )
        .unwrap();
        assert!(state.episodes.kind(&"tl-lead".parse().unwrap()) == Some(EpisodeKind::Blocked));

        // Draining the inbox (a human answers the dialog and the hook
        // delivers) must clear the tracked episode on the next tick.
        relay
            .pop_inbox(&"tl-lead".parse().unwrap(), NonZeroUsize::new(50).unwrap())
            .unwrap();
        tick(
            &mut relay,
            &BlockedHerdr,
            &"node-a".parse().unwrap(),
            &mut state,
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(
            state.episodes.kind(&"tl-lead".parse().unwrap()),
            None,
            "an empty inbox ends the tracked episode"
        );
    }

    #[test]
    fn tick_skips_a_stale_registration_and_leaves_the_message_queued() {
        let Some(mut relay) = test_relay("stale-session") else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        // Registered under one session id; herdr now reports a different one
        // (the pane's agent was restarted).
        register(
            &mut relay,
            "tl-lead",
            "node-a",
            Some("w1:p1"),
            Some("old-session"),
        );
        relay
            .send(&Message {
                from: "sender".parse().unwrap(),
                to: Target::Agent("tl-lead".parse().unwrap()),
                text: "hi".into(),
                sent_at_ms: 0,
            })
            .unwrap();

        let herdr = StubHerdr {
            session_id: Some("new-session"),
            prompts: std::cell::RefCell::default(),
        };
        let mut state = WatcherState::default();
        tick(
            &mut relay,
            &herdr,
            &"node-a".parse().unwrap(),
            &mut state,
            &mut |_| {},
        )
        .unwrap();

        assert!(
            herdr.prompts.borrow().is_empty(),
            "never types into a stale registration"
        );
        assert_eq!(relay.inbox_len(&"tl-lead".parse().unwrap()).unwrap(), 1);
        assert!(state.episodes.kind(&"tl-lead".parse().unwrap()) == Some(EpisodeKind::Stale));
    }

    /// AGE-2026: a batch over the prompt budget is never popped in the first
    /// place, so nothing needs putting back and a crash cannot lose the tail.
    #[test]
    fn pop_leaves_messages_over_the_prompt_budget_queued() {
        let Some(mut relay) = test_relay("pop-budget") else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        let lead: AgentName = "tl-lead".parse().unwrap();
        register(&mut relay, "tl-lead", "node-a", Some("w1:p1"), Some("s1"));
        for i in 0..8 {
            relay
                .send(&Message {
                    from: "sender".parse().unwrap(),
                    to: Target::Agent(lead.clone()),
                    text: format!("{i}{}", "x".repeat(3000)),
                    sent_at_ms: i,
                })
                .unwrap();
        }

        let book = OnceCell::new();
        let mut inbox = RelayInbox {
            relay: &mut relay,
            lead: &lead,
            registry: &[],
            book: &book,
            lost: None,
        };
        let PopOutcome::Popped(batch) = inbox.pop() else {
            panic!("expected a batch");
        };
        assert_eq!(batch.messages.len(), 5, "5 x ~3.1KB fits 16KiB, 6 does not");
        assert!(batch.messages[0].text.starts_with('0'));
        assert_eq!(
            relay.inbox_len(&lead).unwrap(),
            3,
            "the tail was never popped"
        );
    }

    // ---- the in-flight list (AGE-2032) --------------------------------------

    /// A `Herdr` that reports one idle agent in session `s1` and answers every
    /// prompt with `result`.
    struct ResultHerdr {
        result: Result<(), DeliverError>,
        prompts: std::cell::RefCell<Vec<String>>,
    }

    impl ResultHerdr {
        fn answering(result: Result<(), DeliverError>) -> Self {
            Self {
                result,
                prompts: std::cell::RefCell::default(),
            }
        }
    }

    impl Herdr for ResultHerdr {
        fn get(&self, _pane: &PaneId) -> Result<Option<AgentGet>, ProcessError> {
            Ok(Some(AgentGet {
                status: AgentStatus::Idle,
                state_change_seq: 1,
                session_id: Some("s1".parse().unwrap()),
            }))
        }
        fn notify(&self, _title: &str, _body: &str) -> Result<(), ProcessError> {
            Ok(())
        }
    }

    impl Deliverer for ResultHerdr {
        fn deliver(&self, _pane: &PaneId, text: &str) -> Result<(), DeliverError> {
            self.prompts.borrow_mut().push(text.to_owned());
            self.result.clone()
        }
    }

    /// A relay with `tl-lead` registered on `node-a` and one message queued.
    fn one_lead_with_mail(test: &str) -> Option<Relay> {
        let mut relay = test_relay(test)?;
        register(&mut relay, "tl-lead", "node-a", Some("w1:p1"), Some("s1"));
        queue(&mut relay, "tl-lead");
        Some(relay)
    }

    fn tick_once(relay: &mut Relay, herdr: &ResultHerdr) {
        tick(
            relay,
            herdr,
            &"node-a".parse().unwrap(),
            &mut WatcherState::default(),
            &mut |_| {},
        )
        .unwrap();
    }

    /// Tracing: TC-020-03
    /// ACs: FR-020-AC-3
    ///
    /// A prompt herdr reports as delivered is acknowledged: the batch is not left
    /// in flight, so nothing is delivered twice.
    #[test]
    fn a_delivered_prompt_is_acknowledged() {
        let Some(mut relay) = one_lead_with_mail("inflight-delivered") else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        let herdr = ResultHerdr::answering(Ok(()));
        tick_once(&mut relay, &herdr);
        assert_eq!(herdr.prompts.borrow().len(), 1);
        assert_eq!(relay.inflight_len(&lead("tl-lead")).unwrap(), 0);
        assert_eq!(relay.inbox_len(&lead("tl-lead")).unwrap(), 0);
    }

    /// Tracing: TC-020-06
    /// ACs: FR-020-AC-6
    ///
    /// A prompt whose outcome is unknown is neither acknowledged nor requeued:
    /// the batch stays in flight, and once its lease is out the reap returns it,
    /// and the next tick types it again, marked as a redelivery.
    #[test]
    fn an_ambiguous_prompt_is_redelivered_marked_after_its_lease() {
        let Some(mut relay) = one_lead_with_mail("inflight-ambiguous") else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        let stalled = ResultHerdr::answering(Err(DeliverError::Ambiguous("timed out".into())));
        tick_once(&mut relay, &stalled);
        let tl = lead("tl-lead");
        assert_eq!(stalled.prompts.borrow().len(), 1);
        assert_eq!(relay.inbox_len(&tl).unwrap(), 0);
        assert_eq!(relay.inflight_len(&tl).unwrap(), 1, "not acknowledged");
        assert!(!stalled.prompts.borrow()[0].contains("(redelivered)"));

        // Inside the lease nothing returns; past it the reap does.
        let inside = relay.reap_expired(&tl, Now::AtMs(now_ms())).unwrap();
        assert_eq!(inside.requeued, 0);
        let later =
            Now::AtMs(now_ms() + 2 * i64::try_from(relay_core::WATCHER_LEASE.as_millis()).unwrap());
        assert_eq!(relay.reap_expired(&tl, later).unwrap().requeued, 1);

        let fine = ResultHerdr::answering(Ok(()));
        tick_once(&mut relay, &fine);
        assert_eq!(fine.prompts.borrow().len(), 1);
        assert!(
            fine.prompts.borrow()[0].contains("from sender (redelivered): hi"),
            "{:?}",
            fine.prompts
        );
        assert_eq!(relay.inflight_len(&tl).unwrap(), 0);
    }

    /// Tracing: TC-020-03
    /// ACs: FR-020-AC-3
    ///
    /// A watcher that crashed between its pop and its prompt: the batch is in
    /// flight and never typed. A new watcher's first sweep (its start-up) returns
    /// the batch once the lease is out and delivers it, marked.
    #[test]
    fn a_batch_popped_by_a_crashed_watcher_is_delivered_by_the_next_sweep() {
        let Some(mut relay) = one_lead_with_mail("inflight-crashed") else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        let tl = lead("tl-lead");
        let crashed = relay
            .pop_inbox_budget(
                &tl,
                BATCH,
                MAX_PROMPT_BYTES,
                Claim::new(Popper::Watcher).with_lease(Duration::ZERO),
            )
            .unwrap();
        assert_eq!(crashed.messages.len(), 1);
        assert_eq!(relay.inbox_len(&tl).unwrap(), 0);

        let herdr = ResultHerdr::answering(Ok(()));
        tick_once(&mut relay, &herdr);
        assert_eq!(herdr.prompts.borrow().len(), 1);
        assert!(herdr.prompts.borrow()[0].contains("(redelivered)"));
        assert_eq!(relay.inflight_len(&tl).unwrap(), 0);
    }

    /// Tracing: TC-013-02
    /// ACs: FR-013-AC-2, FR-020-AC-3
    ///
    /// A prompt herdr documents as not sent goes back to the front of the inbox
    /// through the in-flight record: nothing is left in flight, and the messages
    /// are not marked, because nobody saw them.
    #[test]
    fn a_prompt_that_was_not_sent_returns_the_batch_unmarked() {
        let Some(mut relay) = one_lead_with_mail("inflight-not-sent") else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        let refused = ResultHerdr::answering(Err(DeliverError::NotSent("agent_blocked".into())));
        tick_once(&mut relay, &refused);
        let tl = lead("tl-lead");
        assert_eq!(relay.inbox_len(&tl).unwrap(), 1);
        assert_eq!(relay.inflight_len(&tl).unwrap(), 0);
        let fine = ResultHerdr::answering(Ok(()));
        tick_once(&mut relay, &fine);
        assert!(!fine.prompts.borrow()[0].contains("(redelivered)"));
    }

    /// Tracing: TC-020-14
    /// ACs: FR-020-AC-14
    ///
    /// A lead already at the in-flight cap is not "emptied by its hook": the tick
    /// says the cap is reached, once when the episode starts, and not again while it
    /// lasts; when a lease frees room the mail is delivered and the episode ends.
    #[test]
    fn a_lead_at_the_in_flight_cap_is_logged_once_per_episode() {
        let Some(mut relay) = one_lead_with_mail("inflight-capped") else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        let tl = lead("tl-lead");
        // The lead's own hook has sixteen batches out. (One message each.)
        for _ in 1..relay_core::MAX_INFLIGHT_BATCHES {
            queue(&mut relay, "tl-lead");
        }
        for _ in 0..relay_core::MAX_INFLIGHT_BATCHES {
            let popped = relay
                .pop_inbox_budget(
                    &tl,
                    NonZeroUsize::MIN,
                    MAX_PROMPT_BYTES,
                    Claim::new(Popper::Hook),
                )
                .unwrap();
            assert_eq!(popped.messages.len(), 1);
        }
        queue(&mut relay, "tl-lead");
        let herdr = ResultHerdr::answering(Ok(()));
        let mut state = WatcherState::default();
        let mut logs = Vec::new();
        for _ in 0..3 {
            tick(
                &mut relay,
                &herdr,
                &"node-a".parse().unwrap(),
                &mut state,
                &mut |l| logs.push(l),
            )
            .unwrap();
        }
        let capped = logs
            .iter()
            .filter(|l| l.contains("in-flight cap reached"))
            .count();
        assert_eq!(capped, 1, "once per episode: {logs:?}");
        assert!(
            !logs.iter().any(|l| l.contains("emptied by its hook")),
            "not misreported: {logs:?}"
        );
        assert!(herdr.prompts.borrow().is_empty());
        assert_eq!(relay.inbox_len(&tl).unwrap(), 1, "the mail waits");
    }

    /// Tracing: TC-020-14
    /// ACs: FR-020-AC-14
    ///
    /// A lead whose in-flight keys cannot be used fails its reap on every sweep; that
    /// is one line when it starts and one when it ends, not one per sweep.
    #[test]
    fn a_reap_that_keeps_failing_is_logged_once_per_episode() {
        let Some(mut relay) = one_lead_with_mail("inflight-reap-failing") else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        let tl = lead("tl-lead");
        let key = format!("{}inflight:tl-lead", test_prefix("inflight-reap-failing"));
        let url = std::env::var("IX_RELAY_TEST_URL").unwrap();
        raw_command(&url, &["SET", &key, "x"]);
        let herdr = ResultHerdr::answering(Err(DeliverError::NotSent("agent_blocked".into())));
        let mut state = WatcherState::default();
        let mut logs = Vec::new();
        for _ in 0..3 {
            tick(
                &mut relay,
                &herdr,
                &"node-a".parse().unwrap(),
                &mut state,
                &mut |l| logs.push(l),
            )
            .unwrap();
        }
        let failing = logs
            .iter()
            .filter(|l| l.contains("reaping in-flight failed"))
            .count();
        assert_eq!(failing, 1, "{logs:?}");
        // Repaired: the next sweep says so, once.
        raw_command(&url, &["DEL", &key]);
        for _ in 0..2 {
            tick(
                &mut relay,
                &herdr,
                &"node-a".parse().unwrap(),
                &mut state,
                &mut |l| logs.push(l),
            )
            .unwrap();
        }
        assert_eq!(
            logs.iter()
                .filter(|l| l.contains("in-flight reap is healthy again"))
                .count(),
            1,
            "{logs:?}"
        );
        drop(tl);
    }

    // ---- maybe_prune (AGE-2018 PR review, finding 7) ----------------------

    /// A `Herdr` whose `get` answer is fixed for the whole test, so the
    /// pruning tests can say exactly what herdr reports for the one pane in
    /// play (no agent there at all, or a specific session id).
    struct PruneStubHerdr {
        answer: Option<&'static str>,
    }

    impl Herdr for PruneStubHerdr {
        fn get(&self, _pane: &PaneId) -> Result<Option<AgentGet>, ProcessError> {
            Ok(self.answer.map(|session_id| AgentGet {
                status: AgentStatus::Idle,
                state_change_seq: 1,
                session_id: Some(session_id.parse().unwrap()),
            }))
        }
        fn notify(&self, _title: &str, _body: &str) -> Result<(), ProcessError> {
            Ok(())
        }
    }

    impl Deliverer for PruneStubHerdr {
        fn deliver(&self, _pane: &PaneId, _text: &str) -> Result<(), DeliverError> {
            panic!("pruning tests never deliver")
        }
    }

    #[test]
    fn maybe_prune_leaves_a_young_registration_alone() {
        let Some(mut relay) = test_relay("prune-young") else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        // Registered "just now": well under PRUNE_AGE, even though herdr
        // reports nobody in the pane any more.
        register_at(
            &mut relay,
            "young-lead",
            "node-a",
            Some("w1:p1"),
            Some("s1"),
            now_ms(),
        );

        let herdr = PruneStubHerdr { answer: None };
        maybe_prune(
            &mut relay,
            &herdr,
            &"young-lead".parse().unwrap(),
            &"w1:p1".parse().unwrap(),
            Some(&"s1".parse().unwrap()),
            now_ms(),
        )
        .unwrap();

        assert!(
            relay
                .agent(&"young-lead".parse().unwrap())
                .unwrap()
                .is_some(),
            "too young to prune, regardless of liveness"
        );
    }

    #[test]
    fn maybe_prune_removes_an_old_abandoned_registration_and_its_aliases() {
        let Some(mut relay) = test_relay("prune-old-dead") else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        let stale_seen = now_ms() - i64::try_from(PRUNE_AGE.as_millis()).unwrap() - 60_000;
        register_at(
            &mut relay,
            "old-lead",
            "node-a",
            Some("w1:p1"),
            Some("s1"),
            stale_seen,
        );
        relay
            .set_alias(&"old-nick".parse().unwrap(), &"old-lead".parse().unwrap())
            .unwrap();

        // herdr reports no agent at all in that pane any more.
        let herdr = PruneStubHerdr { answer: None };
        maybe_prune(
            &mut relay,
            &herdr,
            &"old-lead".parse().unwrap(),
            &"w1:p1".parse().unwrap(),
            Some(&"s1".parse().unwrap()),
            stale_seen,
        )
        .unwrap();

        assert!(
            relay.agent(&"old-lead".parse().unwrap()).unwrap().is_none(),
            "old + abandoned pane => pruned"
        );
        assert_eq!(
            relay.resolve_alias(&"old-nick".parse().unwrap()).unwrap(),
            "old-nick".parse().unwrap(),
            "the alias pointing at the pruned name is removed too (resolves to itself, unknown)"
        );
    }

    /// Tracing: TC-008-06
    /// ACs: FR-008-AC-6
    ///
    /// Stale records this binary refuses to decode (an older writer's empty,
    /// 300-byte, tab or zero-width session id, a zero-width pane) are still
    /// pruned, as `main` pruned all five: the rule reads what it needs from the
    /// raw JSON. A young one and one on another machine are left alone; the cases where herdr's
    /// answer keeps a stale record are `refused_records_are_kept_unless_the_evidence_proves_them_stale`.
    #[test]
    fn stale_records_the_registry_read_refuses_are_still_pruned() {
        let Some(mut relay) = test_relay("prune-refused") else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        let url = std::env::var("IX_RELAY_TEST_URL").expect("set: test_relay returned Some");
        let mut raw = redis::Client::open(url)
            .and_then(|c| c.get_connection())
            .expect("raw connection to test redis");
        let stale = now_ms() - i64::try_from(PRUNE_AGE.as_millis()).unwrap() - 60_000;
        let week_old =
            now_ms() - i64::try_from(REFUSED_PANE_PRUNE_AGE.as_millis()).unwrap() - 60_000;
        let long = format!("\"{}\"", "x".repeat(300));
        let plant = |raw: &mut redis::Connection,
                     name: &str,
                     machine: &str,
                     session: &str,
                     pane: &str,
                     seen: i64| {
            let json = format!(
                r#"{{"name":"{name}","machine":"{machine}","kind":"claude","session_id":{session},"cwd":null,"herdr_pane":{pane},"place":null,"last_seen_ms":{seen}}}"#
            );
            redis::cmd("HSET")
                .arg(format!("{}agents", test_prefix("prune-refused")))
                .arg(name)
                .arg(json)
                .exec(raw)
                .expect("plant");
        };
        for (name, session, pane) in [
            ("empty-sid", "\"\"", "\"w1:p1\""),
            ("long-sid", long.as_str(), "\"w1:p2\""),
            ("tab-sid", "\"a\\tb\"", "\"w1:p3\""),
            ("zw-sid", "\"a\\u200bb\"", "\"w1:p4\""),
            ("zw-pane", "\"s-ok\"", "\"w1:p\\u200b5\""),
        ] {
            // A refused pane cannot be confirmed with herdr: it needs a week.
            let seen = if name == "zw-pane" { week_old } else { stale };
            plant(&mut raw, name, "Node-A.local", session, pane, seen);
        }
        // ...and at a day old it is kept.
        plant(
            &mut raw,
            "zw-pane-day",
            "Node-A.local",
            "\"s-ok\"",
            "\"w1:p\\u200b9\"",
            stale,
        );
        plant(&mut raw, "young", "node-a", "\"\"", "\"w1:p6\"", now_ms());
        plant(&mut raw, "elsewhere", "hub", "\"\"", "\"w1:p7\"", stale);

        let herdr = PruneStubHerdr { answer: None };
        let mut state = WatcherState::default();
        tick(
            &mut relay,
            &herdr,
            &"node-a".parse().unwrap(),
            &mut state,
            &mut |_| {},
        )
        .unwrap();

        let left: Vec<String> = relay
            .agents()
            .unwrap()
            .skipped
            .into_iter()
            .map(|s| s.field)
            .collect();
        assert_eq!(
            left,
            ["elsewhere", "young", "zw-pane-day"],
            "the five stale ones are pruned, a refused pane only after a week"
        );

        // A refused session on a pane herdr still shows with a live session is
        // replaced by it: pruned. With no session shown it proves nothing: kept.
        plant(&mut raw, "replaced", "node-a", "\"\"", "\"w1:p8\"", stale);
        let mut state = WatcherState::default();
        let shown = PruneStubHerdr {
            answer: Some("live-session"),
        };
        tick(
            &mut relay,
            &shown,
            &"node-a".parse().unwrap(),
            &mut state,
            &mut |_| {},
        )
        .unwrap();
        assert!(
            !relay
                .agents()
                .unwrap()
                .skipped
                .iter()
                .any(|s| s.field == "replaced"),
            "a live session in the pane replaces the refused one"
        );
    }

    /// What a scripted herdr shows for every pane.
    enum Shows {
        Session(&'static str),
        AgentWithoutSession,
        Failure,
    }

    struct ScriptedHerdr(Shows);

    impl Herdr for ScriptedHerdr {
        fn get(&self, _pane: &PaneId) -> Result<Option<AgentGet>, ProcessError> {
            match self.0 {
                Shows::Session(s) => Ok(Some(AgentGet {
                    status: AgentStatus::Idle,
                    state_change_seq: 1,
                    session_id: Some(s.parse().unwrap()),
                })),
                Shows::AgentWithoutSession => Ok(Some(AgentGet {
                    status: AgentStatus::Idle,
                    state_change_seq: 1,
                    session_id: None,
                })),
                Shows::Failure => Err(ProcessError::Reported {
                    code: "boom".into(),
                    message: "herdr failed".into(),
                }),
            }
        }
        fn notify(&self, _title: &str, _body: &str) -> Result<(), ProcessError> {
            Ok(())
        }
    }

    impl Deliverer for ScriptedHerdr {
        fn deliver(&self, _pane: &PaneId, _text: &str) -> Result<(), DeliverError> {
            panic!("pruning tests never deliver")
        }
    }

    /// Tracing: TC-008-06
    /// ACs: FR-008-AC-6
    ///
    /// A stale record this binary refuses is kept unless the evidence proves it
    /// stale: queued mail, a pane herdr still shows with the same session, a
    /// refused session on an agent that reports none, a herdr that fails. Each is
    /// planted, and each guard of `prune_refused` fails a case if removed.
    #[test]
    fn refused_records_are_kept_unless_the_evidence_proves_them_stale() {
        let Some(mut relay) = test_relay("prune-refused-kept") else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        let url = std::env::var("IX_RELAY_TEST_URL").expect("set: test_relay returned Some");
        let mut raw = redis::Client::open(url)
            .and_then(|c| c.get_connection())
            .expect("raw connection to test redis");
        let prefix = test_prefix("prune-refused-kept");
        let stale = now_ms() - i64::try_from(PRUNE_AGE.as_millis()).unwrap() - 60_000;
        let week_old =
            now_ms() - i64::try_from(REFUSED_PANE_PRUNE_AGE.as_millis()).unwrap() - 60_000;
        // `cwd` as a number: undecodable for a reason that is not an id, with a
        // valid pane and session.
        let plant = |raw: &mut redis::Connection,
                     name: &str,
                     session: &str,
                     pane: &str,
                     cwd: &str| {
            // The mail record is old enough for its refused pane to prove it stale.
            let seen = if name == "mail" { week_old } else { stale };
            let json = format!(
                r#"{{"name":"{name}","machine":"node-a","kind":"claude","session_id":{session},"cwd":{cwd},"herdr_pane":{pane},"place":null,"last_seen_ms":{seen}}}"#
            );
            redis::cmd("HSET")
                .arg(format!("{prefix}agents"))
                .arg(name)
                .arg(json)
                .exec(raw)
                .expect("plant");
        };
        // Queued mail on a record whose pane is refused (which alone proves stale).
        plant(&mut raw, "mail", "\"s1\"", "\"w1:p\\u200b1\"", "null");
        redis::cmd("RPUSH")
            .arg(format!("{prefix}inbox:mail"))
            .arg(r#"{"from":"a","to":"mail","text":"hi","sent_at_ms":1}"#)
            .exec(&mut raw)
            .expect("queue mail");
        // Same readable session shown in the pane.
        plant(&mut raw, "same", "\"live\"", "\"w1:p2\"", "7");
        // A refused session, on an agent that reports no session.
        plant(&mut raw, "nosession", "\"\"", "\"w1:p3\"", "null");
        let prune = |relay: &mut Relay, shows: Shows| {
            let mut state = WatcherState::default();
            tick(
                relay,
                &ScriptedHerdr(shows),
                &"node-a".parse().unwrap(),
                &mut state,
                &mut |_| {},
            )
            .unwrap();
        };
        let left = |relay: &mut Relay| -> Vec<String> {
            relay
                .agents()
                .unwrap()
                .skipped
                .into_iter()
                .map(|s| s.field)
                .collect()
        };
        let all = ["mail", "nosession", "same"].map(String::from);
        prune(&mut relay, Shows::Failure);
        assert_eq!(left(&mut relay), all, "a herdr that fails proves nothing");
        prune(&mut relay, Shows::AgentWithoutSession);
        assert_eq!(left(&mut relay), all, "no session shown proves nothing");
        // A live session in the pane replaces the refused one, and only that one:
        // the same session and the queued mail still keep theirs.
        prune(&mut relay, Shows::Session("live"));
        assert_eq!(left(&mut relay), ["mail", "same"].map(String::from));
    }

    #[test]
    fn maybe_prune_never_touches_a_registration_with_a_nonempty_inbox() {
        let Some(mut relay) = test_relay("prune-nonempty-inbox") else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        let stale_seen = now_ms() - i64::try_from(PRUNE_AGE.as_millis()).unwrap() - 60_000;
        register_at(
            &mut relay,
            "queued-lead",
            "node-a",
            Some("w1:p1"),
            Some("s1"),
            stale_seen,
        );
        relay
            .send(&Message {
                from: "sender".parse().unwrap(),
                to: Target::Agent("queued-lead".parse().unwrap()),
                text: "still waiting".into(),
                sent_at_ms: 0,
            })
            .unwrap();

        // herdr reports no agent in the pane, but `tick` must never reach
        // `maybe_prune` while the inbox is non-empty (see its own doc
        // comment): the message still needs somewhere to land.
        let herdr = PruneStubHerdr { answer: None };
        let mut state = WatcherState::default();
        tick(
            &mut relay,
            &herdr,
            &"node-a".parse().unwrap(),
            &mut state,
            &mut |_| {},
        )
        .unwrap();

        assert!(
            relay
                .agent(&"queued-lead".parse().unwrap())
                .unwrap()
                .is_some(),
            "never pruned while a message is still queued"
        );
        assert_eq!(
            relay.inbox_len(&"queued-lead".parse().unwrap()).unwrap(),
            1,
            "the queued message is left alone (no pane to deliver into, but not dropped)"
        );
    }

    /// AGE-2026: a live pane whose registration has no session id was pruned,
    /// because "no id" counted as "does not match".
    #[test]
    fn maybe_prune_leaves_a_live_pane_alone_when_the_registration_has_no_session_id() {
        let Some(mut relay) = test_relay("prune-no-session") else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        let stale_seen = now_ms() - i64::try_from(PRUNE_AGE.as_millis()).unwrap() - 60_000;
        register_at(
            &mut relay,
            "no-sid-lead",
            "node-a",
            Some("w1:p1"),
            None,
            stale_seen,
        );

        let herdr = PruneStubHerdr {
            answer: Some("s-live"),
        };
        maybe_prune(
            &mut relay,
            &herdr,
            &"no-sid-lead".parse().unwrap(),
            &"w1:p1".parse().unwrap(),
            None,
            stale_seen,
        )
        .unwrap();
        assert!(
            relay
                .agent(&"no-sid-lead".parse().unwrap())
                .unwrap()
                .is_some(),
            "an absent id is not a mismatch"
        );

        // A different, known session in the pane still proves it replaced.
        register_at(
            &mut relay,
            "replaced-lead",
            "node-a",
            Some("w1:p2"),
            Some("s-old"),
            stale_seen,
        );
        maybe_prune(
            &mut relay,
            &PruneStubHerdr {
                answer: Some("s-new"),
            },
            &"replaced-lead".parse().unwrap(),
            &"w1:p2".parse().unwrap(),
            Some(&"s-old".parse().unwrap()),
            stale_seen,
        )
        .unwrap();
        assert!(
            relay
                .agent(&"replaced-lead".parse().unwrap())
                .unwrap()
                .is_none()
        );
    }

    /// AGE-2026 review: a skipped record is logged once across every read of
    /// the registry, and again if it is fixed and then breaks again.
    #[test]
    fn decodable_reports_each_skipped_record_once_until_it_recovers() {
        let skipped = |field: &str| relay_core::SkippedRegistration {
            field: field.to_owned(),
            error: relay_core::Error::InvalidName {
                name: field.to_owned(),
            },
            raw: Vec::new(),
        };
        let list = |fields: &[&str]| relay_core::AgentList {
            registrations: Vec::new(),
            skipped: fields.iter().map(|f| skipped(f)).collect(),
        };
        let mut reported = HashSet::new();
        decodable(list(&["bad"]), &mut reported);
        assert!(reported.contains("bad"));
        // Still bad: already reported, stays recorded (and is not re-inserted).
        decodable(list(&["bad", "worse"]), &mut reported);
        assert_eq!(reported.len(), 2);
        // Recovered: forgotten, so a later break is a fresh report.
        decodable(list(&["worse"]), &mut reported);
        assert!(!reported.contains("bad") && reported.contains("worse"));
    }

    /// AGE-2026 review: the snapshot said "old and abandoned", but the lead
    /// re-registered before the prune ran; the fresh registration survives.
    #[test]
    fn maybe_prune_spares_a_registration_that_was_refreshed_after_the_snapshot() {
        let Some(mut relay) = test_relay("prune-refreshed") else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        let stale_seen = now_ms() - i64::try_from(PRUNE_AGE.as_millis()).unwrap() - 60_000;
        register_at(
            &mut relay,
            "back-lead",
            "node-a",
            Some("w1:p1"),
            Some("s2"),
            now_ms(),
        );
        maybe_prune(
            &mut relay,
            &PruneStubHerdr { answer: None },
            &"back-lead".parse().unwrap(),
            &"w1:p1".parse().unwrap(),
            Some(&"s1".parse().unwrap()),
            stale_seen,
        )
        .unwrap();
        assert!(
            relay
                .agent(&"back-lead".parse().unwrap())
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn maybe_prune_leaves_a_still_live_old_registration_alone() {
        let Some(mut relay) = test_relay("prune-old-live") else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        let stale_seen = now_ms() - i64::try_from(PRUNE_AGE.as_millis()).unwrap() - 60_000;
        register_at(
            &mut relay,
            "long-lived-lead",
            "node-a",
            Some("w1:p1"),
            Some("s1"),
            stale_seen,
        );

        // herdr confirms the same session is still occupying that pane.
        let herdr = PruneStubHerdr { answer: Some("s1") };
        maybe_prune(
            &mut relay,
            &herdr,
            &"long-lived-lead".parse().unwrap(),
            &"w1:p1".parse().unwrap(),
            Some(&"s1".parse().unwrap()),
            stale_seen,
        )
        .unwrap();

        assert!(
            relay
                .agent(&"long-lived-lead".parse().unwrap())
                .unwrap()
                .is_some(),
            "old but still genuinely live => never pruned"
        );
    }

    // ---- episodes, connection loss (AGE-2033) -------------------------------

    fn lead(name: &str) -> AgentName {
        name.parse().unwrap()
    }

    /// Feeds `outcomes` for one lead through the same observe-then-log path
    /// `tick` uses and returns every line that would be logged.
    fn logged_over(outcomes: &[Outcome]) -> Vec<String> {
        let mut episodes = Episodes::default();
        let name = lead("tl-lead");
        outcomes
            .iter()
            .flat_map(|outcome| {
                let transition = episodes.observe(&name, outcome.episode());
                episode_lines(&name, &"w1:p1".parse().unwrap(), outcome, transition)
            })
            .collect()
    }

    #[test]
    fn a_no_agent_episode_is_logged_at_its_start_and_its_end_only() {
        let lines = logged_over(&[
            Outcome::NoAgent,
            Outcome::NoAgent,
            Outcome::NoAgent,
            Outcome::Delivered { messages: 1 },
        ]);
        assert_eq!(lines.len(), 3, "start, end, then the delivery: {lines:?}");
        assert!(lines[0].contains("has no agent"));
        assert!(lines[1].contains("no agent in its pane ended"));
        assert!(lines[2].contains("delivered"));
    }

    /// Tracing: TC-N003-M-06 (NFR-003-M-6). Evidence: unit test.
    #[test]
    fn a_get_failed_episode_and_a_blocked_episode_are_each_logged_once() {
        let failure = || {
            Outcome::GetFailed(ProcessError::Timeout {
                program: "herdr".into(),
                secs: 10,
            })
        };
        assert_eq!(logged_over(&[failure(), failure(), failure()]).len(), 1);
        assert_eq!(
            logged_over(&[
                Outcome::SkippedBlocked,
                Outcome::SkippedBlocked,
                Outcome::SkippedBlocked
            ])
            .len(),
            1
        );
    }

    #[test]
    fn events_are_logged_every_time() {
        let lines = logged_over(&[Outcome::SkippedWorking, Outcome::SkippedWorking]);
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn a_connection_error_ends_the_tick_and_a_data_error_does_not() {
        use crate::test_support::{connection_error, data_error};
        assert!(per_lead(connection_error(), "ctx").is_err_and(|e| e.is_connection()));
        assert!(per_lead(data_error(), "ctx").is_ok());
    }

    /// A server that accepts the connection and then never answers, as a dead
    /// tunnel does: the first command times out.
    /// Tracing: TC-N003-M-04 (NFR-003-M-4). Evidence: unit test; the row's method is fault-injection.
    #[test]
    fn tick_returns_a_connection_error_when_redis_stops_answering() {
        use std::io::Read;

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("redis://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            // Never reply: hold the connection until the client leaves.
            while stream.read(&mut buf).is_ok_and(|n| n > 0) {}
        });
        let mut relay = Relay::connect(&url, Duration::from_millis(300)).unwrap();
        let mut state = WatcherState::default();
        let err = tick(
            &mut relay,
            &layout_herdr(Default::default(), &[]),
            &"node-a".parse().unwrap(),
            &mut state,
            &mut |_| {},
        )
        .expect_err("a silent server fails the tick");
        assert!(err.is_connection(), "{err}");
        drop(relay);
        server.join().unwrap();
    }

    /// Tracing: TC-044-14
    /// ACs: FR-044-AC-14
    ///
    /// A postponement leaves the episode state alone: held, postponed, held is one held
    /// episode, one notification's worth of `notified`, no "ended" line.
    #[test]
    fn tc_044_14_a_postponement_leaves_a_held_episode_untouched() {
        let name: AgentName = "tl-lead".parse().unwrap();
        let pane: PaneId = "w1:p1".parse().unwrap();
        let mut episodes = episodes::Episodes::default();
        let held = Outcome::SocketHeld {
            why: "bypass".into(),
            notified: true,
        };
        let postponed = Outcome::SocketPostponed { requeued: 1 };
        let mut lines = Vec::new();
        for outcome in [&held, &postponed, &held] {
            let transition = observe_outcome(&mut episodes, &name, outcome);
            lines.extend(episode_lines(&name, &pane, outcome, transition));
        }
        assert!(
            !lines.iter().any(|l| l.contains("ended")),
            "no false end: {lines:?}"
        );
        assert_eq!(
            lines
                .iter()
                .filter(|l| l.contains("holds relay mail"))
                .count(),
            1,
            "the held episode is logged once: {lines:?}"
        );
        assert_eq!(
            episodes.notice(&name, Instant::now()),
            episodes::Notice::Done,
            "the notification already sent stays sent"
        );
    }

    #[test]
    fn tick_forgets_a_lead_that_left_the_registry() {
        let Some(mut relay) = test_relay("episode-hygiene") else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        register(&mut relay, "gone-lead", "node-a", Some("w1:p1"), Some("s1"));
        register(&mut relay, "kept-lead", "node-a", Some("w1:p2"), Some("s2"));
        for name in ["gone-lead", "kept-lead"] {
            relay
                .send(&Message {
                    from: "sender".parse().unwrap(),
                    to: Target::Agent(lead(name)),
                    text: "hi".into(),
                    sent_at_ms: 0,
                })
                .unwrap();
        }
        // No agent in either pane: both leads enter a NoAgent episode.
        let herdr = PruneStubHerdr { answer: None };
        let mut state = WatcherState::default();
        tick(
            &mut relay,
            &herdr,
            &"node-a".parse().unwrap(),
            &mut state,
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(state.episodes.names().count(), 2);

        relay.prune(&lead("gone-lead"), 1).unwrap();
        tick(
            &mut relay,
            &herdr,
            &"node-a".parse().unwrap(),
            &mut state,
            &mut |_| {},
        )
        .unwrap();
        let tracked: Vec<_> = state.episodes.names().cloned().collect();
        assert_eq!(tracked, [lead("kept-lead")]);
    }

    // ---- faults injected mid-tick (NFR-003) ---------------------------------

    fn test_url() -> Option<String> {
        std::env::var("IX_RELAY_TEST_URL").ok()
    }

    /// The `n`th (1-based) command called `name` gets `fault`; all others pass.
    fn nth(name: &'static str, n: usize, fault: Fault) -> impl FnMut(&str) -> Fault + Send {
        let mut seen = 0;
        move |command| {
            if command != name {
                return Fault::Forward;
            }
            seen += 1;
            if seen == n { fault } else { Fault::Forward }
        }
    }

    /// Tracing: TC-N003-M-04 (NFR-003-M-4). Evidence: fault injected through a command proxy in front of Redis (skips without IX_RELAY_TEST_URL); covers the tick ending on a lost connection, so the supervisor reconnects.
    #[test]
    fn a_delivery_is_journalled_even_when_the_connection_dies_right_after_it() {
        let Some(url) = test_url() else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        let mut relay = test_relay("journal-then-lost").unwrap();
        register(&mut relay, "a-lead", "node-a", Some("w1:p1"), Some("s1"));
        register(&mut relay, "b-lead", "node-a", Some("w1:p2"), Some("s1"));
        queue(&mut relay, "a-lead");
        queue(&mut relay, "b-lead");
        // The sweep's reap is the 1st EVAL and a's pop the 2nd; both go through
        // and a is typed into. The connection is reset on a's acknowledgement (the
        // 3rd), the very next thing the tick does: the delivery must still be
        // journalled.
        let proxy = FaultProxy::start(&url, nth("EVAL", 3, Fault::Cut));
        let mut via = relay_via(&proxy, "journal-then-lost");

        let herdr = idle_herdr("s1");
        let (mut state, mut logs) = (WatcherState::default(), Vec::new());
        let err = tick(
            &mut via,
            &herdr,
            &"node-a".parse().unwrap(),
            &mut state,
            &mut |l| logs.push(l),
        )
        .expect_err("the connection loss ends the tick");
        assert!(err.is_connection(), "{err}");
        assert_eq!(
            herdr.prompts.borrow().len(),
            1,
            "a's messages were typed in"
        );
        assert!(
            logs.iter()
                .any(|l| l.contains("delivered 1 message(s) to a-lead")),
            "the delivery has its journal line: {logs:?}"
        );
    }

    /// Tracing: TC-N003-M-04 (NFR-003-M-4). Evidence: fault injected through a command proxy in front of Redis (skips without IX_RELAY_TEST_URL); covers the tick ending on a lost connection, so the supervisor reconnects.
    #[test]
    fn a_connection_lost_mid_tick_leaves_later_leads_untouched() {
        let Some(url) = test_url() else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        let mut relay = test_relay("lost-mid-tick").unwrap();
        for (i, name) in ["a-lead", "b-lead", "c-lead"].into_iter().enumerate() {
            register(
                &mut relay,
                name,
                "node-a",
                Some(&format!("w1:p{i}")),
                Some("s1"),
            );
            queue(&mut relay, name);
        }
        // The reap (the 1st EVAL), a's pop and a's acknowledgement pass; b's pop
        // (the 4th) blackholes.
        let proxy = FaultProxy::start(&url, nth("EVAL", 4, Fault::Silent));
        let mut via = relay_via(&proxy, "lost-mid-tick");

        let herdr = idle_herdr("s1");
        let mut state = WatcherState::default();
        let err = tick(
            &mut via,
            &herdr,
            &"node-a".parse().unwrap(),
            &mut state,
            &mut |_| {},
        )
        .expect_err("a blackholed connection ends the tick");
        assert!(err.is_connection(), "{err}");
        assert_eq!(herdr.prompts.borrow().len(), 1, "only a-lead was reached");
        for (name, left) in [("a-lead", 0), ("b-lead", 1), ("c-lead", 1)] {
            assert_eq!(relay.inbox_len(&lead(name)).unwrap(), left, "{name}");
        }
    }

    /// Tracing: TC-N003-M-04 (NFR-003-M-4). Evidence: fault injected through a command proxy in front of Redis (skips without IX_RELAY_TEST_URL); covers the tick ending on a lost connection, so the supervisor reconnects.
    #[test]
    fn a_connection_lost_refreshing_places_ends_the_tick() {
        let Some(url) = test_url() else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        let mut relay = test_relay("lost-refreshing").unwrap();
        register_at(
            &mut relay,
            "a-lead",
            "node-a",
            Some("wB:p1"),
            Some("s1"),
            now_ms(),
        );
        // The refresh reads the lead's record (HGET); that read is blackholed.
        let proxy = FaultProxy::start(&url, nth("HGET", 1, Fault::Silent));
        let mut via = relay_via(&proxy, "lost-refreshing");
        let herdr = layout_herdr(
            [("wB:p1".parse().unwrap(), place("EA", "code", 1))]
                .into_iter()
                .collect(),
            &[("wB:p1", "s1")],
        );
        let err = tick(
            &mut via,
            &herdr,
            &"node-a".parse().unwrap(),
            &mut WatcherState::default(),
            &mut |_| {},
        )
        .expect_err("a blackholed refresh ends the tick");
        assert!(err.is_connection(), "{err}");
        assert_eq!(
            proxy.seen.lock().unwrap().last().map(String::as_str),
            Some("HGET"),
            "the tick stopped at the failed refresh instead of carrying on"
        );
    }

    /// Tracing: TC-N003-M-04 (NFR-003-M-4). Evidence: fault injected through a command proxy in front of Redis (skips without IX_RELAY_TEST_URL); covers the tick ending on a lost connection, so the supervisor reconnects.
    #[test]
    fn a_connection_lost_pruning_ends_the_tick() {
        let Some(url) = test_url() else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        let mut relay = test_relay("lost-pruning").unwrap();
        let stale_seen = now_ms() - i64::try_from(PRUNE_AGE.as_millis()).unwrap() - 60_000;
        register_at(
            &mut relay,
            "a-old",
            "node-a",
            Some("w1:p1"),
            Some("s1"),
            stale_seen,
        );
        // The prune is the first script the tick runs; it is reset.
        let proxy = FaultProxy::start(&url, |command| match command {
            "EVAL" | "EVALSHA" => Fault::Cut,
            _ => Fault::Forward,
        });
        let mut via = relay_via(&proxy, "lost-pruning");
        let err = tick(
            &mut via,
            &PruneStubHerdr { answer: None },
            &"node-a".parse().unwrap(),
            &mut WatcherState::default(),
            &mut |_| {},
        )
        .expect_err("a reset while pruning ends the tick");
        assert!(err.is_connection(), "{err}");
    }

    /// NFR-003-M-1: one lead's inbox holds the wrong type of value, which is a
    /// data error. It is logged, and every other lead is still delivered to on
    /// the same connection.
    /// Tracing: TC-N003-M-01 (NFR-003-M-1). Evidence: Redis-backed test with a wrong-typed inbox key; the row's method is fault-injection with a herdr stub.
    #[test]
    fn a_data_error_on_one_lead_does_not_stop_the_others() {
        let Some(url) = test_url() else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        let mut relay = test_relay("data-error-one-lead").unwrap();
        for (i, name) in ["a-lead", "b-lead", "c-lead"].into_iter().enumerate() {
            register(
                &mut relay,
                name,
                "node-a",
                Some(&format!("w1:p{i}")),
                Some("s1"),
            );
            queue(&mut relay, name);
        }
        let key = format!("{}inbox:b-lead", test_prefix("data-error-one-lead"));
        raw_command(&url, &["DEL", &key]);
        assert_eq!(raw_command(&url, &["SET", &key, "not a list"]), "+OK");

        let herdr = idle_herdr("s1");
        let (mut state, mut logs) = (WatcherState::default(), Vec::new());
        tick(
            &mut relay,
            &herdr,
            &"node-a".parse().unwrap(),
            &mut state,
            &mut |l| logs.push(l),
        )
        .expect("a data error on one lead does not fail the tick");
        assert_eq!(herdr.prompts.borrow().len(), 2, "a-lead and c-lead");
        assert_eq!(relay.inbox_len(&lead("a-lead")).unwrap(), 0);
        assert_eq!(relay.inbox_len(&lead("c-lead")).unwrap(), 0);
        assert!(
            state
                .retry_at
                .get(&lead("b-lead"))
                .is_some_and(|at| *at > Instant::now()),
            "the unreadable lead is retried later, not spun on"
        );
    }

    // ---- blocked-episode notifications (NFR-003-M-6) -------------------------

    /// A herdr whose `get` answers come from a script and whose notifications
    /// are counted (and optionally refused).
    struct BlockedScript {
        gets:
            std::cell::RefCell<std::collections::VecDeque<Result<Option<AgentGet>, ProcessError>>>,
        notifies: std::cell::Cell<usize>,
        notify_works: bool,
    }

    fn blocked() -> Result<Option<AgentGet>, ProcessError> {
        Ok(Some(AgentGet {
            status: AgentStatus::Blocked,
            state_change_seq: 1,
            session_id: Some("s1".parse().unwrap()),
        }))
    }

    impl Herdr for BlockedScript {
        fn get(&self, _pane: &PaneId) -> Result<Option<AgentGet>, ProcessError> {
            self.gets.borrow_mut().pop_front().expect("scripted get")
        }
        fn notify(&self, _title: &str, _body: &str) -> Result<(), ProcessError> {
            self.notifies.set(self.notifies.get() + 1);
            if self.notify_works {
                Ok(())
            } else {
                Err(ProcessError::Timeout {
                    program: "herdr".into(),
                    secs: 10,
                })
            }
        }
    }

    impl Deliverer for BlockedScript {
        fn deliver(&self, _pane: &PaneId, _text: &str) -> Result<(), DeliverError> {
            panic!("must not prompt a blocked lead")
        }
    }

    fn blocked_lead(test: &str) -> Option<Relay> {
        let mut relay = test_relay(test)?;
        register(&mut relay, "tl-lead", "node-a", Some("w1:p1"), Some("s1"));
        queue(&mut relay, "tl-lead");
        Some(relay)
    }

    /// NFR-003-M-6: a `herdr agent get` failure in the middle of a blocked
    /// stretch does not make the next blocked tick a new episode.
    /// Tracing: TC-N003-M-06 (NFR-003-M-6). Evidence: unit test.
    #[test]
    fn a_get_failure_blip_mid_blocked_episode_notifies_operator_once() {
        let Some(mut relay) = blocked_lead("blocked-blip") else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        let herdr = BlockedScript {
            gets: std::cell::RefCell::new(
                [
                    blocked(),
                    Err(ProcessError::Timeout {
                        program: "herdr".into(),
                        secs: 10,
                    }),
                    blocked(),
                ]
                .into(),
            ),
            notifies: std::cell::Cell::default(),
            notify_works: true,
        };
        let mut state = WatcherState::default();
        for _ in 0..3 {
            tick(
                &mut relay,
                &herdr,
                &"node-a".parse().unwrap(),
                &mut state,
                &mut |_| {},
            )
            .unwrap();
        }
        assert_eq!(herdr.notifies.get(), 1);
    }

    /// Tracing: TC-N003-M-06 (NFR-003-M-6). Evidence: unit test.
    #[test]
    fn a_refused_notification_is_not_retried_every_tick() {
        let Some(mut relay) = blocked_lead("notify-retry") else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        let herdr = BlockedScript {
            gets: std::cell::RefCell::new(std::iter::repeat_n(blocked(), 3).collect()),
            notifies: std::cell::Cell::default(),
            notify_works: false,
        };
        let (mut state, mut logs) = (WatcherState::default(), Vec::new());
        for _ in 0..3 {
            tick(
                &mut relay,
                &herdr,
                &"node-a".parse().unwrap(),
                &mut state,
                &mut |l| logs.push(l),
            )
            .unwrap();
        }
        assert_eq!(
            herdr.notifies.get(),
            1,
            "one attempt, then wait for the retry interval"
        );
        assert_eq!(
            logs.iter()
                .filter(|l| l.contains("notification failed"))
                .count(),
            1,
            "the attempt is logged, the waiting ticks are silent: {logs:?}"
        );
    }

    // ---- wake-up, timers and round trips (AGE-2028) -------------------------

    /// A herdr that reports every pane as `status`, with the session it was
    /// asked to, and counts `get` calls and deliveries.
    struct CountingHerdr {
        status: AgentStatus,
        session_id: &'static str,
        gets: std::cell::Cell<usize>,
        delivered: std::cell::RefCell<Vec<(String, String)>>,
    }

    impl CountingHerdr {
        fn new(status: AgentStatus, session_id: &'static str) -> Self {
            Self {
                status,
                session_id,
                gets: std::cell::Cell::default(),
                delivered: std::cell::RefCell::default(),
            }
        }
    }

    impl Herdr for CountingHerdr {
        fn get(&self, _pane: &PaneId) -> Result<Option<AgentGet>, ProcessError> {
            self.gets.set(self.gets.get() + 1);
            Ok(Some(AgentGet {
                status: self.status,
                state_change_seq: 1,
                session_id: Some(self.session_id.parse().unwrap()),
            }))
        }

        fn notify(&self, _title: &str, _body: &str) -> Result<(), ProcessError> {
            Ok(())
        }
    }

    impl Deliverer for CountingHerdr {
        fn deliver(&self, pane: &PaneId, text: &str) -> Result<(), DeliverError> {
            self.delivered
                .borrow_mut()
                .push((pane.to_string(), text.to_owned()));
            Ok(())
        }
    }

    /// A bus for `test`'s keyspace with `herdr`, on machine `node-a`, whose
    /// retry timers run every `retry_every`.
    fn bus_for<H: Herdr + Deliverer>(test: &str, herdr: H, retry_every: Duration) -> RedisBus<H> {
        RedisBus {
            url: std::env::var("IX_RELAY_TEST_URL").expect("callers checked IX_RELAY_TEST_URL"),
            prefix: test_prefix(test),
            machine: "node-a".parse().unwrap(),
            herdr,
            state: WatcherState::new(retry_every, Duration::from_secs(DEFAULT_SWEEP_SECS)),
        }
    }

    /// Tracing: TC-038-03
    /// ACs: FR-038-AC-3
    ///
    /// An idle lead is delivered to when the message is *sent*, not when a timer
    /// fires: with the sweep 15 s away and no retry timer, the wait returns for
    /// the send, and the tick it wakes delivers.
    #[test]
    fn a_send_wakes_the_watcher_and_the_message_is_delivered_without_a_timer() {
        let Some(mut relay) = test_relay("wake-on-send") else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        register(&mut relay, "tl-lead", "node-a", Some("w1:p1"), Some("s1"));
        let mut bus = bus_for(
            "wake-on-send",
            CountingHerdr::new(AgentStatus::Idle, "s1"),
            Duration::from_secs(2),
        );
        let mut conn = bus.connect().unwrap();
        bus.poll(&mut conn, Wake::Sweep).unwrap();
        assert_eq!(bus.state.due(Instant::now()), None, "no timer is due");
        assert!(
            bus.state.until_due(Instant::now()) > Duration::from_secs(10),
            "the sweep is far off"
        );

        let sender = std::thread::spawn({
            let url = std::env::var("IX_RELAY_TEST_URL").unwrap();
            let prefix = test_prefix("wake-on-send");
            move || {
                std::thread::sleep(Duration::from_millis(50));
                let mut other = Relay::connect(&url, Duration::from_secs(2))
                    .unwrap()
                    .with_prefix(prefix);
                queue(&mut other, "tl-lead");
            }
        });
        assert_eq!(bus.wait(&mut conn).unwrap(), Wake::Message);
        sender.join().unwrap();
        bus.poll(&mut conn, Wake::Message).unwrap();
        assert_eq!(
            bus.herdr.delivered.borrow().len(),
            1,
            "delivered on the send"
        );
        assert_eq!(relay.inbox_len(&lead("tl-lead")).unwrap(), 0);
    }

    /// Tracing: TC-038-04
    /// ACs: FR-038-AC-4
    ///
    /// A lead left for its hook has mail and nothing new coming, so it has a
    /// retry timer: a retry tick before the timer is due does not spawn herdr
    /// for it, one after does, and a lead whose inbox drained loses its timer.
    #[test]
    fn a_lead_left_for_its_hook_is_retried_on_its_own_timer_and_only_then() {
        let Some(mut relay) = test_relay("retry-timer") else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        register(&mut relay, "tl-lead", "node-a", Some("w1:p1"), Some("s1"));
        queue(&mut relay, "tl-lead");
        let mut bus = bus_for(
            "retry-timer",
            CountingHerdr::new(AgentStatus::Working, "s1"),
            Duration::from_secs(2),
        );
        let mut conn = bus.connect().unwrap();
        bus.poll(&mut conn, Wake::Sweep).unwrap();
        assert_eq!(bus.herdr.gets.get(), 1, "the sweep looked at the lead");
        assert!(bus.state.retry_at.contains_key(&lead("tl-lead")));
        assert_eq!(
            bus.state.due(Instant::now()),
            None,
            "the timer is not due yet"
        );

        bus.poll(&mut conn, Wake::Retry).unwrap();
        assert_eq!(
            bus.herdr.gets.get(),
            1,
            "a retry before the timer is due is free"
        );

        bus.state
            .retry_at
            .insert(lead("tl-lead"), Instant::now() - Duration::from_secs(1));
        assert_eq!(bus.state.due(Instant::now()), Some(Wake::Retry));
        bus.poll(&mut conn, Wake::Retry).unwrap();
        assert_eq!(bus.herdr.gets.get(), 2, "the due retry looked again");

        // The hook delivers meanwhile: the empty inbox drops the timer.
        relay.pop_inbox(&lead("tl-lead"), BATCH).unwrap();
        bus.poll(&mut conn, Wake::Sweep).unwrap();
        assert!(bus.state.retry_at.is_empty());
    }

    /// Tracing: TC-038-05
    /// ACs: FR-038-AC-5
    ///
    /// A tick with every inbox empty costs two requests however many leads are
    /// local, and no herdr call: one registry read, one pipelined length check.
    #[test]
    fn an_idle_tick_is_two_requests_and_no_herdr_call() {
        let Some(url) = test_url() else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        let mut relay = test_relay("idle-tick").unwrap();
        for (i, name) in ["a-lead", "b-lead", "c-lead"].into_iter().enumerate() {
            register_at(
                &mut relay,
                name,
                "node-a",
                Some(&format!("w1:p{i}")),
                Some("s1"),
                now_ms(),
            );
        }
        let proxy = FaultProxy::start(&url, |_| Fault::Forward);
        let mut via = relay_via(&proxy, "idle-tick");
        let herdr = CountingHerdr::new(AgentStatus::Idle, "s1");
        tick(
            &mut via,
            &herdr,
            &"node-a".parse().unwrap(),
            &mut WatcherState::default(),
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(
            *proxy.requests.lock().unwrap(),
            [
                vec!["HGETALL"],
                // A sweep (a tick's default wake) returns every lead's expired
                // in-flight batches first (one EVAL), in the same request as the
                // lengths.
                vec!["EVAL", "LLEN", "LLEN", "LLEN"]
            ],
        );
        assert_eq!(herdr.gets.get(), 0);
    }

    /// Tracing: TC-045-22
    /// ACs: FR-045-AC-22, FR-038-AC-5
    ///
    /// With OTLP on, the tick that pushes adds exactly the two pipelined requests
    /// FR-045 pins (the local leads' queues, the newest hook failures) and no key
    /// scan; the next tick, inside the interval, is the two requests of an idle
    /// tick again. The tick with OTLP off stays at two (the test above).
    #[test]
    fn tc_045_22_a_push_tick_adds_two_pipelined_requests_and_the_next_tick_none() {
        let Some(url) = test_url() else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        let mut relay = test_relay("push-tick").unwrap();
        for (i, name) in ["a-lead", "b-lead", "c-lead"].into_iter().enumerate() {
            register_at(
                &mut relay,
                name,
                "node-a",
                Some(&format!("w1:p{i}")),
                Some("s1"),
                now_ms(),
            );
        }
        let proxy = FaultProxy::start(&url, |_| Fault::Forward);
        let mut via = relay_via(&proxy, "push-tick");
        let herdr = CountingHerdr::new(AgentStatus::Idle, "s1");
        let node_a: MachineName = "node-a".parse().unwrap();
        let (publisher, bodies) = recording_publisher(&node_a, Duration::from_secs(30));
        let mut state = WatcherState {
            otlp: publisher,
            ..WatcherState::default()
        };
        tick(&mut via, &herdr, &node_a, &mut state, &mut |_| {}).unwrap();
        let first = proxy.requests.lock().unwrap().clone();
        assert_eq!(
            first,
            [
                vec!["HGETALL"],
                vec!["EVAL", "LLEN", "LLEN", "LLEN"],
                // One pipeline over the three local leads: inbox length, oldest
                // message, in-flight length and dead-letter length each.
                ["LLEN", "LINDEX", "LLEN", "LLEN"].repeat(3),
                vec!["XREVRANGE"],
            ],
            "2 + otlp::PUSH_REQUESTS requests, and no SCAN"
        );
        assert_eq!(first.len(), 2 + otlp::PUSH_REQUESTS);
        assert!(
            bodies.recv().is_ok(),
            "and the push was handed to the exporter"
        );
        proxy.requests.lock().unwrap().clear();
        tick(&mut via, &herdr, &node_a, &mut state, &mut |_| {}).unwrap();
        assert_eq!(
            proxy.requests.lock().unwrap().len(),
            2,
            "inside the interval: no push"
        );
    }

    /// Tracing: TC-045-22
    /// ACs: FR-045-AC-22
    ///
    /// A failing store read for the push never skips the rest of the tick: the
    /// registry cache is updated, the tick returns Ok, and the failure is logged once.
    #[test]
    fn tc_045_22_a_failing_push_read_is_logged_once_and_never_skips_the_tick() {
        let Some(url) = test_url() else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        let mut relay = test_relay("push-fault").unwrap();
        register_at(
            &mut relay,
            "a-lead",
            "node-a",
            Some("w1:p0"),
            Some("s1"),
            now_ms(),
        );
        // The push's own pipeline is the first LINDEX: cut it and everything after.
        let mut cut = false;
        let proxy = FaultProxy::start(&url, move |name| {
            cut |= name == "LINDEX";
            if cut { Fault::Cut } else { Fault::Forward }
        });
        let mut via = relay_via(&proxy, "push-fault");
        let herdr = CountingHerdr::new(AgentStatus::Idle, "s1");
        let node_a: MachineName = "node-a".parse().unwrap();
        let (publisher, bodies) = recording_publisher(&node_a, Duration::ZERO);
        let mut state = WatcherState {
            otlp: publisher,
            ..WatcherState::default()
        };
        let mut lines = Vec::new();
        tick(&mut via, &herdr, &node_a, &mut state, &mut |l| {
            lines.push(l)
        })
        .unwrap();
        assert!(
            state.registry.is_some(),
            "the registry cache was still updated"
        );
        assert_eq!(
            lines.iter().filter(|l| l.contains("metrics push")).count(),
            1,
            "{lines:?}"
        );
        // The push still goes out (the registration count needs no store read), but
        // with no per-lead point: a failed read must not read as a drained backlog.
        let body = String::from_utf8(bodies.recv().unwrap()).unwrap();
        assert!(body.contains("ix.relay.registrations"), "{body}");
        for name in [
            "ix.relay.inbox.depth",
            "ix.relay.inbox.oldest_age",
            "ix.relay.inflight.entries",
            "ix.relay.deadletter.entries",
            "a-lead",
        ] {
            assert!(
                !body.contains(name),
                "{name} was sent after a failed read: {body}"
            );
        }
    }

    /// Tracing: TC-038-06
    /// ACs: FR-038-AC-6
    ///
    /// A delivery reuses the registry the tick already read for sender
    /// addresses: one `HGETALL` for the whole tick, however many leads get mail.
    #[test]
    fn a_delivering_tick_reads_the_registry_once() {
        let Some(url) = test_url() else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        let mut relay = test_relay("registry-once").unwrap();
        for (i, name) in ["a-lead", "b-lead"].into_iter().enumerate() {
            register(
                &mut relay,
                name,
                "node-a",
                Some(&format!("w1:p{i}")),
                Some("s1"),
            );
            queue(&mut relay, name);
        }
        let proxy = FaultProxy::start(&url, |_| Fault::Forward);
        let mut via = relay_via(&proxy, "registry-once");
        let herdr = CountingHerdr::new(AgentStatus::Idle, "s1");
        tick(
            &mut via,
            &herdr,
            &"node-a".parse().unwrap(),
            &mut WatcherState::default(),
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(herdr.delivered.borrow().len(), 2, "both were delivered");
        let hgetall = proxy
            .seen
            .lock()
            .unwrap()
            .iter()
            .filter(|c| *c == "HGETALL")
            .count();
        assert_eq!(hgetall, 1, "{:?}", proxy.requests.lock().unwrap());
    }

    /// Tracing: TC-038-08
    /// ACs: FR-038-AC-8
    ///
    /// A lead idle for over a day whose session is still live is checked with
    /// herdr once, not on every tick, and again once the recheck interval passes.
    #[test]
    fn an_old_live_registration_is_not_checked_with_herdr_every_tick() {
        let Some(mut relay) = test_relay("prune-throttle") else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        // Last seen at the epoch: long past PRUNE_AGE, empty inbox, and herdr
        // says the registered session still sits in the pane.
        register(&mut relay, "old-lead", "node-a", Some("w1:p1"), Some("s1"));
        let herdr = CountingHerdr::new(AgentStatus::Idle, "s1");
        let mut state = WatcherState::default();
        for _ in 0..3 {
            tick(
                &mut relay,
                &herdr,
                &"node-a".parse().unwrap(),
                &mut state,
                &mut |_| {},
            )
            .unwrap();
        }
        assert_eq!(herdr.gets.get(), 1, "three ticks, one herdr call");
        // That one look also moved the stamp forward (FR-042), so the lead is no
        // longer a pruning candidate at all.
        let refreshed = relay.agent(&lead("old-lead")).unwrap().unwrap();
        assert!(!old_enough_to_prune(refreshed.last_seen_ms));

        // Aged again, and past the recheck interval: one more look, not a
        // prune, because the session is live.
        register(&mut relay, "old-lead", "node-a", Some("w1:p1"), Some("s1"));
        state
            .live
            .touch_checked
            .insert(lead("old-lead"), Instant::now() - last_seen::CHECK_EVERY);
        tick(
            &mut relay,
            &herdr,
            &"node-a".parse().unwrap(),
            &mut state,
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(herdr.gets.get(), 2, "the recheck interval passed");
        assert!(
            relay.agent(&lead("old-lead")).unwrap().is_some(),
            "a live session is never pruned"
        );
    }

    /// Tracing: TC-038-11
    /// ACs: FR-038-AC-11
    ///
    /// A burst of sends to leads on other machines does not amplify into work
    /// here: a local lead left for its hook with mail queued costs no herdr call
    /// per send, and the registry is read once for the burst, not once per send.
    #[test]
    fn a_burst_of_other_machines_sends_costs_no_herdr_calls_and_one_registry_read() {
        let Some(url) = test_url() else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        let mut relay = test_relay("burst").unwrap();
        register(&mut relay, "tl-lead", "node-a", Some("w1:p1"), Some("s1"));
        register(
            &mut relay,
            "far-lead",
            "elsewhere",
            Some("w1:p1"),
            Some("s9"),
        );
        queue(&mut relay, "tl-lead");
        let proxy = FaultProxy::start(&url, |_| Fault::Forward);
        let mut bus = bus_for(
            "burst",
            CountingHerdr::new(AgentStatus::Working, "s1"),
            Duration::from_secs(2),
        );
        bus.url.clone_from(&proxy.url);
        let mut conn = bus.connect().unwrap();
        bus.poll(&mut conn, Wake::Sweep).unwrap();
        assert_eq!(bus.herdr.gets.get(), 1, "the sweep looked at the lead once");
        for _ in 0..20 {
            queue(&mut relay, "far-lead");
            assert_eq!(bus.wait(&mut conn).unwrap(), Wake::Message);
            bus.poll(&mut conn, Wake::Message).unwrap();
        }
        assert_eq!(
            bus.herdr.gets.get(),
            1,
            "no herdr call for other machines' sends"
        );
        let registry_reads = proxy
            .seen
            .lock()
            .unwrap()
            .iter()
            .filter(|c| *c == "HGETALL")
            .count();
        assert_eq!(
            registry_reads, 1,
            "one read of the registry for the whole burst"
        );
        // Mail for the local lead itself does look at it, once per gap.
        queue(&mut relay, "tl-lead");
        assert_eq!(bus.wait(&mut conn).unwrap(), Wake::Message);
        bus.poll(&mut conn, Wake::Message).unwrap();
        assert_eq!(bus.herdr.gets.get(), 1, "but not inside the minimum gap");
    }

    /// Tracing: TC-038-12
    /// ACs: FR-038-AC-12
    ///
    /// A message log the watcher cannot follow (a key of the wrong type) costs it
    /// the wake-ups, not its deliveries: the connect succeeds, the log is marked
    /// unfollowable, and a sweep still delivers.
    #[test]
    fn an_unfollowable_log_degrades_to_the_timers_instead_of_failing_every_connect() {
        let Some(url) = test_url() else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        let mut relay = test_relay("bad-log").unwrap();
        register(&mut relay, "tl-lead", "node-a", Some("w1:p1"), Some("s1"));
        queue(&mut relay, "tl-lead");
        let key = format!("{}log", test_prefix("bad-log"));
        raw_command(&url, &["DEL", &key]);
        assert_eq!(raw_command(&url, &["SET", &key, "not a stream"]), "+OK");
        let mut bus = bus_for(
            "bad-log",
            CountingHerdr::new(AgentStatus::Idle, "s1"),
            Duration::from_secs(2),
        );
        let mut conn = bus.connect().expect("a bad log does not fail the connect");
        assert!(bus.state.log_id.is_none() && bus.state.degraded_logged);
        bus.poll(&mut conn, Wake::Sweep).unwrap();
        assert_eq!(bus.herdr.delivered.borrow().len(), 1, "the sweep delivers");
    }

    /// Tracing: TC-038-04
    /// ACs: FR-038-AC-4
    ///
    /// The timers, without Redis: the sweep outranks a retry, a retry is due
    /// only once its time comes, and the wait is as long as the nearest one.
    #[test]
    fn the_timers_pick_the_wake_and_bound_the_wait() {
        let mut state = WatcherState::new(Duration::from_secs(2), Duration::from_secs(15));
        let now = Instant::now();
        state.next_sweep = now + Duration::from_secs(15);
        assert_eq!(state.due(now), None);
        assert_eq!(state.until_due(now), Duration::from_secs(15));

        state
            .retry_at
            .insert(lead("a-lead"), now + Duration::from_secs(2));
        assert_eq!(state.due(now), None);
        assert_eq!(state.until_due(now), Duration::from_secs(2));
        assert_eq!(state.due(now + Duration::from_secs(2)), Some(Wake::Retry));

        state.next_sweep = now;
        assert_eq!(
            state.due(now),
            Some(Wake::Sweep),
            "the sweep outranks a retry"
        );
        assert_eq!(
            state.until_due(now + Duration::from_secs(1)),
            Duration::ZERO
        );

        // Only a lead whose timer is due is looked at on a retry tick; the sweep
        // looks at everyone; a message wake looks at a lead whose inbox grew.
        state.wake = Wake::Retry;
        assert!(!state.examines(&lead("a-lead"), 1, now));
        assert!(state.examines(&lead("a-lead"), 1, now + Duration::from_secs(2)));
        assert!(
            state.examines(&lead("never-seen"), 1, now),
            "no timer yet: look"
        );
        state.wake = Wake::Sweep;
        assert!(state.examines(&lead("a-lead"), 1, now));
        state.wake = Wake::Message;
        assert!(
            !state.examines(&lead("a-lead"), 1, now),
            "same length, timer set: another machine's send is not this lead's business"
        );
        assert!(state.examines(&lead("a-lead"), 2, now), "it grew");
        state.last_examined.insert(lead("a-lead"), now);
        assert!(
            !state.examines(&lead("a-lead"), 3, now + Duration::from_millis(200)),
            "but at most once per gap"
        );
    }
}
