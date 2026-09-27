//! Turns [`Facts`] into [`Check`]s. Pure: no clock, no I/O, so every code has a
//! test that builds the facts it fires on.
//!
//! # Severity
//!
//! Exit 1 has to mean something, so a check is a `fail` only when something needs
//! attention now, and a `warn` (exit 0, `--strict` makes it fail) when it is
//! expected to clear, is not urgent, or is not known to matter:
//!
//! - a missing watcher heartbeat fails when that machine has a lead seen in the
//!   last hour, and warns otherwise; a local watcher that runs without
//!   publishing one (older than the heartbeat) warns;
//! - hook failures and a non-empty spool fail while the newest is under 15
//!   minutes old, and warn after that (they are counted for 24 hours);
//! - a registration whose pane herdr no longer has warns, and only once its stamp
//!   is an hour old (the watcher prunes it after a day);
//! - dead letters warn (`agent-msg inbox --dead` reads them; nothing clears
//!   them); overdue in-flight leases warn;
//! - a stuck inbox fails when its lead was seen in the last hour, else warns;
//! - a herdr whose shape ix-relay cannot read fails (the watcher types into no
//!   pane), an old version with a good shape warns.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use relay_core::ScanCut;
use relay_core::moves::{MoveEvent, MoveKind};
use relay_core::{
    LinkFault, MachineName, Registration,
    health::{
        HeartbeatRead, MachineHeartbeat, OtlpStatus, WatcherHeartbeat, heartbeat_ttl, tick_is_stale,
    },
};
use relay_ops::herdr_check::HerdrCheck;

use super::{
    facts::{
        BusFailure, Facts, FileState, RedisFacts, RedisUp, ServiceState, SocketFacts, SocketState,
        SpoolState,
    },
    report::{Check, Code, Family, wire_name},
};
use crate::spool::{SpoolRecord, Stage};

const HOUR_MS: i64 = 60 * 60 * 1000;

/// A registration unseen this long is one nothing has pruned; also how recently a
/// lead must have been seen for its machine to be expected to run a watcher.
const DAY_MS: i64 = 24 * HOUR_MS;

/// A lead seen this recently is one whose machine's watcher matters now.
const RECENT_LEAD_MS: i64 = HOUR_MS;

/// A hook failure this recent is one that may still be happening.
const RECENT_FAILURE_MS: i64 = 15 * 60 * 1000;

/// How long a pane must have been gone (its registration unrefreshed) before it
/// is reported: a session closed a minute ago is not a finding.
const PANE_GONE_GRACE_MS: i64 = HOUR_MS;

/// How far past its deadline a lease must be before it is reported: the watcher
/// reaps on its sweep.
const LEASE_OVERDUE_GRACE_MS: i64 = 2 * 60 * 1000;

/// Redis slower than this per connect and `PING` is failing the hooks' 300 ms
/// budget many times over.
const SLOW_REDIS_MS: u128 = 1000;

/// A message waiting longer than this was not delivered by a watcher tick or a
/// hook: the watcher wakes on a send and sweeps every 15 s.
const INBOX_STUCK_MS: i64 = 5 * 60 * 1000;

/// The least `watcher.json`'s last good poll may age before it is stale.
const FILE_STALE_MIN_MS: i64 = 120_000;

/// The most preview characters of one lost message.
const PREVIEW_CHARS: usize = 100;

/// `ms` as `3s`, `4m`, `2h` or `3d`.
fn ago(ms: i64) -> String {
    let secs = ms.max(0) / 1000;
    match secs {
        0..=99 => format!("{secs}s"),
        100..=5999 => format!("{}m", secs / 60),
        6000..=172_799 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86_400),
    }
}

/// Every check, in the order they are reported.
pub(crate) fn evaluate(f: &Facts) -> Vec<Check> {
    let mut checks = Vec::new();
    if let Some(error) = &f.config_error {
        checks.push(Check::fail(
            Family::Config,
            Code::ConfigInvalid,
            None,
            format!("config.toml did not load: {error}"),
            "the file has a syntax error or an unknown key",
            "fix or remove ~/.ix/relay/config.toml (or the file IX_RELAY_CONFIG names)",
        ));
    }
    if let Some(problem) = &f.watcher_config_problem {
        checks.push(Check::warn(
            Family::Config,
            Code::WatcherOtlpConfigInvalid,
            None,
            format!("the watcher will refuse config.toml's OTLP settings: {problem}"),
            "a watcher-only key holds a value the watcher cannot use; agent-msg and the hook ignore it",
            "fix [watcher] otlp_endpoint / otlp_headers / otlp_interval_secs, then restart the watcher",
        ));
    }
    let up = match &f.redis {
        RedisFacts::Up(up) => Some(up.as_ref()),
        RedisFacts::Down(..) | RedisFacts::NotAttempted => None,
    };
    redis(f, up, &mut checks);
    if let Some(up) = up {
        heartbeats(f, up, &mut checks);
    }
    watcher_file(f, up, &mut checks);
    spool(f, &mut checks);
    if let Some(up) = up {
        bus_failures(f, up, &mut checks);
        recent_moves(f, up, &mut checks);
    }
    herdr(f, &mut checks);
    if let Some(up) = up {
        registrations(f, up, &mut checks);
        backlog(up, &mut checks);
        inboxes(f, up, &mut checks);
    }
    service(f, up, &mut checks);
    socket(f, &mut checks);
    checks
}

/// Delivery through Claude Code's inbox sockets (FR-044). Reports the mode, the
/// sockets each watcher resolved and the leads held; never a failure, because the
/// socket is opt-in and herdr is behind it.
fn socket(f: &Facts, checks: &mut Vec<Check>) {
    let states = match &f.socket {
        SocketFacts::Herdr => {
            checks.push(Check::ok(
                Family::Socket,
                None,
                "delivery = herdr (the default): Claude Code inbox sockets are not used",
            ));
            return;
        }
        SocketFacts::ConfigFailed(why) => {
            checks.push(Check::unknown(
                Family::Socket,
                Code::SocketStatusUnknown,
                None,
                format!("the delivery mode is not known because config.toml did not load: {why}"),
                "fix ~/.ix/relay/config.toml; the watcher does not start on a bad config either",
            ));
            return;
        }
        SocketFacts::Socket(states) => states,
    };
    // A status left by a watcher that is gone says nothing while another lives.
    let any_live = states.iter().any(|s| {
        matches!(
            s,
            SocketState::Read {
                pid_alive: Some(true) | None,
                ..
            }
        )
    });
    for state in states {
        match state {
            SocketState::Read {
                pid_alive: Some(false),
                ..
            } if any_live => {}
            other => socket_state(other, checks),
        }
    }
}

/// The check for one watcher's status.
fn socket_state(state: &SocketState, checks: &mut Vec<Check>) {
    let fix = "start relay-watcher; it writes its report after each socket delivery";
    let unknown = |why: String| {
        Check::unknown(
            Family::Socket,
            Code::SocketStatusUnknown,
            None,
            format!("delivery = socket, but {why}"),
            fix,
        )
    };
    let (status, pid_alive) = match state {
        SocketState::NoPath => {
            checks.push(unknown(
                "there is no config location to find its report".into(),
            ));
            return;
        }
        SocketState::Missing => {
            checks.push(unknown(
                "the watcher has not reported (no socket-<pid>.json)".into(),
            ));
            return;
        }
        SocketState::Unreadable(e) => {
            checks.push(unknown(format!("its report cannot be read: {e}")));
            return;
        }
        SocketState::Untrusted(e) => {
            checks.push(unknown(format!(
                "socket status directory untrusted: held batches will not survive a restart ({e})"
            )));
            return;
        }
        SocketState::Read { status, pid_alive } => (status, *pid_alive),
    };
    if pid_alive == Some(false) {
        checks.push(unknown(format!(
            "the watcher that reported (pid {}) is not running",
            status.pid
        )));
        return;
    }
    let mut details: Vec<String> = status
        .leads
        .iter()
        .map(|l| {
            format!(
                "{}: pid {} at {} (Claude Code {})",
                l.lead, l.pid, l.socket, l.version
            )
        })
        .collect();
    details.push(
        "the watcher's reply socket is removed when it exits on SIGTERM or SIGINT, and at the \
         next start after a SIGKILL"
            .to_owned(),
    );
    let subject = format!("watcher pid {}", status.pid);
    let mut versions: Vec<&str> = status.leads.iter().map(|l| l.version.as_str()).collect();
    versions.sort_unstable();
    versions.dedup();
    let unverified: Vec<String> = versions
        .into_iter()
        .filter(|v| *v != claude_wire::CAPTURED_ON)
        .map(|v| {
            format!(
                "wire format captured on {}; running {v}: unverified",
                claude_wire::CAPTURED_ON
            )
        })
        .collect();
    details.push(format!(
        "wire format captured on Claude Code {} (claude-wire); the version gate is >= {}.{}.{}, \
         peerProtocol {}",
        claude_wire::CAPTURED_ON,
        claude_wire::MIN_VERSION.0,
        claude_wire::MIN_VERSION.1,
        claude_wire::MIN_VERSION.2,
        claude_wire::KNOWN_PEER_PROTOCOL
    ));
    details.extend(unverified.iter().cloned());
    let mut summary = format!(
        "delivery = socket: {} lead(s) resolved, {} held, {} using herdr",
        status.leads.len(),
        status.held.len(),
        status.notes.len()
    );
    for note in &unverified {
        summary.push_str("; ");
        summary.push_str(note);
    }
    wire_drift(status, &subject, checks);
    if !status.dead_lettered.is_empty() {
        // Dead letters are bare messages with no reason: the reasons are only here.
        checks.push(
            Check::warn(
                Family::Socket,
                Code::SocketDeadLettered,
                Some(&subject),
                format!(
                    "the watcher dead-lettered {} batch(es) on its own account",
                    status.dead_lettered.len()
                ),
                "the receiver's user denied a held message, or nobody answered its approval \
                 prompt before the maximum hold; a dead letter does not carry the reason",
                "read them with `agent-msg inbox --dead`; to stop mail being held, set \
                 crossSessionInbound to accept or delivery = herdr",
            )
            .with_details(status.dead_lettered.iter().map(|d| {
                format!(
                    "{}: batch {} dead-lettered as {}",
                    d.lead, d.batch, d.reason
                )
            })),
        );
    }
    if !status.held.is_empty() {
        checks.push(
            Check::warn(
                Family::Socket,
                Code::SocketHeld,
                Some(&status.held.join(", ")),
                summary,
                "Claude Code holds relay mail behind an approval prompt for a bypass-mode lead \
                 without crossSessionInbound: accept; the mail waits in flight",
                "set crossSessionInbound to accept in that lead's Claude Code settings, or set \
                 delivery = herdr in ~/.ix/relay/config.toml",
            )
            .with_details(status.notes.iter().cloned().chain(details)),
        );
    } else if !status.notes.is_empty() || status.reply_socket.is_none() {
        let mut lines = status.notes.clone();
        if status.reply_socket.is_none() && !lines.iter().any(|n| n.contains("not bound")) {
            lines.push("reply socket not bound: every batch goes to herdr".to_owned());
        }
        checks.push(
            Check::warn(
                Family::Socket,
                Code::SocketFallback,
                Some(&subject),
                summary,
                "the socket could not be used for these leads (no session record, an unsupported \
                 Claude Code, several processes on one session, a refused connection, an \
                 unconfirmed delivery or a reply socket that is not bound yet); their batches \
                 were typed in through herdr",
                "nothing is lost; see the lines below, and the README section on delivery \
                 through the inbox socket",
            )
            .with_details(lines.into_iter().chain(details)),
        );
    } else {
        checks.push(Check::ok(Family::Socket, Some(&subject), summary).with_details(details));
    }
}

/// The drift signals of the typed Claude Code wire formats (`claude-wire`): warns, never
/// fails, and never changes how a message is delivered.
fn wire_drift(
    status: &relay_core::socket_status::SocketStatus,
    subject: &str,
    checks: &mut Vec<Check>,
) {
    let d = &status.drift;
    if d.control_total() > 0 {
        checks.push(
            Check::warn(
                Family::Socket,
                Code::SocketWireDrift,
                Some(subject),
                format!(
                    "Claude Code sent {} reply-socket line(s) in a shape this build does not \
                     know ({} unrecognised, {} not JSON, {} receipt(s) with an unknown status)",
                    d.control_total(),
                    d.unrecognised_lines,
                    d.not_json_lines,
                    d.unknown_status_receipts
                ),
                "Claude Code's cross-session formats are undocumented and can change in any \
                 release; the running version may not be the one the wire format was captured on",
                "delivery goes on (an unknown receipt leaves the batch in flight); check the \
                 Claude Code version against the one the wire format was captured on and update \
                 the claude-wire crate's types and fixtures",
            )
            .with_details([format!(
                "wire format captured on Claude Code {}",
                claude_wire::CAPTURED_ON
            )]),
        );
    }
    if d.transcript_shape_unrecognised > 0 {
        checks.push(Check::warn(
            Family::Socket,
            Code::SocketTranscriptShapeUnrecognised,
            Some(subject),
            format!(
                "{} transcript look(s) found a message id without a structured origin.msg_id",
                d.transcript_shape_unrecognised
            ),
            "Claude Code's transcript shape may have changed: such a message is acknowledged as \
             received but unconfirmed (its transcript names it), with no herdr fallback and no \
             redelivery; it can also be a lead that read socket-<pid>.json, which lists held \
             message ids",
            "check the Claude Code version against the one the wire format was captured on and \
             update the claude-wire crate's transcript type and fixtures",
        ));
    }
}

/// `redis_unreachable`'s cause and fix for what the connect hit. The bus is
/// relay-redis on the hub, reached over WireGuard at 10.99.0.1:6380 (README.md
/// "Transport"); each fault points at the one layer that produces it.
fn unreachable_hint(fault: LinkFault) -> (&'static str, &'static str) {
    macro_rules! retries {
        ($fix:literal) => {
            concat!(
                $fix,
                "; hooks and the watcher retry on their own once it answers"
            )
        };
    }
    match fault {
        LinkFault::Auth => (
            "the bus refused this machine's user or password",
            "compare `url` in `~/.ix/relay/config.toml` with what `scripts/relay-admin add-user \
             <machine>` printed on the hub (running it again issues a new password)",
        ),
        LinkFault::Refused => (
            "the bus address answered but nothing listens there: the hub's forwarder \
             (relay-redis-proxy.socket) is stopped",
            retries!(
                "on the hub: `systemctl --user enable --now relay-redis-proxy.socket`, or \
                 `scripts/relay-admin status`"
            ),
        ),
        LinkFault::Dropped => (
            "the hub's forwarder accepted the connection and then dropped it: relay-redis \
             behind it is down, or the forwarder points at the wrong kind node address",
            retries!(
                "on the hub: `kubectl -n relay get pods` and `journalctl --user -u \
                 relay-redis-proxy.service`, or `scripts/relay-admin status`"
            ),
        ),
        LinkFault::TimedOut => (
            "nothing came back: the WireGuard link to the hub carries nothing (the tunnel down on either \
             end, the hub asleep or off the LAN), or Redis is hung",
            retries!(
                "`wg show` here: no `latest handshake` in the last 2 minutes means the link is \
                 down (`sudo systemctl start wg-quick@relay`, or activate the tunnel in the WireGuard app); on the \
                 hub `scripts/relay-admin status`"
            ),
        ),
        LinkFault::NoRoute => (
            "this machine has no route to the bus address: WireGuard is not up here",
            retries!(
                "`sudo systemctl start wg-quick@relay` (or activate the tunnel in the WireGuard app), then \
                 `wg show`"
            ),
        ),
        LinkFault::Other => (
            "the URL in `~/.ix/relay/config.toml` is wrong, or Redis failed",
            retries!(
                "check `url` in `~/.ix/relay/config.toml`, and on the hub \
                 `scripts/relay-admin status`"
            ),
        ),
    }
}

fn redis(f: &Facts, up: Option<&RedisUp>, checks: &mut Vec<Check>) {
    match (&f.redis, up) {
        (RedisFacts::Down(fault, error), _) => {
            let (cause, fix) = unreachable_hint(*fault);
            checks.push(Check::fail(
                Family::Redis,
                Code::RedisUnreachable,
                None,
                format!("cannot reach redis at {}: {error}", f.redis_url),
                cause,
                fix,
            ));
        }
        (_, Some(up)) if up.rtt.as_millis() > SLOW_REDIS_MS => checks.push(Check::warn(
            Family::Redis,
            Code::RedisSlow,
            None,
            format!(
                "redis at {} answers in {} ms",
                f.redis_url,
                up.rtt.as_millis()
            ),
            "a congested link or a busy Redis: hooks give up after 300 ms and skip delivering",
            "check the WireGuard link (`wg show`) and, on the hub, `scripts/relay-admin status`",
        )),
        (_, Some(up)) => checks.push(Check::ok(
            Family::Redis,
            None,
            format!(
                "redis at {} answers in {} ms",
                f.redis_url,
                up.rtt.as_millis()
            ),
        )),
        (RedisFacts::NotAttempted, None) | (RedisFacts::Up(_), None) => {}
    }
    if let Some(up) = up {
        for (what, error) in [
            ("heartbeats", up.heartbeats.as_ref().err()),
            ("registrations", up.registrations.as_ref().err()),
            ("in-flight and dead-letter lists", up.backlog.as_ref().err()),
            ("in-flight leases", up.leases.as_ref().err()),
            ("hook failures", up.hook_failures.as_ref().err()),
        ] {
            if let Some(error) = error {
                checks.push(Check::fail(
                    Family::Redis,
                    Code::RedisReadFailed,
                    Some(what),
                    format!("could not read the {what}: {error}"),
                    "a key of the wrong type, a permission the Redis user lacks, the connection dropped, or doctor's time budget for Redis ran out",
                    "read the key with redis-cli; a key of another type under the relay's prefix has to go",
                ));
            }
        }
    }
}

/// The local machine's live heartbeat, if any.
fn local_beat<'a>(f: &Facts, up: Option<&'a RedisUp>) -> Option<&'a WatcherHeartbeat> {
    let beats = up?.heartbeats.as_ref().ok()?;
    beats
        .found
        .iter()
        .find_map(|b| match (&b.read, b.machine == f.machine) {
            (HeartbeatRead::Live(hb), true) => Some(hb.as_ref()),
            _ => None,
        })
}

/// What the registry says about a machine's leads.
struct MachineLeads {
    /// How long ago its most recently seen lead was seen, in milliseconds.
    freshest_ms: i64,
}

/// Each machine that has a registered lead, and how recently one was seen.
fn leads_by_machine(f: &Facts, up: &RedisUp) -> BTreeMap<MachineName, MachineLeads> {
    let mut leads: BTreeMap<MachineName, MachineLeads> = BTreeMap::new();
    if let Ok((regs, _)) = &up.registrations {
        for r in regs {
            let age = f.now_ms.saturating_sub(r.last_seen_ms);
            let entry = leads
                .entry(r.machine.clone())
                .or_insert(MachineLeads { freshest_ms: age });
            entry.freshest_ms = entry.freshest_ms.min(age);
        }
    }
    leads
}

/// Whether a machine with no live heartbeat is an alarm, and it must not depend
/// on anything the missing watcher itself keeps fresh (`last_seen_ms` is
/// refreshed by that watcher, so an hour after it dies every idle lead looks
/// long gone). Two evidence sources that a dead watcher does not maintain:
///
/// - the non-expiring last-tick marker the watcher wrote while it lived: a
///   machine with a registered lead whose watcher ticked within the prune
///   horizon (24 h) is an alarm for that whole horizon, and drops to a warning
///   only once the machine has been silent longer (asleep, or decommissioned);
/// - a lead seen in the last hour, for a machine whose watcher predates the
///   marker (an old binary writes neither).
fn missing_is_alarm(leads: Option<&MachineLeads>, last_tick_age_ms: Option<i64>) -> bool {
    let marker = leads.is_some() && last_tick_age_ms.is_some_and(|a| a < DAY_MS);
    let recent_lead = leads.is_some_and(|l| l.freshest_ms < RECENT_LEAD_MS);
    marker || recent_lead
}

/// The watcher heartbeat of this machine, and of every other machine that has a
/// lead seen in the last day or a heartbeat of its own.
fn heartbeats(f: &Facts, up: &RedisUp, checks: &mut Vec<Check>) {
    let Ok(beats) = &up.heartbeats else {
        return;
    };
    let by_machine: BTreeMap<&MachineName, &MachineHeartbeat> =
        beats.found.iter().map(|b| (&b.machine, b)).collect();
    // The scan ran out of doctor's time: nothing was read, so nothing is claimed
    // about any machine's watcher, only that the reading was cut.
    if beats.truncated == Some(ScanCut::Deadline) {
        checks.push(heartbeats_cut(ScanCut::Deadline));
        return;
    }
    let leads = leads_by_machine(f, up);
    let mut machines: BTreeSet<MachineName> =
        beats.found.iter().map(|b| b.machine.clone()).collect();
    machines.insert(f.machine.clone());
    machines.extend(
        leads
            .iter()
            .filter(|(_, l)| l.freshest_ms < DAY_MS)
            .map(|(m, _)| m.clone()),
    );
    // A machine that ever ran a heartbeat-writing watcher and has leads is listed
    // even when its leads look long gone (nothing refreshes them once the watcher
    // is dead): an alarm for the prune horizon, a warning after it.
    machines.extend(
        beats
            .last_ticks
            .iter()
            .filter(|(m, _)| leads.contains_key(m))
            .map(|(m, _)| m.clone()),
    );
    for machine in machines {
        checks.push(heartbeat_check(
            f,
            &machine,
            by_machine.get(&machine).copied(),
            leads.get(&machine),
            beats
                .last_ticks
                .get(&machine)
                .map(|t| f.now_ms.saturating_sub(*t)),
        ));
    }
    if beats.unreadable > 0 {
        checks.push(Check::warn(
            Family::Watcher,
            Code::WatcherHeartbeatUnreadable,
            Some("watcher-seen"),
            format!(
                "{} heartbeat key(s) or last-tick marker(s) name no machine and were left out",
                beats.unreadable
            ),
            "something wrote a heartbeat key or a `health:watcher-seen` field whose machine is not a machine name",
            "remove the stray keys or fields; a watcher writes only its own machine's",
        ));
    }
    if let Some(cut) = beats.truncated {
        checks.push(heartbeats_cut(cut));
    }
}

/// The warning for a heartbeat scan that stopped at a bound, naming the bound.
fn heartbeats_cut(cut: ScanCut) -> Check {
    let (which, cause) = match cut {
        ScanCut::Keys => (
            "its key bound (256)",
            "the bus has more than 256 heartbeat keys",
        ),
        ScanCut::Calls => (
            "its call bound (64 SCAN calls)",
            "the Redis keyspace is large: the scan looks at 1,000 keys per call",
        ),
        ScanCut::Deadline => (
            "doctor's time budget for Redis, so no watcher heartbeat was read",
            "Redis answered slowly: each SCAN call took long enough that the budget ran out",
        ),
    };
    Check::warn(
        Family::Watcher,
        Code::HeartbeatScanTruncated,
        None,
        format!("the scan for watcher heartbeats stopped at {which}"),
        cause,
        if cut == ScanCut::Deadline {
            "check the WireGuard link (`wg show`) and Redis load, and run doctor again"
        } else {
            "this machine's and every lead's machine's heartbeat were still read by name"
        },
    )
}

fn heartbeat_check(
    f: &Facts,
    machine: &MachineName,
    beat: Option<&MachineHeartbeat>,
    leads: Option<&MachineLeads>,
    last_tick_age_ms: Option<i64>,
) -> Check {
    let local = *machine == f.machine;
    let Some(beat) = beat else {
        return heartbeat_missing(f, machine, local, leads, last_tick_age_ms);
    };
    let HeartbeatRead::Live(hb) = &beat.read else {
        return Check::warn(
            Family::Watcher,
            Code::WatcherHeartbeatUnreadable,
            Some(machine.as_str()),
            format!("the heartbeat key for {machine} is not a heartbeat this binary can read"),
            "a watcher of a newer, incompatible version wrote it, or something else wrote the key",
            "update agent-msg on this machine, or delete the key once the watcher rewrites it",
        );
    };
    let age = f.now_ms.saturating_sub(hb.last_tick_ms);
    let ttl_ms =
        i64::try_from(heartbeat_ttl(std::time::Duration::from_secs(hb.sweep_secs)).as_millis())
            .unwrap_or(i64::MAX);
    let summary = summary_of(f, hb, age);
    if tick_is_stale(age, hb.sweep_secs) {
        let cause = if age > ttl_ms {
            "the clocks of the two machines disagree (a key this old would have expired)"
        } else {
            "the watcher stopped ticking: it is wedged, or blocked on a hung herdr or Redis call; \
             its key has not expired yet"
        };
        return Check::fail(
            Family::Watcher,
            Code::WatcherHeartbeatStale,
            Some(machine.as_str()),
            format!(
                "the watcher on {machine} last ticked {} ago: {summary}",
                ago(age)
            ),
            cause,
            "restart the watcher and read its journal; compare `date` on both machines",
        );
    }
    if let (false, Some(problem)) = (hb.herdr_ok, hb.herdr_problem.as_deref()) {
        let consequence = if hb.herdr_blocks_delivery {
            "it types into no pane until this passes"
        } else {
            "it keeps running, but may skip leads"
        };
        return Check::severe(
            hb.herdr_blocks_delivery,
            Family::Watcher,
            Code::WatcherHerdrProblem,
            Some(machine.as_str()),
            format!(
                "the watcher on {machine} reports herdr failing its check: {problem}: {consequence}"
            ),
            "herdr was replaced or changed its output",
            "install a supported herdr on that machine; the watcher re-checks by itself",
        );
    }
    if let Some(OtlpStatus::Failing {
        target,
        reason,
        counters,
    }) = &hb.otlp
    {
        return Check::warn(
            Family::Watcher,
            Code::WatcherOtlpFailing,
            Some(machine.as_str()),
            format!(
                "the watcher on {machine} cannot push metrics to {}: {}",
                relay_core::text::neutralise(target),
                relay_core::text::neutralise(reason)
            ),
            "the collector is down, unreachable, slow or refusing the data; deliveries are not affected",
            "check the collector at that address, or unset [watcher] otlp_endpoint; the watcher retries by itself",
        )
        .with_details(counters_line(counters));
    }
    Check::ok(
        Family::Watcher,
        Some(machine.as_str()),
        format!("watcher on {machine}: {summary}"),
    )
    .with_details(heartbeat_details(f, hb))
}

/// No key: the wording and the severity depend on whether anything there needs
/// a watcher, and whether a watcher visibly runs.
fn heartbeat_missing(
    f: &Facts,
    machine: &MachineName,
    local: bool,
    leads: Option<&MachineLeads>,
    last_tick_age_ms: Option<i64>,
) -> Check {
    let last = last_tick_age_ms.map_or(String::new(), |a| {
        format!("; its last tick was {} ago", ago(a))
    });
    let message = format!(
        "no watcher heartbeat from {machine} (watcher stopped, or older than the heartbeat){last}"
    );
    // A local watcher that visibly runs (a live pid, a fresh state file) and
    // publishes nothing is one that predates the heartbeat.
    if local
        && let FileState::Read { file, pid_alive } = &f.watcher_file
        && *pid_alive != Some(false)
        && file
            .last_ok_tick_ms
            .is_some_and(|t| f.now_ms.saturating_sub(t) < FILE_STALE_MIN_MS)
    {
        return Check::warn(
            Family::Watcher,
            Code::WatcherHeartbeatMissing,
            Some(machine.as_str()),
            format!(
                "{message}: pid {} runs here and polls, so it is older than the heartbeat",
                file.pid
            ),
            "the running watcher was built before the heartbeat existed",
            "reinstall relay-watcher from this repo and restart it",
        );
    }
    let alarm = missing_is_alarm(leads, last_tick_age_ms);
    Check::severe(
        alarm,
        Family::Watcher,
        Code::WatcherHeartbeatMissing,
        Some(machine.as_str()),
        format!(
            "{message}{}",
            if alarm {
                ": leads there are registered, so idle leads are not being woken"
            } else {
                ""
            }
        ),
        &format!(
            "the watcher on {} is stopped, not installed, cannot reach this Redis, or is a version that predates the heartbeat",
            if local {
                "this machine"
            } else {
                "that machine"
            }
        ),
        &if local {
            "`systemctl --user status relay-watcher` (or `launchctl list com.agent-ix.relay-watcher`); \
             install it as README.md \"Installing the watcher\" says"
                .to_owned()
        } else {
            format!(
                "on {machine}: start or install relay-watcher; see README.md \"Installing the watcher\""
            )
        },
    )
}

fn summary_of(f: &Facts, hb: &WatcherHeartbeat, age_ms: i64) -> String {
    let sweep = hb.last_sweep_ms.map_or_else(
        || "no sweep yet".to_owned(),
        |t| format!("last sweep {} ago", ago(f.now_ms.saturating_sub(t))),
    );
    format!(
        "pid {}, v{}, up {}, last tick {} ago, {sweep}, {} lead(s) watched",
        hb.pid,
        hb.version,
        ago(f.now_ms.saturating_sub(hb.started_at_ms)),
        ago(age_ms),
        hb.leads_watched
    )
}

/// The exporter's counters as one detail line, when any is not zero.
fn counters_line(c: &relay_core::health::OtlpCounters) -> Option<String> {
    (!c.is_empty()).then(|| {
        format!(
            "otlp counters: {} push(es) sent, {} failed, {} snapshot(s) dropped, {} point(s) rejected",
            c.sent, c.failed, c.dropped_snapshots, c.rejected_points
        )
    })
}

/// What a healthy heartbeat still says: a ridden-out Redis flap, and the last
/// failed tick.
fn heartbeat_details(f: &Facts, hb: &WatcherHeartbeat) -> Vec<String> {
    let mut details = Vec::new();
    if !hb.redis_ok {
        details.push(
            "a Redis connection error was ridden out since the previous heartbeat".to_owned(),
        );
    }
    if let Some(error) = &hb.last_error {
        details.push(format!(
            "last failed tick {} ago: {}",
            ago(f.now_ms.saturating_sub(error.at_ms)),
            wire_name(&error.kind)
        ));
    }
    match &hb.otlp {
        Some(OtlpStatus::Sending {
            target,
            dropped,
            counters,
        }) => {
            let mut line = format!("otlp: sending to {}", relay_core::text::neutralise(target));
            if *dropped > 0 {
                line.push_str(&format!(
                    " ({dropped} series left out or folded into lead=other to stay in bounds)"
                ));
            }
            details.push(line);
            details.extend(counters_line(counters));
        }
        Some(OtlpStatus::Disabled) => details.push("otlp: disabled".to_owned()),
        Some(OtlpStatus::Ignoring { note }) => details.push(format!(
            "otlp: disabled ({})",
            relay_core::text::neutralise(note)
        )),
        // A watcher too old to say, or one whose state this binary cannot read.
        None | Some(OtlpStatus::Unknown) => details.push("otlp: unknown".to_owned()),
        // A warning of its own says it.
        Some(OtlpStatus::Failing { .. }) => {}
    }
    details
}

/// `state/watcher.json`, and whether it agrees with this machine's heartbeat.
fn watcher_file(f: &Facts, up: Option<&RedisUp>, checks: &mut Vec<Check>) {
    let subject = Some("watcher.json");
    let local = local_beat(f, up);
    match &f.watcher_file {
        FileState::NoPath => checks.push(Check::unknown(
            Family::WatcherFile,
            Code::WatcherFileUnlocated,
            subject,
            "no config location (neither HOME nor IX_RELAY_CONFIG is set), so no state file is looked for",
            "set HOME or IX_RELAY_CONFIG",
        )),
        FileState::Missing => checks.push(Check::severe(
            local.is_none(),
            Family::WatcherFile,
            Code::WatcherFileMissing,
            subject,
            "the watcher state file is missing",
            "no watcher has run under this config directory, or it runs with a different IX_RELAY_CONFIG",
            "start the watcher, and give it the same IX_RELAY_CONFIG as agent-msg",
        )),
        FileState::Unreadable(error) => checks.push(Check::warn(
            Family::WatcherFile,
            Code::WatcherFileUnreadable,
            subject,
            format!("the watcher state file cannot be read: {error}"),
            "a permissions problem, or a file that is not a state file",
            "delete state/watcher.json; the watcher rewrites it on its next poll",
        )),
        FileState::Read { file, pid_alive } => {
            let limit = local
                .map_or(FILE_STALE_MIN_MS, |hb| {
                    i64::try_from(hb.sweep_secs.saturating_mul(4_000)).unwrap_or(i64::MAX)
                })
                .max(FILE_STALE_MIN_MS);
            let age = file.last_ok_tick_ms.map(|t| f.now_ms.saturating_sub(t));
            if *pid_alive == Some(false) {
                checks.push(Check::fail(
                    Family::WatcherFile,
                    Code::WatcherPidGone,
                    subject,
                    format!("watcher.json names pid {}, which is not running", file.pid),
                    "the watcher was stopped or killed and nothing restarted it",
                    "`systemctl --user restart relay-watcher` (or `launchctl kickstart`)",
                ));
            } else if age.is_none_or(|a| a > limit) {
                checks.push(Check::fail(
                    Family::WatcherFile,
                    Code::WatcherFileStale,
                    subject,
                    match age {
                        Some(a) => format!("the watcher's last good poll was {} ago", ago(a)),
                        None => "the watcher has never completed a poll".to_owned(),
                    },
                    "the watcher is stopped, wedged, or cannot reach Redis (see last_error in the file)",
                    "read state/watcher.json and `journalctl --user -u relay-watcher`",
                ));
            } else if let Some(hb) = local.filter(|hb| hb.pid != file.pid) {
                checks.push(Check::warn(
                    Family::WatcherFile,
                    Code::WatcherFileMismatch,
                    subject,
                    format!(
                        "watcher.json is pid {} but the bus heartbeat for this machine is pid {}",
                        file.pid, hb.pid
                    ),
                    "two watchers run for this machine, or they use different config directories",
                    "stop the extra watcher, and give both the same IX_RELAY_CONFIG",
                ));
            } else {
                checks.push(Check::ok(
                    Family::WatcherFile,
                    subject,
                    format!(
                        "watcher.json: pid {}, last good poll {} ago",
                        file.pid,
                        ago(age.unwrap_or(0))
                    ),
                ));
            }
        }
    }
}

/// Failure counts by `stage/error_kind`, as detail lines.
fn group_lines(records: &[&SpoolRecord]) -> Vec<String> {
    let mut groups: BTreeMap<(String, String), (usize, BTreeSet<MachineName>)> = BTreeMap::new();
    for r in records {
        let entry = groups
            .entry((wire_name(&r.stage), wire_name(&r.error_kind)))
            .or_default();
        entry.0 += 1;
        entry.1.insert(r.machine.clone());
    }
    let mut lines: Vec<String> = groups
        .into_iter()
        .map(|((stage, kind), (n, machines))| {
            format!(
                "{stage} / {kind}: {n} ({})",
                machines
                    .iter()
                    .map(MachineName::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
        .collect();
    // Records the spool dropped to stay inside its bound, or lines it removed that
    // were not records (an id outside its type's rule, bytes no binary wrote).
    let dropped: u64 = records.iter().map(|r| r.dropped).sum();
    if dropped > 0 {
        lines.push(format!(
            "{dropped} spooled line(s) were dropped or removed unreadable before these"
        ));
    }
    lines.extend(
        records
            .iter()
            .filter(|r| r.stage == Stage::RequeueFailed)
            .map(|r| {
                // The record is the only copy of these messages: name whose they are
                // and show the start of each, neutralised.
                let previews: Vec<String> = r
                    .messages
                    .iter()
                    .map(|m| {
                        relay_core::text::neutralise(&m.text)
                            .chars()
                            .take(PREVIEW_CHARS)
                            .collect()
                    })
                    .collect();
                format!(
                    "requeue_failed on {}: lead {} lost {} message(s): {}",
                    r.machine,
                    r.lead.as_deref().unwrap_or("?"),
                    previews.len(),
                    previews.join(" | ")
                )
            }),
    );
    lines
}

/// Whether any of `records` is under [`RECENT_FAILURE_MS`] old.
fn any_recent(f: &Facts, records: &[&SpoolRecord]) -> bool {
    records
        .iter()
        .any(|r| f.now_ms.saturating_sub(r.at_ms) < RECENT_FAILURE_MS)
}

fn spool(f: &Facts, checks: &mut Vec<Check>) {
    let subject = Some("hook spool");
    match &f.spool {
        SpoolState::NoPath => checks.push(Check::unknown(
            Family::HookSpool,
            Code::HookSpoolUnlocated,
            subject,
            "no config location, so no hook spool is looked for",
            "set HOME or IX_RELAY_CONFIG",
        )),
        SpoolState::Empty => checks.push(Check::ok(
            Family::HookSpool,
            subject,
            "no hook failures are waiting in the local spool",
        )),
        SpoolState::Unreadable(error) => checks.push(Check::warn(
            Family::HookSpool,
            Code::HookSpoolUnreadable,
            subject,
            format!("the hook spool cannot be read: {error}"),
            "a permissions problem, or the spool path is a symlink or not a file",
            "inspect spool/hook.jsonl beside config.toml",
        )),
        SpoolState::Pending {
            bytes,
            records,
            undecodable,
            modified_ms,
        } => {
            let refs: Vec<&SpoolRecord> = records.iter().collect();
            // Age comes from the newest record that decodes; a spool with none
            // that decodes is as old as its file's last write.
            let recent = if refs.is_empty() {
                modified_ms.is_some_and(|t| f.now_ms.saturating_sub(t) < RECENT_FAILURE_MS)
            } else {
                any_recent(f, &refs)
            };
            let mut details = group_lines(&refs);
            if *undecodable > 0 {
                details.push(format!("{undecodable} spooled line(s) do not decode"));
            }
            checks.push(
                Check::severe(
                    recent,
                    Family::HookSpool,
                    Code::HookSpoolPending,
                    subject,
                    format!(
                        "{} hook failure(s) ({bytes} bytes) are waiting in the local spool, not yet on the bus",
                        records.len()
                    ),
                    "hooks on this machine cannot reach Redis, or none has run since it came back",
                    "fix the Redis check above; the next healthy hook call ships them",
                )
                .with_details(details),
            );
        }
    }
}

fn bus_failures(f: &Facts, up: &RedisUp, checks: &mut Vec<Check>) {
    let Ok(failures) = &up.hook_failures else {
        return;
    };
    let recent: Vec<&SpoolRecord> = failures
        .iter()
        .filter_map(|BusFailure { record, .. }| record.as_ref())
        .filter(|r| f.now_ms.saturating_sub(r.at_ms) < DAY_MS)
        .collect();
    let undecodable = failures.iter().filter(|b| b.record.is_none()).count() + up.hook_unreadable;
    if recent.is_empty() && undecodable == 0 {
        checks.push(Check::ok(
            Family::HookFailures,
            None,
            "no hook failures were recorded on the bus in the last 24 hours",
        ));
        return;
    }
    let mut details = group_lines(&recent);
    if undecodable > 0 {
        details.push(format!("{undecodable} record(s) on the bus do not decode"));
    }
    let now_bad = recent
        .iter()
        .filter(|r| f.now_ms.saturating_sub(r.at_ms) < RECENT_FAILURE_MS)
        .count();
    checks.push(
        Check::severe(
            now_bad > 0,
            Family::HookFailures,
            Code::HookFailuresRecorded,
            None,
            format!(
                "{} hook failure(s) were recorded in the last 24 hours, {now_bad} in the last 15 minutes",
                recent.len()
            ),
            "hooks failed on some machine: Redis unreachable, a budget spent, an output pipe closed",
            "the stage / error_kind lines say where; a repeated `connect` is a link problem (see `redis_unreachable`)",
        )
        .with_details(details),
    );
}

/// The most move lines one check lists.
const MOVE_LINES: usize = 10;

/// Panes that moved and places that were renamed in the last 24 hours: what the
/// watcher did on its own, so a lead's changed address is explained. Information,
/// never a failure (FR-043).
fn recent_moves(f: &Facts, up: &RedisUp, checks: &mut Vec<Check>) {
    let read = match &up.moves {
        Ok(read) => read,
        Err(error) => {
            // Recorded moves explain an address; failing to read them is not a
            // failure of the relay (FR-043): reported, never failing the exit.
            checks.push(Check::unknown(
                Family::Moves,
                Code::MovesUnreadable,
                None,
                format!("could not read the recorded pane moves: {error}"),
                "read the `health:moves` key with redis-cli; a key of another type under the relay's prefix has to go",
            ));
            return;
        }
    };
    let moves = &read.moves;
    let recent: Vec<&MoveEvent> = moves
        .entries
        .iter()
        .filter(|e| f.now_ms.saturating_sub(e.at_ms) < DAY_MS)
        .collect();
    if recent.is_empty() && moves.unreadable == 0 {
        checks.push(Check::ok(
            Family::Moves,
            None,
            "no pane moves or renames were recorded in the last 24 hours",
        ));
        return;
    }
    let relinks = recent.iter().filter(|e| e.kind == MoveKind::Relink).count();
    let mut details: Vec<String> = recent
        .iter()
        .take(MOVE_LINES)
        .map(|e| move_line(f.now_ms, e))
        .collect();
    if recent.len() > MOVE_LINES {
        details.push(format!("{} older", recent.len() - MOVE_LINES));
    }
    if moves.unreadable > 0 {
        details.push(format!(
            "{} record(s) on the bus do not decode",
            moves.unreadable
        ));
    }
    checks.push(
        Check::ok(
            Family::Moves,
            None,
            format!(
                "{}{} pane move(s) and rename(s) were recorded in the last 24 hours, {relinks} of them relinks",
                if read.more { "at least " } else { "" },
                recent.len()
            ),
        )
        .with_details(details),
    );
}

/// `text` cut to [`relay_core::MAX_LABEL_BYTES`] bytes at a character boundary,
/// with `...` when it was cut: a herdr label is any length, a detail line is not.
fn bounded(text: &str) -> String {
    if text.len() <= relay_core::MAX_LABEL_BYTES {
        return text.to_owned();
    }
    let mut end = relay_core::MAX_LABEL_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &text[..end])
}

/// One recorded change, as a line: how long ago, which lead, what changed.
fn move_line(now_ms: i64, e: &MoveEvent) -> String {
    let at = |place: &Option<relay_core::Place>| {
        place.as_ref().map_or_else(
            || "-".to_owned(),
            |p| bounded(&p.address(&e.machine).to_string()),
        )
    };
    let pane = |p: &Option<relay_core::PaneId>| p.as_ref().map_or("-", |p| p.as_str()).to_owned();
    let what = match e.kind {
        MoveKind::Relink => format!("moved pane {} to {}", pane(&e.previous_pane), pane(&e.pane)),
        MoveKind::Rename => format!("renamed at pane {}", pane(&e.pane)),
        MoveKind::Unknown => "changed (a kind this version does not know)".to_owned(),
    };
    format!(
        "{} ago: {} {what}, address {} -> {}",
        ago(now_ms.saturating_sub(e.at_ms)),
        e.lead,
        at(&e.old_place),
        at(&e.new_place)
    )
}

fn herdr(f: &Facts, checks: &mut Vec<Check>) {
    let subject = Some("herdr");
    checks.push(match &f.herdr {
        HerdrCheck::Ok {
            version,
            shape_verified,
        } => Check::ok(
            Family::Herdr,
            subject,
            format!(
                "herdr {version}: {}",
                if *shape_verified {
                    "version and output shape are supported"
                } else {
                    "version is supported (no agents to verify the output shape)"
                }
            ),
        ),
        HerdrCheck::Missing { .. } => Check::fail(
            Family::Herdr,
            Code::HerdrMissing,
            subject,
            f.herdr.problem().unwrap_or_default(),
            "herdr is not installed or not on PATH for this shell",
            "install herdr, or put it on PATH (a systemd unit sets its own PATH)",
        ),
        HerdrCheck::Unsupported { .. } => Check::warn(
            Family::Herdr,
            Code::HerdrUnsupportedVersion,
            subject,
            f.herdr.problem().unwrap_or_default(),
            "herdr is older than the version ix-relay's decoders were written against; its output shape is fine, so the watcher keeps delivering",
            "upgrade herdr",
        ),
        HerdrCheck::ShapeMismatch { .. } => Check::fail(
            Family::Herdr,
            Code::HerdrShapeMismatch,
            subject,
            f.herdr.problem().unwrap_or_default(),
            "herdr changed its output: the watcher types into no pane until ix-relay can read it",
            "upgrade ix-relay to a release that reads this herdr, or downgrade herdr",
        ),
        HerdrCheck::VersionUnreadable { .. } => Check::warn(
            Family::Herdr,
            Code::HerdrVersionUnreadable,
            subject,
            f.herdr.problem().unwrap_or_default(),
            "`herdr --version` printed something that is not a version",
            "run `herdr --version` by hand",
        ),
        HerdrCheck::Failed { .. } | HerdrCheck::NotChecked => Check::unknown(
            Family::Herdr,
            Code::HerdrUnchecked,
            subject,
            f.herdr
                .problem()
                .unwrap_or_else(|| "herdr was not checked".to_owned()),
            "run `herdr agent list` by hand",
        ),
    });
}

/// Registration problems, one check per lead and problem.
fn registrations(f: &Facts, up: &RedisUp, checks: &mut Vec<Check>) {
    let Ok((regs, skipped)) = &up.registrations else {
        return;
    };
    let before = checks.len();
    if *skipped > 0 {
        checks.push(Check::warn(
            Family::Registration,
            Code::RegistrationUndecodable,
            None,
            format!("{skipped} registry record(s) cannot be decoded and are skipped"),
            "written by a binary of another version, or by hand",
            "`agent-msg who` names the fields; update agent-msg or delete the record",
        ));
    }
    let mut unchecked_panes = false;
    for reg in regs {
        registration_problems(f, reg, &mut unchecked_panes, checks);
    }
    if unchecked_panes && let Err(error) = &f.panes {
        checks.push(Check::unknown(
            Family::Registration,
            Code::PanesUnchecked,
            None,
            format!("this machine's registrations were not checked against herdr's panes: {error}"),
            "fix the herdr check above",
        ));
    }
    if checks.len() == before {
        checks.push(Check::ok(
            Family::Registration,
            None,
            format!("{} registration(s) look healthy", regs.len()),
        ));
    }
}

fn registration_problems(
    f: &Facts,
    reg: &Registration,
    unchecked_panes: &mut bool,
    checks: &mut Vec<Check>,
) {
    let name = reg.name.as_str();
    let machine = &reg.machine;
    let age = f.now_ms.saturating_sub(reg.last_seen_ms);
    if age >= DAY_MS {
        checks.push(Check::warn(
            Family::Registration,
            Code::RegistrationStale,
            Some(name),
            format!("{name} on {machine} was last seen {} ago", ago(age)),
            &format!("nothing prunes it: the watcher on {machine} is down, or the machine is gone"),
            "start the watcher there, or remove the record once the lead is gone",
        ));
        return;
    }
    if *machine != f.machine {
        return;
    }
    let Some(pane) = reg.herdr_pane.as_ref() else {
        return;
    };
    if reg.place.is_none() {
        checks.push(Check::warn(
            Family::Registration,
            Code::RegistrationNoPlace,
            Some(name),
            format!("{name} has a pane ({pane}) and no place, so it has no address"),
            "the watcher has not read herdr's layout for it: it is down or herdr is failing",
            "fix the watcher and herdr checks above; the watcher fills the place in",
        ));
    }
    match &f.panes {
        // A pane the watcher still vouches for is refreshed within the hour, so a
        // stamp older than the grace on a pane herdr lacks is a pane that has
        // been gone that long.
        Ok(panes) if !panes.contains(pane) && age >= PANE_GONE_GRACE_MS => {
            checks.push(Check::warn(
                Family::Registration,
                Code::RegistrationPaneGone,
                Some(name),
                format!(
                    "{name}'s pane {pane} has not been in herdr for about {}",
                    ago(age)
                ),
                "the session was closed; mail queued for it waits until the lead registers again",
                "nothing to do: the watcher prunes it after 24 hours idle",
            ));
        }
        Ok(_) => {}
        Err(_) => *unchecked_panes = true,
    }
}

/// Dead letters and in-flight entries, by lead.
fn backlog(up: &RedisUp, checks: &mut Vec<Check>) {
    let Ok(found) = &up.backlog else {
        return;
    };
    let mut any_dead = false;
    for lead in &found.leads {
        if lead.unreadable {
            checks.push(Check::fail(
                Family::DeadLetter,
                Code::RedisReadFailed,
                Some(&lead.lead),
                format!(
                    "the in-flight or dead-letter list of {} cannot be read",
                    lead.lead
                ),
                "one of its keys is not a list",
                "inspect it with redis-cli",
            ));
        } else if lead.dead > 0 {
            any_dead = true;
            checks.push(Check::warn(
                Family::DeadLetter,
                Code::DeadLettersPresent,
                Some(&lead.lead),
                format!(
                    "{} dead letter(s) for {}: messages that could not be delivered",
                    lead.dead, lead.lead
                ),
                "a message failed delivery repeatedly and was set aside instead of dropped; they stay until the list's cap drops the oldest",
                "read them with `agent-msg inbox --dead` as that lead",
            ));
        }
    }
    if !any_dead {
        checks.push(Check::ok(Family::DeadLetter, None, "no dead letters"));
    }
    in_flight(up, found, checks);
    if found.truncated {
        checks.push(Check::warn(
            Family::InFlight,
            Code::BacklogScanTruncated,
            None,
            "the scan for in-flight and dead-letter lists hit its bound: some leads may be missing from this report",
            "the bus has over a thousand such lists",
            "`agent-msg inbox --in-flight` shows the same bound; delete lists of leads that are gone",
        ));
    }
}

/// In-flight entries against batches, and leases past their deadline.
fn in_flight(up: &RedisUp, found: &relay_core::Backlog, checks: &mut Vec<Check>) {
    let leases: HashMap<&str, &relay_core::LeaseState> = up
        .leases
        .as_ref()
        .map(|all| all.iter().map(|l| (l.lead.as_str(), l)).collect())
        .unwrap_or_default();
    let mut lines = Vec::new();
    let mut overdue_leads = Vec::new();
    for lead in found
        .leads
        .iter()
        .filter(|l| l.in_flight > 0 && !l.unreadable)
    {
        let Some(state) = leases.get(lead.lead.as_str()) else {
            lines.push(format!(
                "{}: {} entr(y/ies) in flight",
                lead.lead, lead.in_flight
            ));
            continue;
        };
        let cut = if state.rows_cut {
            " (first rows read)"
        } else {
            ""
        };
        lines.push(format!(
            "{}: {} entr(y/ies) in {} batch(es){cut}, {} overdue",
            lead.lead, state.entries, state.batches, state.overdue
        ));
        if state.overdue > 0 && state.most_overdue_ms > LEASE_OVERDUE_GRACE_MS {
            overdue_leads.push(format!(
                "{}: {} entr(y/ies) {} past their lease",
                lead.lead,
                state.overdue,
                ago(state.most_overdue_ms)
            ));
        }
    }
    if overdue_leads.is_empty() {
        checks.push(
            Check::ok(
                Family::InFlight,
                None,
                if lines.is_empty() {
                    "nothing is in flight".to_owned()
                } else {
                    format!(
                        "{} lead(s) have entries in flight, none overdue",
                        lines.len()
                    )
                },
            )
            .with_details(lines),
        );
    } else {
        checks.push(
            Check::warn(
                Family::InFlight,
                Code::InflightOverdue,
                None,
                format!("{} lead(s) have in-flight entries past their lease", overdue_leads.len()),
                "a popped batch was neither acknowledged nor reaped: no watcher is sweeping, or the hook that popped it died",
                "check the watcher above; the next sweep of a running watcher redelivers them",
            )
            .with_details(overdue_leads.into_iter().chain(lines)),
        );
    }
}

/// Whether `machine`'s watcher being what it is means mail will not be woken for:
/// a wedged heartbeat (last tick too old), or a missing one where
/// [`missing_is_alarm`] says a watcher is expected. A fresh heartbeat is not, and
/// an unreadable one is not known to be.
fn watcher_alarm(
    f: &Facts,
    up: &RedisUp,
    machine: &MachineName,
    leads: Option<&MachineLeads>,
) -> bool {
    let Ok(beats) = &up.heartbeats else {
        return false;
    };
    let last_tick = beats
        .last_ticks
        .get(machine)
        .map(|t| f.now_ms.saturating_sub(*t));
    match beats
        .found
        .iter()
        .find(|b| b.machine == *machine)
        .map(|b| &b.read)
    {
        Some(HeartbeatRead::Live(hb)) => {
            tick_is_stale(f.now_ms.saturating_sub(hb.last_tick_ms), hb.sweep_secs)
        }
        Some(HeartbeatRead::Undecodable) => false,
        None => missing_is_alarm(leads, last_tick),
    }
}

/// Inbox depth per lead, and the age of the oldest waiting message.
fn inboxes(f: &Facts, up: &RedisUp, checks: &mut Vec<Check>) {
    let machine_of: HashMap<&str, MachineName> = up
        .registrations
        .as_ref()
        .map(|(regs, _)| {
            regs.iter()
                .map(|r| (r.name.as_str(), r.machine.clone()))
                .collect()
        })
        .unwrap_or_default();
    let leads = leads_by_machine(f, up);
    let mut waiting = Vec::new();
    let mut stuck = 0;
    for (lead, overview) in &up.inboxes {
        match overview {
            Err(error) => checks.push(Check::fail(
                Family::Inbox,
                Code::RedisReadFailed,
                Some(lead.as_str()),
                format!("cannot read {lead}'s inbox: {error}"),
                "the key is not a list, or doctor's time budget for Redis ran out",
                "inspect it with redis-cli",
            )),
            Ok(o) if o.len == 0 => {}
            Ok(o) => {
                let age = o.oldest_sent_at_ms.map(|t| f.now_ms.saturating_sub(t));
                let line = format!(
                    "{lead}: {} waiting, oldest {}",
                    o.len,
                    age.map_or_else(|| "of unknown age".to_owned(), ago)
                );
                if age.is_some_and(|a| a > INBOX_STUCK_MS) {
                    stuck += 1;
                    // Mail a fresh watcher has not delivered is a lead busy in a long
                    // call (a subagent, a long tool run), which the watcher leaves to
                    // its own hook: a warning. It is a failure when nothing would
                    // wake the lead: its machine's watcher is wedged, or missing
                    // where a watcher is expected.
                    let machine = machine_of.get(lead.as_str());
                    let fails = machine.is_none_or(|m| watcher_alarm(f, up, m, leads.get(m)));
                    checks.push(Check::severe(
                        fails,
                        Family::Inbox,
                        Code::InboxStuck,
                        Some(lead.as_str()),
                        format!("{line}: longer than a delivery should take"),
                        "the lead is idle and no watcher wakes it, or it is mid-turn for a long time; if its session closed, nothing will read it",
                        "check the watcher for that lead's machine above; the lead reads it on its next tool call",
                    ));
                } else {
                    waiting.push(line);
                }
            }
        }
    }
    if stuck == 0 || !waiting.is_empty() {
        checks.push(
            Check::ok(
                Family::Inbox,
                None,
                if waiting.is_empty() {
                    "no mail is waiting".to_owned()
                } else {
                    format!("{} lead(s) have mail waiting", waiting.len())
                },
            )
            .with_details(waiting),
        );
    }
}

fn service(f: &Facts, up: Option<&RedisUp>, checks: &mut Vec<Check>) {
    let subject = Some("relay-watcher unit");
    // A unit that is not running while a heartbeat is live means someone runs the
    // watcher by hand: worth a note, not a failure.
    let runs_anyway = local_beat(f, up).is_some();
    checks.push(match &f.service {
        ServiceState::Active => Check::ok(Family::Service, subject, "the watcher unit is active"),
        ServiceState::Inactive(word) => Check::severe(
            !runs_anyway,
            Family::Service,
            Code::ServiceInactive,
            subject,
            format!("the watcher unit is installed and {word}"),
            "it was stopped or keeps crashing",
            "`systemctl --user start relay-watcher` and `journalctl --user -u relay-watcher`",
        ),
        ServiceState::NotInstalled => Check::warn(
            Family::Service,
            Code::ServiceNotInstalled,
            subject,
            "no watcher unit is installed for this user",
            "deploy/ holds the unit files but this repo does not install them; a watcher started by hand does not need one, but nothing restarts it",
            "install as README.md \"Installing the watcher\" says",
        ),
        ServiceState::Unknown(why) => Check::unknown(
            Family::Service,
            Code::ServiceUnknown,
            subject,
            format!("the watcher unit's state could not be read: {why}"),
            "run `systemctl --user status relay-watcher` (or `launchctl list`) by hand",
        ),
    });
}

#[cfg(test)]
mod tests {
    use crate::doctor::facts::MovesRead;
    use std::{collections::HashSet, time::Duration};

    use relay_core::{
        AgentName, Backlog, Heartbeats, InboxOverview, LeadBacklog, LeaseState, Message, Place,
        health::{HeartbeatError, HeartbeatErrorKind, LastError, WatcherFile},
    };
    use relay_ops::herdr_check::MIN_HERDR_VERSION;

    use super::*;
    use crate::doctor::report::{Report, Status};

    const NOW: i64 = 1_000_000_000_000;

    fn name(s: &str) -> AgentName {
        s.parse().unwrap()
    }

    fn beat(machine: &str, pid: u32, age_ms: i64) -> MachineHeartbeat {
        MachineHeartbeat {
            machine: machine.parse().unwrap(),
            read: HeartbeatRead::Live(Box::new(WatcherHeartbeat {
                machine: machine.parse().unwrap(),
                pid,
                version: "1.2.3".into(),
                started_at_ms: NOW - 3 * HOUR_MS,
                last_tick_ms: NOW - age_ms,
                last_sweep_ms: Some(NOW - age_ms),
                sweep_secs: 15,
                herdr_ok: true,
                herdr_problem: None,
                herdr_blocks_delivery: false,
                redis_ok: true,
                leads_watched: 1,
                otlp: Some(OtlpStatus::Disabled),
                last_error: Some(HeartbeatError {
                    kind: HeartbeatErrorKind::PollFailed,
                    at_ms: NOW - HOUR_MS,
                }),
            })),
        }
    }

    /// A registration last seen at the absolute time `at_ms`.
    fn reg_at(n: &str, machine: &str, pane: Option<&str>, at_ms: i64) -> Registration {
        Registration {
            last_seen_ms: at_ms,
            ..reg(n, machine, pane, 0)
        }
    }

    /// A heartbeat whose last tick was at the absolute time `tick_ms`.
    fn beat_at(machine: &str, pid: u32, tick_ms: i64) -> MachineHeartbeat {
        let mut b = beat(machine, pid, 0);
        if let HeartbeatRead::Live(hb) = &mut b.read {
            hb.last_tick_ms = tick_ms;
            hb.last_sweep_ms = Some(tick_ms);
        }
        b
    }

    fn beats(found: Vec<MachineHeartbeat>) -> Result<Heartbeats, String> {
        Ok(Heartbeats {
            found,
            ..Heartbeats::default()
        })
    }

    fn reg(n: &str, machine: &str, pane: Option<&str>, seen_ago_ms: i64) -> Registration {
        Registration {
            name: name(n),
            machine: machine.parse().unwrap(),
            kind: "claude".into(),
            session_id: None,
            cwd: None,
            herdr_pane: pane.map(|p| p.parse().unwrap()),
            place: pane.map(|_| Place {
                workspace: "EA".into(),
                tab: "1".into(),
                herdr_ids: None,
            }),
            last_seen_ms: NOW - seen_ago_ms,
        }
    }

    fn empty_inbox(n: &str) -> (AgentName, Result<InboxOverview, String>) {
        (
            name(n),
            Ok(InboxOverview {
                len: 0,
                oldest_sent_at_ms: None,
            }),
        )
    }

    fn waiting(n: &str, oldest_ago_ms: i64) -> Vec<(AgentName, Result<InboxOverview, String>)> {
        vec![(
            name(n),
            Ok(InboxOverview {
                len: 2,
                oldest_sent_at_ms: Some(NOW - oldest_ago_ms),
            }),
        )]
    }

    fn lead_backlog(in_flight: usize, dead: usize) -> Backlog {
        Backlog {
            leads: vec![LeadBacklog {
                lead: "a".parse().unwrap(),
                in_flight,
                dead,
                unreadable: false,
            }],
            truncated: false,
        }
    }

    /// Facts for a healthy relay on `hub`: a watcher on hub and on node-a, one
    /// lead on each, herdr supported, nothing waiting or failed.
    fn healthy() -> Facts {
        Facts {
            now_ms: NOW,
            machine: "hub".parse().unwrap(),
            redis_url: "redis://***@127.0.0.1:6379".into(),
            config_error: None,
            watcher_config_problem: None,
            redis: RedisFacts::Up(Box::new(RedisUp {
                rtt: Duration::from_millis(3),
                heartbeats: beats(vec![beat("hub", 42, 3_000), beat("node-a", 7, 9_000)]),
                registrations: Ok((
                    vec![
                        reg("a", "hub", Some("w1:p1"), 60_000),
                        reg("b", "node-a", Some("w2:p1"), 60_000),
                    ],
                    0,
                )),
                inboxes: vec![empty_inbox("a"), empty_inbox("b")],
                backlog: Ok(Backlog::default()),
                leases: Ok(vec![]),
                hook_failures: Ok(vec![]),
                hook_unreadable: 0,
                moves: Ok(MovesRead::default()),
            })),
            watcher_file: FileState::Read {
                file: WatcherFile {
                    pid: 42,
                    started_at_ms: 1,
                    last_ok_tick_ms: Some(NOW - 2_000),
                    last_error: Some(LastError {
                        at_ms: 5,
                        message: "old".into(),
                    }),
                },
                pid_alive: Some(true),
            },
            spool: SpoolState::Empty,
            herdr: HerdrCheck::Ok {
                version: MIN_HERDR_VERSION,
                shape_verified: true,
            },
            panes: Ok(HashSet::from(["w1:p1".parse().unwrap()])),
            service: ServiceState::Active,
            socket: SocketFacts::Herdr,
        }
    }

    fn up(f: &mut Facts) -> &mut RedisUp {
        match &mut f.redis {
            RedisFacts::Up(up) => up,
            _ => unreachable!("healthy() is up"),
        }
    }

    /// Makes the local file's process not run.
    fn pid_gone(f: &mut Facts) {
        if let FileState::Read { pid_alive, .. } = &mut f.watcher_file {
            *pid_alive = Some(false);
        }
    }

    fn record(
        stage: Stage,
        kind: crate::spool::ErrorKind,
        machine: &str,
        at_ms: i64,
    ) -> SpoolRecord {
        SpoolRecord {
            at_ms,
            machine: machine.parse().unwrap(),
            lead: Some("a".into()),
            event: None,
            session_id: None,
            stage,
            error_kind: kind,
            dropped: 0,
            candidate: None,
            messages: Vec::new(),
        }
    }

    fn statuses(f: &Facts) -> Vec<(Code, Status)> {
        evaluate(f)
            .into_iter()
            .filter(|c| c.status != Status::Ok)
            .map(|c| (c.code, c.status))
            .collect()
    }

    fn status_of(f: &Facts, code: Code) -> Status {
        evaluate(f)
            .into_iter()
            .find(|c| c.code == code)
            .unwrap_or_else(|| panic!("{code:?} never fired"))
            .status
    }

    /// Tracing: TC-045-08
    /// ACs: FR-045-AC-8
    ///
    /// A watcher that cannot reach its collector is a warning naming the target,
    /// never a failure; one that is sending or has OTLP off is `ok` and says which.
    #[test]
    fn tc_045_08_doctor_warns_on_a_failing_push_and_states_the_others() {
        let with = |status: Option<OtlpStatus>| {
            let mut f = healthy();
            let mut b = beat("hub", 42, 1_000);
            if let HeartbeatRead::Live(hb) = &mut b.read {
                hb.otlp = status;
            }
            up(&mut f).heartbeats = beats(vec![b, beat("node-a", 7, 1_000)]);
            f
        };
        let failed = with(Some(OtlpStatus::Failing {
            target: "collector:4318".into(),
            reason: "timeout\u{1b}[2J".into(),
            counters: Default::default(),
        }));
        assert_eq!(status_of(&failed, Code::WatcherOtlpFailing), Status::Warn);
        let message = evaluate(&failed)
            .into_iter()
            .find(|c| c.code == Code::WatcherOtlpFailing)
            .unwrap()
            .message;
        assert!(
            message.contains("collector:4318") && message.contains("timeout"),
            "{message}"
        );
        assert!(
            !message.contains('\u{1b}'),
            "a heartbeat is untrusted text: {message:?}"
        );

        for (status, expect) in [
            (
                Some(OtlpStatus::Sending {
                    target: "collector:4318".into(),
                    dropped: 0,
                    counters: Default::default(),
                }),
                "otlp: sending to collector:4318",
            ),
            (
                Some(OtlpStatus::Sending {
                    target: "collector:4318".into(),
                    dropped: 3,
                    counters: Default::default(),
                }),
                "otlp: sending to collector:4318 (3 series left out or folded into lead=other to stay in bounds)",
            ),
            (Some(OtlpStatus::Disabled), "otlp: disabled"),
            (
                Some(OtlpStatus::Ignoring {
                    note: "OTEL_EXPORTER_OTLP_ENDPOINT ignored: https is not supported\u{1b}[2J"
                        .into(),
                }),
                "otlp: disabled (OTEL_EXPORTER_OTLP_ENDPOINT ignored: https is not supported\u{FFFD}[2J)",
            ),
            // An older watcher wrote no member; a newer one wrote a state this
            // binary cannot read. Neither is "disabled".
            (None, "otlp: unknown"),
            (Some(OtlpStatus::Unknown), "otlp: unknown"),
        ] {
            let f = with(status);
            assert!(statuses(&f).is_empty(), "{:?}", statuses(&f));
            let hub = evaluate(&f)
                .into_iter()
                .find(|c| c.subject.as_deref() == Some("hub") && c.check == Family::Watcher)
                .unwrap();
            assert!(hub.details.iter().any(|d| d == expect), "{:?}", hub.details);
        }
    }

    /// The exporter's counters are a detail line of a sending watcher and of a
    /// failing one (`stuck` and `rejected_points` included), and only when not all
    /// zero.
    ///
    /// Tracing: TC-045-21
    /// ACs: FR-045-AC-21
    #[test]
    fn tc_045_21_doctor_shows_the_exporter_counters_and_a_stuck_or_rejecting_push() {
        let counters = relay_core::health::OtlpCounters {
            sent: 12,
            failed: 3,
            dropped_snapshots: 1,
            rejected_points: 9,
        };
        let with = |status: OtlpStatus| {
            let mut f = healthy();
            let mut b = beat("hub", 42, 1_000);
            if let HeartbeatRead::Live(hb) = &mut b.read {
                hb.otlp = Some(status);
            }
            up(&mut f).heartbeats = beats(vec![b, beat("node-a", 7, 1_000)]);
            f
        };
        let hub = |f: &Facts| {
            evaluate(f)
                .into_iter()
                .find(|c| c.subject.as_deref() == Some("hub") && c.check == Family::Watcher)
                .unwrap()
        };
        let sending = hub(&with(OtlpStatus::Sending {
            target: "c:1".into(),
            dropped: 0,
            counters,
        }));
        assert!(
            sending.details.iter().any(|d| d
                == "otlp counters: 12 push(es) sent, 3 failed, 1 snapshot(s) dropped, 9 point(s) rejected"),
            "{:?}",
            sending.details
        );
        for reason in ["stuck", "rejected_points"] {
            let f = with(OtlpStatus::Failing {
                target: "c:1".into(),
                reason: reason.into(),
                counters,
            });
            assert_eq!(status_of(&f, Code::WatcherOtlpFailing), Status::Warn);
            let check = hub(&f);
            assert!(check.message.contains(reason), "{}", check.message);
            assert!(
                check.details.iter().any(|d| d.contains("12 push(es) sent")),
                "{:?}",
                check.details
            );
        }
        let quiet = hub(&with(OtlpStatus::Sending {
            target: "c:1".into(),
            dropped: 0,
            counters: Default::default(),
        }));
        assert!(
            !quiet.details.iter().any(|d| d.contains("counters")),
            "{:?}",
            quiet.details
        );
    }

    /// Tracing: TC-029-01
    /// ACs: FR-029-AC-1
    #[test]
    fn tc_029_01_a_healthy_relay_has_no_failing_warning_or_unknown_check() {
        let f = healthy();
        let report = Report::new(evaluate(&f), true);
        assert!(report.ok, "even strict");
        assert_eq!(statuses(&f), Vec::new());
        let subjects: Vec<_> = report
            .checks
            .iter()
            .filter(|c| c.check == Family::Watcher)
            .filter_map(|c| c.subject.clone())
            .collect();
        assert_eq!(subjects, ["hub", "node-a"]);
        // A healthy heartbeat still says what it knows.
        let hub = report
            .checks
            .iter()
            .find(|c| c.subject.as_deref() == Some("hub") && c.check == Family::Watcher)
            .unwrap();
        assert!(hub.message.contains("up 3h"), "{}", hub.message);
        assert!(hub.message.contains("last sweep 3s ago"), "{}", hub.message);
        assert!(hub.details[0].contains("poll_failed"), "{:?}", hub.details);
        assert_eq!(hub.details[1], "otlp: disabled", "{:?}", hub.details);
    }

    /// Tracing: TC-029-02
    /// ACs: FR-029-AC-2
    #[test]
    fn tc_029_02_a_machine_with_a_recent_lead_and_no_heartbeat_fails_and_is_named() {
        let mut f = healthy();
        up(&mut f).heartbeats = beats(vec![beat("hub", 42, 3_000)]);
        let checks = evaluate(&f);
        let missing = checks
            .iter()
            .find(|c| c.code == Code::WatcherHeartbeatMissing)
            .expect("node-a has a lead seen a minute ago and no heartbeat");
        assert_eq!(missing.status, Status::Fail);
        assert_eq!(missing.subject.as_deref(), Some("node-a"));
        assert!(
            missing.message.starts_with(
                "no watcher heartbeat from node-a (watcher stopped, or older than the heartbeat)"
            ),
            "{}",
            missing.message
        );
        assert!(!Report::new(checks, false).ok);
    }

    /// Exit 1 has to mean something: a machine whose leads were last seen hours
    /// ago (a sleeping laptop) is a warning, and one whose leads are all a day
    /// old is not expected to run a watcher at all.
    #[test]
    fn a_missing_heartbeat_fails_only_for_a_machine_with_a_recent_lead() {
        let mut f = healthy();
        up(&mut f).heartbeats = beats(vec![beat("hub", 42, 3_000)]);
        up(&mut f).registrations = Ok((
            vec![
                reg("a", "hub", Some("w1:p1"), 60_000),
                reg("b", "node-a", Some("w2:p1"), 5 * HOUR_MS),
            ],
            0,
        ));
        assert_eq!(status_of(&f, Code::WatcherHeartbeatMissing), Status::Warn);
        assert!(Report::new(evaluate(&f), false).ok, "a warning exits 0");
        assert!(!Report::new(evaluate(&f), true).ok, "--strict fails it");

        up(&mut f).registrations = Ok((vec![reg("b", "node-a", Some("w2:p1"), 3 * DAY_MS)], 0));
        let found: Vec<_> = statuses(&f)
            .into_iter()
            .filter(|(code, _)| *code == Code::WatcherHeartbeatMissing)
            .collect();
        assert_eq!(found, Vec::new());
    }

    /// A watcher that visibly runs here (a live pid, a fresh state file) and
    /// publishes nothing is older than the heartbeat: a warning that says so, not
    /// a missing watcher.
    #[test]
    fn a_running_local_watcher_without_a_heartbeat_is_older_than_the_heartbeat() {
        let mut f = healthy();
        up(&mut f).heartbeats = beats(vec![beat("node-a", 7, 1_000)]);
        let checks = evaluate(&f);
        let local = checks
            .iter()
            .find(|c| {
                c.code == Code::WatcherHeartbeatMissing && c.subject.as_deref() == Some("hub")
            })
            .unwrap();
        assert_eq!(local.status, Status::Warn);
        assert!(
            local.message.contains("older than the heartbeat"),
            "{}",
            local.message
        );
        // Without a live process behind the file it is missing, and it fails.
        pid_gone(&mut f);
        assert_eq!(status_of(&f, Code::WatcherHeartbeatMissing), Status::Fail);
    }

    /// A watcher whose ticks stopped is found by the age of its last tick, before
    /// its key expires: at 3 sweeps the key is gone and it would read as missing.
    #[test]
    fn a_wedged_watcher_is_found_by_its_last_tick_before_the_key_expires() {
        let mut f = healthy();
        // Sweep 15 s: expiry 45 s. 41 s old is past two sweeps and ten seconds.
        up(&mut f).heartbeats = beats(vec![beat("hub", 42, 41_000), beat("node-a", 7, 1_000)]);
        let stale = evaluate(&f)
            .into_iter()
            .find(|c| c.code == Code::WatcherHeartbeatStale)
            .unwrap();
        assert_eq!(stale.status, Status::Fail);
        assert!(stale.cause.as_deref().unwrap().contains("stopped ticking"));
        // 30 s is inside two sweeps plus the slack.
        up(&mut f).heartbeats = beats(vec![beat("hub", 42, 30_000), beat("node-a", 7, 1_000)]);
        assert_eq!(statuses(&f), Vec::new());
    }

    /// Tracing: TC-029-03
    /// ACs: FR-029-AC-3
    #[test]
    fn tc_029_03_with_redis_down_the_local_checks_still_run_and_redis_fails() {
        let mut f = healthy();
        f.redis = RedisFacts::Down(LinkFault::Refused, "Connection refused".into());
        let checks = evaluate(&f);
        assert!(checks.iter().any(|c| c.code == Code::RedisUnreachable));
        assert!(
            checks
                .iter()
                .any(|c| c.check == Family::Herdr && c.status == Status::Ok)
        );
        assert!(checks.iter().any(|c| c.check == Family::Service));
        assert!(checks.iter().any(|c| c.check == Family::WatcherFile));
        assert!(
            !checks.iter().any(|c| c.check == Family::Watcher),
            "no heartbeat claim is made without the bus"
        );
        assert!(!Report::new(checks, false).ok);
    }

    /// Tracing: TC-029-13
    /// ACs: FR-029-AC-13
    #[test]
    fn tc_029_13_an_unreachable_bus_names_the_layer_that_failed() {
        let hint = |fault| {
            let mut f = healthy();
            f.redis = RedisFacts::Down(fault, "e".into());
            let checks = evaluate(&f);
            let c = checks
                .into_iter()
                .find(|c| c.code == Code::RedisUnreachable)
                .expect("redis_unreachable fires");
            assert_eq!(c.status, Status::Fail);
            (c.cause.unwrap(), c.fix.unwrap())
        };
        for (fault, cause_has, fix_has) in [
            (LinkFault::Auth, "user or password", "relay-admin add-user"),
            (
                LinkFault::Refused,
                "nothing listens",
                "relay-redis-proxy.socket",
            ),
            (
                LinkFault::Dropped,
                "dropped it",
                "kubectl -n relay get pods",
            ),
            (LinkFault::TimedOut, "WireGuard link", "latest handshake"),
            (LinkFault::NoRoute, "no route", "wg-quick@relay"),
            (LinkFault::Other, "URL", "config.toml"),
        ] {
            let (cause, fix) = hint(fault);
            assert!(cause.contains(cause_has), "{fault:?}: {cause}");
            assert!(fix.contains(fix_has), "{fault:?}: {fix}");
            assert!(!cause.contains("SSH") && !fix.contains("ssh"), "{fault:?}");
        }
    }

    /// One row per code and severity: the facts that make it fire. A code that
    /// cannot be provoked is dead, and a severity that changes is a change to the
    /// catalogue in FR-029.
    #[test]
    fn every_code_fires_with_its_documented_severity_a_cause_and_a_fix() {
        type Mutate = fn(&mut Facts);
        use crate::spool::ErrorKind as K;
        use Status::{Fail, Unknown, Warn};
        let old_lead = |f: &mut Facts| {
            up(f).registrations = Ok((vec![reg("a", "hub", Some("w1:p1"), 6 * HOUR_MS)], 0));
        };
        let _ = old_lead;
        let cases: Vec<(Code, Status, Mutate)> = vec![
            (Code::WatcherOtlpConfigInvalid, Warn, |f| {
                f.watcher_config_problem = Some("otlp_endpoint: not a url".into());
            }),
            (Code::ConfigInvalid, Fail, |f| {
                f.config_error = Some("bad".into())
            }),
            (Code::RedisUnreachable, Fail, |f| {
                f.redis = RedisFacts::Down(LinkFault::Other, "x".into())
            }),
            (Code::RedisSlow, Warn, |f| {
                up(f).rtt = Duration::from_millis(2_500)
            }),
            (Code::RedisReadFailed, Fail, |f| {
                up(f).backlog = Err("WRONGTYPE".into())
            }),
            (Code::WatcherHeartbeatMissing, Fail, |f| {
                up(f).heartbeats = beats(vec![]);
                pid_gone(f);
            }),
            (Code::WatcherHeartbeatUnreadable, Warn, |f| {
                up(f).heartbeats = beats(vec![
                    MachineHeartbeat {
                        machine: "hub".parse().unwrap(),
                        read: HeartbeatRead::Undecodable,
                    },
                    beat("node-a", 7, 1_000),
                ]);
            }),
            (Code::WatcherHeartbeatStale, Fail, |f| {
                up(f).heartbeats =
                    beats(vec![beat("hub", 42, 5 * 60_000), beat("node-a", 7, 1_000)]);
            }),
            (Code::HeartbeatScanTruncated, Warn, |f| {
                if let RedisFacts::Up(up) = &mut f.redis
                    && let Ok(h) = &mut up.heartbeats
                {
                    h.truncated = Some(relay_core::ScanCut::Calls);
                }
            }),
            (Code::WatcherHerdrProblem, Warn, |f| {
                let mut b = beat("hub", 42, 1_000);
                if let HeartbeatRead::Live(hb) = &mut b.read {
                    hb.herdr_ok = false;
                    hb.herdr_problem = Some("herdr 0.8.0 is too old".into());
                }
                up(f).heartbeats = beats(vec![b, beat("node-a", 7, 1_000)]);
            }),
            (Code::WatcherHerdrProblem, Fail, |f| {
                let mut b = beat("hub", 42, 1_000);
                if let HeartbeatRead::Live(hb) = &mut b.read {
                    hb.herdr_ok = false;
                    hb.herdr_blocks_delivery = true;
                    hb.herdr_problem = Some("no agent_session".into());
                }
                up(f).heartbeats = beats(vec![b, beat("node-a", 7, 1_000)]);
            }),
            (Code::WatcherOtlpFailing, Warn, |f| {
                let mut b = beat("hub", 42, 1_000);
                if let HeartbeatRead::Live(hb) = &mut b.read {
                    hb.otlp = Some(OtlpStatus::Failing {
                        target: "collector:4318".into(),
                        reason: "timeout".into(),
                        counters: Default::default(),
                    });
                }
                up(f).heartbeats = beats(vec![b, beat("node-a", 7, 1_000)]);
            }),
            (Code::WatcherFileMissing, Warn, |f| {
                f.watcher_file = FileState::Missing
            }),
            (Code::WatcherFileMissing, Fail, |f| {
                f.watcher_file = FileState::Missing;
                up(f).heartbeats = beats(vec![beat("node-a", 7, 1_000)]);
            }),
            (Code::WatcherFileUnlocated, Unknown, |f| {
                f.watcher_file = FileState::NoPath
            }),
            (Code::WatcherFileUnreadable, Warn, |f| {
                f.watcher_file = FileState::Unreadable("bad json".into());
            }),
            (Code::WatcherFileStale, Fail, |f| {
                if let FileState::Read { file, .. } = &mut f.watcher_file {
                    file.last_ok_tick_ms = Some(NOW - 10 * 60_000);
                }
            }),
            (Code::WatcherPidGone, Fail, pid_gone),
            (Code::WatcherFileMismatch, Warn, |f| {
                if let FileState::Read { file, .. } = &mut f.watcher_file {
                    file.pid = 99;
                }
            }),
            (Code::HookSpoolPending, Fail, |f| {
                f.spool = SpoolState::Pending {
                    bytes: 10,
                    modified_ms: None,
                    records: vec![record(Stage::Connect, K::Connection, "hub", NOW - 60_000)],
                    undecodable: 0,
                };
            }),
            (Code::HookSpoolPending, Warn, |f| {
                f.spool = SpoolState::Pending {
                    bytes: 10,
                    modified_ms: None,
                    records: vec![record(
                        Stage::Connect,
                        K::Connection,
                        "hub",
                        NOW - 3 * HOUR_MS,
                    )],
                    undecodable: 0,
                };
            }),
            (Code::HookSpoolUnlocated, Unknown, |f| {
                f.spool = SpoolState::NoPath
            }),
            (Code::HookSpoolUnreadable, Warn, |f| {
                f.spool = SpoolState::Unreadable("denied".into());
            }),
            (Code::HookFailuresRecorded, Fail, |f| {
                up(f).hook_failures = Ok(vec![BusFailure {
                    machine: "hub".parse().unwrap(),
                    record: Some(record(Stage::Pop, K::Timeout, "hub", NOW - 60_000)),
                }]);
            }),
            (Code::HookFailuresRecorded, Warn, |f| {
                up(f).hook_failures = Ok(vec![BusFailure {
                    machine: "hub".parse().unwrap(),
                    record: Some(record(Stage::Pop, K::Timeout, "hub", NOW - 5 * HOUR_MS)),
                }]);
            }),
            (Code::HerdrMissing, Fail, |f| {
                f.herdr = HerdrCheck::Missing {
                    reason: "no such file".into(),
                };
            }),
            (Code::HerdrUnsupportedVersion, Warn, |f| {
                f.herdr = HerdrCheck::Unsupported {
                    found: "0.8.0".parse().unwrap(),
                    required: MIN_HERDR_VERSION,
                };
            }),
            (Code::HerdrShapeMismatch, Fail, |f| {
                f.herdr = HerdrCheck::ShapeMismatch {
                    command: "agent get",
                    detail: "missing field `state_change_seq`".into(),
                };
            }),
            (Code::HerdrVersionUnreadable, Warn, |f| {
                f.herdr = HerdrCheck::VersionUnreadable {
                    output: "hello".into(),
                };
            }),
            (Code::HerdrUnchecked, Unknown, |f| {
                f.herdr = HerdrCheck::Failed {
                    reason: "timed out".into(),
                };
            }),
            (Code::RegistrationStale, Warn, |f| {
                up(f).registrations = Ok((vec![reg("b", "node-a", None, 2 * DAY_MS)], 0));
            }),
            (Code::RegistrationNoPlace, Warn, |f| {
                let mut r = reg("a", "hub", Some("w1:p1"), 1_000);
                r.place = None;
                up(f).registrations = Ok((vec![r], 0));
            }),
            (Code::RegistrationPaneGone, Warn, |f| {
                up(f).registrations = Ok((vec![reg("a", "hub", Some("w9:p9"), 2 * HOUR_MS)], 0));
            }),
            (Code::RegistrationUndecodable, Warn, |f| {
                up(f).registrations = Ok((vec![], 2))
            }),
            (Code::PanesUnchecked, Unknown, |f| {
                f.panes = Err("timed out".into())
            }),
            (Code::DeadLettersPresent, Warn, |f| {
                up(f).backlog = Ok(lead_backlog(0, 3))
            }),
            (Code::InflightOverdue, Warn, |f| {
                up(f).backlog = Ok(lead_backlog(2, 0));
                up(f).leases = Ok(vec![LeaseState {
                    lead: "a".parse().unwrap(),
                    entries: 2,
                    batches: 1,
                    overdue: 2,
                    most_overdue_ms: 10 * 60_000,
                    rows_cut: false,
                }]);
            }),
            (Code::BacklogScanTruncated, Warn, |f| {
                up(f).backlog = Ok(Backlog {
                    leads: vec![],
                    truncated: true,
                });
            }),
            (Code::InboxStuck, Fail, |f| {
                up(f).inboxes = waiting("a", 10 * 60_000);
                up(f).heartbeats = beats(vec![beat("node-a", 7, 1_000)]);
                pid_gone(f);
            }),
            (Code::InboxStuck, Warn, |f| {
                up(f).inboxes = waiting("a", 10 * 60_000);
            }),
            (Code::ServiceNotInstalled, Warn, |f| {
                f.service = ServiceState::NotInstalled
            }),
            (Code::ServiceInactive, Fail, |f| {
                f.service = ServiceState::Inactive("failed".into());
                up(f).heartbeats = beats(vec![beat("node-a", 7, 1_000)]);
                pid_gone(f);
            }),
            (Code::ServiceInactive, Warn, |f| {
                f.service = ServiceState::Inactive("failed".into());
            }),
            (Code::ServiceUnknown, Unknown, |f| {
                f.service = ServiceState::Unknown("no bus".into())
            }),
        ];
        for (code, want, mutate) in cases {
            let mut f = healthy();
            mutate(&mut f);
            let checks = evaluate(&f);
            let hit = checks
                .iter()
                .find(|c| c.code == code && c.status == want)
                .unwrap_or_else(|| panic!("{code:?} never fired as {want:?}: {:?}", statuses(&f)));
            if want != Unknown {
                assert!(
                    hit.cause.is_some() && hit.fix.is_some(),
                    "{code:?} names no cause or fix"
                );
            }
            let report = Report::new(checks.clone(), false);
            let any_fail = checks.iter().any(|c| c.status == Fail);
            assert_eq!(report.ok, !any_fail, "{code:?}");
            let strict = Report::new(checks.clone(), true);
            assert_eq!(
                strict.ok,
                !any_fail && !checks.iter().any(|c| c.status == Warn),
                "{code:?} strict"
            );
        }
    }

    /// Unknown is reported and never fails the exit, strict or not.
    #[test]
    fn unknown_is_reported_and_never_fails_the_exit() {
        let mut f = healthy();
        f.service = ServiceState::Unknown("no bus".into());
        f.herdr = HerdrCheck::Failed {
            reason: "timed out".into(),
        };
        f.panes = Err("timed out".into());
        f.watcher_file = FileState::NoPath;
        f.spool = SpoolState::NoPath;
        let checks = evaluate(&f);
        for code in [
            Code::ServiceUnknown,
            Code::HerdrUnchecked,
            Code::PanesUnchecked,
            Code::WatcherFileUnlocated,
            Code::HookSpoolUnlocated,
        ] {
            let hit = checks.iter().find(|c| c.code == code).unwrap();
            assert_eq!(hit.status, Status::Unknown, "{code:?}");
        }
        assert!(Report::new(checks.clone(), false).ok);
        assert!(Report::new(checks, true).ok);
    }

    /// A pane that has been gone under an hour is not reported at all.
    #[test]
    fn a_pane_gone_for_less_than_the_grace_is_not_reported() {
        let mut f = healthy();
        up(&mut f).registrations = Ok((vec![reg("a", "hub", Some("w9:p9"), 20 * 60_000)], 0));
        assert_eq!(statuses(&f), Vec::new());
    }

    /// Dead letters warn with the hint that exists, and are an exit 0 by default.
    #[test]
    fn dead_letters_warn_with_the_inbox_dead_hint() {
        let mut f = healthy();
        up(&mut f).backlog = Ok(lead_backlog(0, 3));
        let checks = evaluate(&f);
        let dead = checks
            .iter()
            .find(|c| c.code == Code::DeadLettersPresent)
            .unwrap();
        assert_eq!(dead.status, Status::Warn);
        assert!(
            dead.fix
                .as_deref()
                .unwrap()
                .contains("agent-msg inbox --dead")
        );
        assert!(Report::new(checks, false).ok);
    }

    /// In-flight is counted in entries and, where the rows say, in batches; an
    /// overdue lease is named, and one within the grace is not.
    #[test]
    fn in_flight_counts_entries_and_batches_and_flags_overdue_leases() {
        let mut f = healthy();
        up(&mut f).backlog = Ok(lead_backlog(3, 0));
        let lease = |overdue: usize, most_overdue_ms: i64| {
            Ok(vec![LeaseState {
                lead: "a".parse().unwrap(),
                entries: 3,
                batches: 1,
                overdue,
                most_overdue_ms,
                rows_cut: false,
            }])
        };
        up(&mut f).leases = lease(0, 0);
        let ok = evaluate(&f)
            .into_iter()
            .find(|c| c.check == Family::InFlight)
            .unwrap();
        assert_eq!(ok.status, Status::Ok);
        assert_eq!(ok.details, ["a: 3 entr(y/ies) in 1 batch(es), 0 overdue"]);
        up(&mut f).leases = lease(3, 30_000);
        assert_eq!(statuses(&f), Vec::new(), "30 s past is inside the grace");
        up(&mut f).leases = lease(3, 20 * 60_000);
        assert_eq!(statuses(&f), [(Code::InflightOverdue, Status::Warn)]);
    }

    #[test]
    fn a_fresh_waiting_message_is_information_not_a_failure() {
        let mut f = healthy();
        up(&mut f).inboxes = waiting("a", 4_000);
        let inbox = evaluate(&f)
            .into_iter()
            .find(|c| c.check == Family::Inbox)
            .unwrap();
        assert_eq!(inbox.status, Status::Ok);
        assert_eq!(inbox.details, ["a: 2 waiting, oldest 4s"]);
    }

    /// The spool's `dropped` count (records dropped for the bound, lines removed
    /// unreadable) is shown, not only carried on the record.
    #[test]
    fn the_spool_dropped_count_is_shown() {
        use crate::spool::ErrorKind as K;
        let mut f = healthy();
        let mut flush = record(Stage::Decode, K::Corrupt, "hub", NOW - 1_000);
        flush.dropped = 3;
        up(&mut f).hook_failures = Ok(vec![BusFailure {
            machine: flush.machine.clone(),
            record: Some(flush),
        }]);
        let check = evaluate(&f)
            .into_iter()
            .find(|c| c.code == Code::HookFailuresRecorded)
            .unwrap();
        assert!(
            check
                .details
                .iter()
                .any(|d| d.starts_with("3 spooled line(s)")),
            "{:?}",
            check.details
        );
    }

    /// Tracing: TC-043-07
    /// ACs: FR-043-AC-7
    #[test]
    fn tc_043_07_recent_moves_are_information_never_a_failure_and_carry_neutralised_text() {
        use relay_core::{Moves, Place};
        let mut f = healthy();
        let place = |workspace: &str| {
            Some(Place {
                workspace: workspace.into(),
                tab: "1".into(),
                herdr_ids: None,
            })
        };
        let event = |kind, at_ms, old: &str, new: &str| MoveEvent {
            lead: name("lead-a"),
            machine: "hub".parse().unwrap(),
            kind,
            session_id: None,
            previous_pane: (kind == MoveKind::Relink).then(|| "w1:p3".parse().unwrap()),
            pane: Some("w2:p2".parse().unwrap()),
            old_place: place(old),
            new_place: place(new),
            at_ms,
        };
        up(&mut f).moves = Ok(MovesRead {
            moves: Moves {
                entries: vec![
                    event(MoveKind::Relink, NOW - 3 * HOUR_MS, "wsA", "ws\u{1b}[31mB"),
                    event(MoveKind::Rename, NOW - 2 * DAY_MS, "old", "older"),
                ],
                unreadable: 1,
            },
            more: false,
        });
        let checks = evaluate(&f);
        let moves = checks.iter().find(|c| c.check == Family::Moves).unwrap();
        assert_eq!(moves.status, Status::Ok, "information, not a failure");
        assert!(
            moves.message.starts_with("1 pane move(s)"),
            "{}",
            moves.message
        );
        assert_eq!(moves.details.len(), 2, "{:?}", moves.details);
        assert!(
            moves.details[0].contains("lead-a moved pane w1:p3 to w2:p2")
                && moves.details[0].contains("hub:wsA:1 -> hub:ws"),
            "{:?}",
            moves.details
        );
        assert!(
            !moves.details[0].contains('\u{1b}'),
            "the label's escape code is neutralised"
        );
        assert!(moves.details[1].contains("do not decode"));
        assert!(
            checks.iter().all(|c| c.status != Status::Fail),
            "nothing fails"
        );
        // Nothing recorded: one line saying so.
        up(&mut f).moves = Ok(MovesRead::default());
        let quiet = evaluate(&f)
            .into_iter()
            .find(|c| c.check == Family::Moves)
            .unwrap();
        assert!(quiet.message.contains("no pane moves"));

        // More than were read: the count says "at least", never stops silently.
        up(&mut f).moves = Ok(MovesRead {
            moves: Moves {
                entries: vec![event(MoveKind::Rename, NOW - HOUR_MS, "a", "b"); 3],
                unreadable: 0,
            },
            more: true,
        });
        let busy = evaluate(&f)
            .into_iter()
            .find(|c| c.check == Family::Moves)
            .unwrap();
        assert!(
            busy.message.starts_with("at least 3 pane move(s)"),
            "{}",
            busy.message
        );

        // A label of any length is bounded in the line.
        let long = "x".repeat(5_000);
        up(&mut f).moves = Ok(MovesRead {
            moves: Moves {
                entries: vec![event(MoveKind::Rename, NOW - HOUR_MS, &long, "b")],
                unreadable: 0,
            },
            more: false,
        });
        let bounded = evaluate(&f)
            .into_iter()
            .find(|c| c.check == Family::Moves)
            .unwrap();
        assert!(
            bounded.details[0].len() < 200 && bounded.details[0].contains("..."),
            "{:?}",
            bounded.details
        );
    }

    /// Tracing: TC-043-07
    /// ACs: FR-043-AC-7
    #[test]
    fn tc_043_07_a_moves_stream_that_cannot_be_read_is_unknown_and_never_fails_the_exit() {
        let mut f = healthy();
        up(&mut f).moves = Err("WRONGTYPE".into());
        let checks = evaluate(&f);
        let moves = checks
            .iter()
            .find(|c| c.check == Family::Moves)
            .expect("a moves check is reported");
        assert_eq!(
            (moves.status, moves.code),
            (Status::Unknown, Code::MovesUnreadable)
        );
        assert!(
            checks.iter().all(|c| c.status != Status::Fail),
            "nothing fails: {checks:?}"
        );
    }

    #[test]
    fn hook_failures_are_grouped_by_stage_and_kind_and_old_ones_are_left_out() {
        use crate::spool::ErrorKind as K;
        let mut f = healthy();
        let mut lost = record(Stage::RequeueFailed, K::Connection, "hub", NOW - 5_000);
        lost.messages = vec![Message {
            from: name("x"),
            to: "a".parse().unwrap(),
            text: "line one\n\u{1b}[31mred".into(),
            sent_at_ms: 1,
        }];
        let entries = [
            record(Stage::Pop, K::Timeout, "hub", NOW - 1_000),
            record(Stage::Pop, K::Timeout, "node-a", NOW - 2_000),
            record(Stage::Connect, K::Connection, "hub", NOW - 3_000),
            record(Stage::Connect, K::Connection, "hub", NOW - 2 * DAY_MS),
            lost,
        ];
        up(&mut f).hook_failures = Ok(entries
            .into_iter()
            .map(|r| BusFailure {
                machine: r.machine.clone(),
                record: Some(r),
            })
            .collect());
        let check = evaluate(&f)
            .into_iter()
            .find(|c| c.code == Code::HookFailuresRecorded)
            .unwrap();
        assert!(
            check.message.starts_with("4 hook failure(s)"),
            "{}",
            check.message
        );
        assert!(
            check
                .details
                .contains(&"pop / timeout: 2 (hub, node-a)".to_owned())
        );
        assert!(
            check
                .details
                .contains(&"connect / connection: 1 (hub)".to_owned())
        );
        let lost_line = check
            .details
            .iter()
            .find(|d| d.starts_with("requeue_failed on"))
            .unwrap();
        assert!(lost_line.contains("lost 1 message(s)"));
        assert!(!lost_line.contains('\u{1b}'), "message text is neutralised");
    }

    /// A herdr problem or spool text with control characters cannot reach the
    /// terminal through any check. (A machine name cannot carry one: `MachineName`
    /// refuses them at decode.)
    #[test]
    fn nothing_a_check_prints_can_carry_a_control_character() {
        let mut f = healthy();
        f.herdr = HerdrCheck::Missing {
            reason: "\u{1b}]0;pwned\u{7}".into(),
        };
        up(&mut f).heartbeats = beats(vec![beat("hub", 1, 1_000)]);
        let mut out = Vec::new();
        Report::new(evaluate(&f), false)
            .write_text(&mut out)
            .unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(
            !text.contains('\u{1b}') && !text.contains('\u{7}'),
            "{text:?}"
        );
    }

    /// The dead watcher's own alarm must not fade: `last_seen_ms` is refreshed by
    /// the watcher, so an hour after it dies every idle lead looks long gone.
    /// The non-expiring last-tick marker is what a dead watcher leaves. Evaluated
    /// at an injected clock: half an hour, three hours and a day and a bit after
    /// the last tick, with the leads' stamps frozen at that tick.
    #[test]
    fn a_dead_watchers_alarm_holds_for_the_prune_horizon_then_becomes_a_warning() {
        let last_tick = NOW;
        let mut f = healthy();
        {
            let u = up(&mut f);
            u.registrations = Ok((
                vec![
                    reg("a", "hub", Some("w1:p1"), 60_000),
                    reg_at("b", "node-a", Some("w2:p1"), last_tick),
                ],
                0,
            ));
            u.heartbeats = Ok(Heartbeats {
                found: vec![beat("hub", 42, 3_000)],
                truncated: None,
                last_ticks: [
                    ("node-a".parse().unwrap(), last_tick),
                    ("hub".parse().unwrap(), NOW),
                ]
                .into(),
                unreadable: 0,
            });
        }
        let node_a_missing = |f: &Facts| {
            evaluate(f)
                .into_iter()
                .find(|c| {
                    c.code == Code::WatcherHeartbeatMissing
                        && c.subject.as_deref() == Some("node-a")
                })
                .map(|c| (c.status, c.message))
        };
        for (after_ms, want) in [
            (30 * 60_000, Status::Fail),
            (3 * HOUR_MS, Status::Fail),
            (23 * HOUR_MS, Status::Fail),
            (25 * HOUR_MS, Status::Warn),
        ] {
            let mut later = f.clone();
            later.now_ms = NOW + after_ms;
            // The local watcher is alive the whole time: keep its side fresh.
            if let RedisFacts::Up(u) = &mut later.redis {
                u.heartbeats = Ok(Heartbeats {
                    found: vec![beat_at("hub", 42, later.now_ms - 3_000)],
                    truncated: None,
                    last_ticks: [("node-a".parse().unwrap(), last_tick)].into(),
                    unreadable: 0,
                });
                u.registrations = Ok((
                    vec![
                        Registration {
                            last_seen_ms: later.now_ms - 60_000,
                            ..reg("a", "hub", Some("w1:p1"), 0)
                        },
                        reg_at("b", "node-a", Some("w2:p1"), last_tick),
                    ],
                    0,
                ));
            }
            if let FileState::Read { file, .. } = &mut later.watcher_file {
                file.last_ok_tick_ms = Some(later.now_ms - 2_000);
            }
            let (status, message) = node_a_missing(&later)
                .unwrap_or_else(|| panic!("node-a not reported {after_ms} ms after its last tick"));
            assert_eq!(status, want, "{after_ms} ms after: {message}");
            assert!(message.contains("its last tick was"), "{message}");
        }
    }

    /// Mail a fresh watcher has not delivered is a lead busy in a long call: a
    /// warning. It fails when nothing would wake the lead: its watcher is wedged
    /// or missing where one is expected.
    #[test]
    fn a_stuck_inbox_is_a_warning_under_a_fresh_watcher_and_a_failure_without_one() {
        let mut f = healthy();
        up(&mut f).inboxes = waiting("a", 30 * 60_000);
        assert_eq!(
            status_of(&f, Code::InboxStuck),
            Status::Warn,
            "fresh heartbeat"
        );

        // Wedged: the heartbeat's last tick is old though the key has not expired.
        up(&mut f).heartbeats = beats(vec![beat("hub", 42, 41_000), beat("node-a", 7, 1_000)]);
        assert_eq!(
            status_of(&f, Code::InboxStuck),
            Status::Fail,
            "wedged watcher"
        );

        // Missing where a watcher is expected: a recent lead and no heartbeat.
        up(&mut f).heartbeats = beats(vec![beat("node-a", 7, 1_000)]);
        assert_eq!(status_of(&f, Code::InboxStuck), Status::Fail, "no watcher");

        // Missing on a machine silent for over the horizon: a warning.
        up(&mut f).registrations = Ok((vec![reg("a", "hub", Some("w1:p1"), 3 * DAY_MS)], 0));
        assert_eq!(status_of(&f, Code::InboxStuck), Status::Warn, "long gone");
    }

    /// A spool holding only lines that do not decode still has an age: the file's.
    #[test]
    fn an_undecodable_spool_is_a_failure_only_while_it_is_being_written() {
        let mut f = healthy();
        let spool = |modified_ms| SpoolState::Pending {
            bytes: 12,
            records: vec![],
            undecodable: 2,
            modified_ms,
        };
        f.spool = spool(Some(NOW - 60_000));
        assert_eq!(status_of(&f, Code::HookSpoolPending), Status::Fail);
        f.spool = spool(Some(NOW - 5 * HOUR_MS));
        assert_eq!(status_of(&f, Code::HookSpoolPending), Status::Warn);
        f.spool = spool(None);
        assert_eq!(status_of(&f, Code::HookSpoolPending), Status::Warn);
    }

    /// The truncation warning names the bound that was hit.
    #[test]
    fn the_heartbeat_scan_warning_says_which_bound_was_hit() {
        for (cut, needle) in [
            (relay_core::ScanCut::Keys, "key bound"),
            (relay_core::ScanCut::Calls, "call bound"),
            (relay_core::ScanCut::Deadline, "time budget"),
        ] {
            let mut f = healthy();
            if let RedisFacts::Up(u) = &mut f.redis
                && let Ok(h) = &mut u.heartbeats
            {
                h.truncated = Some(cut);
            }
            let message = evaluate(&f)
                .into_iter()
                .find(|c| c.code == Code::HeartbeatScanTruncated)
                .unwrap()
                .message;
            assert!(message.contains(needle), "{cut:?}: {message}");
        }
    }

    fn socket_status(
        held: &[&str],
        notes: &[&str],
        reply: bool,
    ) -> relay_core::socket_status::SocketStatus {
        relay_core::socket_status::SocketStatus {
            pid: 42,
            updated_at_ms: NOW,
            reply_socket: reply.then(|| "/run/user/1/cc-socks/42.sock".to_owned()),
            leads: vec![relay_core::socket_status::LeadSocket {
                lead: "a".into(),
                pid: 9,
                socket: "/run/user/1/cc-socks/9.sock".into(),
                version: "2.1.283".into(),
            }],
            held: held.iter().map(|s| (*s).to_owned()).collect(),
            notes: notes.iter().map(|s| (*s).to_owned()).collect(),
            ..relay_core::socket_status::SocketStatus::default()
        }
    }

    fn socket_checks(status: relay_core::socket_status::SocketStatus) -> Vec<Check> {
        let mut f = healthy();
        f.socket = SocketFacts::Socket(vec![SocketState::Read {
            status,
            pid_alive: Some(true),
        }]);
        evaluate(&f)
            .into_iter()
            .filter(|c| c.check == Family::Socket)
            .collect()
    }

    /// Tracing: TC-046-10
    /// ACs: FR-046-AC-10
    ///
    /// Doctor shows the version the wire format was captured on, and says so when a
    /// resolved Claude Code is another version; neither is ever a failure.
    #[test]
    fn tc_046_10_doctor_shows_the_captured_version_and_flags_an_unverified_one() {
        let found = socket_checks(socket_status(&[], &[], true));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].status, Status::Ok);
        assert!(
            found[0]
                .details
                .iter()
                .any(|d| d.contains("captured on Claude Code 2.1.283")),
            "{:?}",
            found[0].details
        );
        assert!(!found[0].message.contains("unverified"));
        let mut newer = socket_status(&[], &[], true);
        newer.leads[0].version = "2.1.290".into();
        let found = socket_checks(newer);
        assert_eq!(found.len(), 1);
        assert_ne!(found[0].status, Status::Fail);
        assert!(
            found[0]
                .message
                .contains("wire format captured on 2.1.283; running 2.1.290: unverified"),
            "{}",
            found[0].message
        );
    }

    /// Tracing: TC-046-11
    /// ACs: FR-046-AC-11
    ///
    /// Drift counts surface as their own warns, with the counts, and never fail.
    #[test]
    fn tc_046_11_doctor_warns_on_wire_drift_and_an_unrecognised_transcript_shape() {
        let mut status = socket_status(&[], &[], true);
        status.drift = relay_core::socket_status::WireDrift {
            unrecognised_lines: 2,
            not_json_lines: 1,
            unknown_status_receipts: 4,
            transcript_shape_unrecognised: 3,
        };
        let found = socket_checks(status);
        let codes: Vec<_> = found.iter().map(|c| (c.code, c.status)).collect();
        assert!(
            codes.contains(&(Code::SocketWireDrift, Status::Warn)),
            "{codes:?}"
        );
        assert!(
            codes.contains(&(Code::SocketTranscriptShapeUnrecognised, Status::Warn)),
            "{codes:?}"
        );
        assert!(found.iter().all(|c| c.status != Status::Fail));
        let drift = found
            .iter()
            .find(|c| c.code == Code::SocketWireDrift)
            .unwrap();
        assert!(
            drift.message.contains("7 reply-socket line(s)")
                && drift
                    .message
                    .contains("2 unrecognised, 1 not JSON, 4 receipt(s)"),
            "{}",
            drift.message
        );
        let shape = found
            .iter()
            .find(|c| c.code == Code::SocketTranscriptShapeUnrecognised)
            .unwrap();
        assert!(
            shape.message.contains("3 transcript look(s)"),
            "{}",
            shape.message
        );
        // No drift, no extra checks.
        assert_eq!(socket_checks(socket_status(&[], &[], true)).len(), 1);
    }

    /// Tracing: TC-044-09
    /// ACs: FR-044-AC-9
    ///
    /// The socket check is ok, warn or unknown and never a failure, whatever
    /// the watcher reported.
    #[test]
    fn tc_044_09_the_socket_check_never_fails_and_says_what_is_wrong() {
        let read =
            |status, pid_alive| SocketFacts::Socket(vec![SocketState::Read { status, pid_alive }]);
        let cases = [
            (SocketFacts::Herdr, Code::Ok, Status::Ok),
            (
                read(socket_status(&[], &[], true), Some(true)),
                Code::Ok,
                Status::Ok,
            ),
            (
                read(socket_status(&["a"], &[], true), Some(true)),
                Code::SocketHeld,
                Status::Warn,
            ),
            (
                read(socket_status(&[], &["a: no record"], true), Some(true)),
                Code::SocketFallback,
                Status::Warn,
            ),
            (
                read(socket_status(&[], &[], false), Some(true)),
                Code::SocketFallback,
                Status::Warn,
            ),
            (
                read(socket_status(&[], &[], true), Some(false)),
                Code::SocketStatusUnknown,
                Status::Unknown,
            ),
            (
                SocketFacts::Socket(vec![SocketState::Missing]),
                Code::SocketStatusUnknown,
                Status::Unknown,
            ),
            (
                SocketFacts::Socket(vec![SocketState::NoPath]),
                Code::SocketStatusUnknown,
                Status::Unknown,
            ),
            (
                SocketFacts::Socket(vec![SocketState::Unreadable("bad".into())]),
                Code::SocketStatusUnknown,
                Status::Unknown,
            ),
        ];
        for (socket, code, status) in cases {
            let mut f = healthy();
            f.socket = socket.clone();
            let found: Vec<_> = evaluate(&f)
                .into_iter()
                .filter(|c| c.check == Family::Socket)
                .collect();
            assert_eq!(found.len(), 1, "{socket:?}");
            assert_eq!(
                (found[0].code, found[0].status),
                (code, status),
                "{socket:?}"
            );
            assert_ne!(found[0].status, Status::Fail);
        }
    }

    /// Tracing: TC-044-09
    /// ACs: FR-044-AC-9
    #[test]
    fn tc_044_09_a_held_lead_is_named_with_the_setting_that_clears_it() {
        let mut f = healthy();
        f.socket = SocketFacts::Socket(vec![SocketState::Read {
            status: socket_status(&["a"], &[], true),
            pid_alive: Some(true),
        }]);
        let check = evaluate(&f)
            .into_iter()
            .find(|c| c.code == Code::SocketHeld)
            .unwrap();
        assert_eq!(check.subject.as_deref(), Some("a"));
        assert!(
            check
                .fix
                .as_deref()
                .unwrap()
                .contains("crossSessionInbound")
        );
        assert!(check.fix.as_deref().unwrap().contains("delivery = herdr"));
        assert!(check.details.iter().any(|d| d.contains("/cc-socks/9.sock")));
    }

    /// Tracing: TC-044-09
    /// ACs: FR-044-AC-9
    ///
    /// A config that did not load makes the mode unknown, not "herdr, ok".
    #[test]
    fn tc_044_09_a_config_that_did_not_load_is_unknown_not_herdr_ok() {
        let mut f = healthy();
        f.socket = SocketFacts::ConfigFailed("line 2: bad".into());
        let check = evaluate(&f)
            .into_iter()
            .find(|c| c.check == Family::Socket)
            .unwrap();
        assert_eq!(
            (check.code, check.status),
            (Code::SocketStatusUnknown, Status::Unknown)
        );
        assert!(check.message.contains("line 2: bad"), "{}", check.message);
        assert!(!check.message.contains("herdr (the default)"));
    }

    /// Tracing: TC-044-09
    /// ACs: FR-044-AC-9
    ///
    /// Two watchers on one machine are two reports, each named by its pid; a
    /// leftover report of a dead watcher is dropped while another lives, and
    /// reported as unknown when it is all there is.
    #[test]
    fn tc_044_09_two_watchers_are_two_checks_and_a_dead_ones_leftover_is_dropped() {
        let read = |pid, alive| {
            let mut status = socket_status(&[], &[], true);
            status.pid = pid;
            SocketState::Read {
                status,
                pid_alive: Some(alive),
            }
        };
        let checks = |states: Vec<SocketState>| {
            let mut f = healthy();
            f.socket = SocketFacts::Socket(states);
            evaluate(&f)
                .into_iter()
                .filter(|c| c.check == Family::Socket)
                .collect::<Vec<_>>()
        };
        let two = checks(vec![read(10, true), read(11, true), read(12, false)]);
        let subjects: Vec<_> = two.iter().map(|c| c.subject.clone().unwrap()).collect();
        assert_eq!(subjects, ["watcher pid 10", "watcher pid 11"]);
        let only_dead = checks(vec![read(12, false)]);
        assert_eq!(only_dead.len(), 1);
        assert_eq!(only_dead[0].status, Status::Unknown);
    }

    /// Tracing: TC-044-09
    /// ACs: FR-044-AC-9
    ///
    /// A reply socket that is not bound is said so, and no check text carries a
    /// run of spaces from a broken line continuation.
    #[test]
    fn tc_044_09_an_unbound_reply_socket_is_named_and_no_text_has_a_run_of_spaces() {
        let mut f = healthy();
        let mut status = socket_status(&["a"], &["ea: no record"], false);
        status.reply_socket = None;
        f.socket = SocketFacts::Socket(vec![SocketState::Read {
            status: status.clone(),
            pid_alive: Some(true),
        }]);
        let all = evaluate(&f);
        let text = |c: &Check| {
            [
                Some(c.message.clone()),
                c.cause.clone(),
                c.fix.clone(),
                c.subject.clone(),
            ]
            .into_iter()
            .flatten()
            .chain(c.details.clone())
            .collect::<Vec<_>>()
            .join("\n")
        };
        for c in all.iter().filter(|c| c.check == Family::Socket) {
            assert!(!text(c).contains("     "), "a run of spaces: {}", text(c));
        }
        status.held.clear();
        f.socket = SocketFacts::Socket(vec![SocketState::Read {
            status,
            pid_alive: Some(true),
        }]);
        let unbound: Vec<_> = evaluate(&f)
            .into_iter()
            .filter(|c| c.code == Code::SocketFallback)
            .collect();
        assert_eq!(unbound.len(), 1);
        assert!(unbound[0].details.iter().any(|d| d.contains("not bound")));
        for c in &unbound {
            assert!(!text(c).contains("     "), "{}", text(c));
        }
    }

    /// Tracing: TC-044-09
    /// ACs: FR-044-AC-9
    ///
    /// A batch the watcher dead-lettered is reported with its reason, apart from the
    /// notes that a later note would overwrite.
    #[test]
    fn tc_044_09_a_denied_or_expired_dead_letter_is_reported_with_its_reason() {
        let mut f = healthy();
        let mut status = socket_status(&[], &[], true);
        status.dead_lettered = vec![
            relay_core::socket_status::DeadLetterNote {
                lead: "a".into(),
                batch: 7,
                reason: "denied".into(),
                at_ms: 1,
            },
            relay_core::socket_status::DeadLetterNote {
                lead: "a".into(),
                batch: 8,
                reason: "hold_expired".into(),
                at_ms: 2,
            },
        ];
        status.notes = vec!["a: a later note".into()];
        f.socket = SocketFacts::Socket(vec![SocketState::Read {
            status,
            pid_alive: Some(true),
        }]);
        let checks: Vec<_> = evaluate(&f)
            .into_iter()
            .filter(|c| c.code == Code::SocketDeadLettered)
            .collect();
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].status, Status::Warn);
        let details = checks[0].details.join("\n");
        assert!(
            details.contains("batch 7 dead-lettered as denied"),
            "{details}"
        );
        assert!(
            details.contains("batch 8 dead-lettered as hold_expired"),
            "{details}"
        );
    }

    /// Tracing: TC-044-09
    /// ACs: FR-044-AC-9
    ///
    /// A status directory the watcher does not trust is said so plainly, as `unknown`.
    #[test]
    fn tc_044_09_an_untrusted_status_directory_is_named_plainly() {
        let mut f = healthy();
        f.socket = SocketFacts::Socket(vec![SocketState::Untrusted("open to others".into())]);
        let check = evaluate(&f)
            .into_iter()
            .find(|c| c.check == Family::Socket)
            .unwrap();
        assert_eq!(
            (check.code, check.status),
            (Code::SocketStatusUnknown, Status::Unknown)
        );
        assert!(
            check.message.contains(
                "socket status directory untrusted: held batches will not survive a restart"
            ),
            "{}",
            check.message
        );
    }
}
