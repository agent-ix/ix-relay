//! `agent-msg status`: bounded, read-only relay health for the Herdr status bar (SWM-43).

use std::{
    io::{self, Write},
    process::ExitCode,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use relay_core::{
    AgentName, PaneRef, Registration, Relay, SessionId,
    config::{self, CoreFlags},
    health::{HeartbeatRead, TICK_FUTURE_TOLERANCE_MS, tick_is_stale},
};
use relay_ops::naming::{HerdrEnv, LocationHerdr, RealLocationHerdr};
use serde::Serialize;

/// Total wall-clock budget for one status query. Herdr and Redis share this deadline.
const STATUS_BUDGET: Duration = Duration::from_millis(1800);

/// Maximum time spent asking Herdr for the focused pane's current session.
const HERDR_TIMEOUT: Duration = Duration::from_millis(500);

/// Whether the Redis bus answered the bounded read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum RedisState {
    Reachable,
    Unreachable,
    ReadFailed,
    Unknown,
}

/// The local watcher's state as reported by its machine heartbeat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum WatcherState {
    Fresh,
    Stale,
    Missing,
    Unreadable,
    Degraded,
    Unknown,
}

/// Whether this Herdr pane is verified as a relay lead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", content = "name", rename_all = "snake_case")]
enum LeadState {
    Verified(AgentName),
    NotEnrolled,
    Unknown,
}

/// What Herdr could prove about the agent currently in the claimed pane.
enum PaneSession {
    NoPane,
    NoAgent,
    Live(SessionId),
    Unknown,
}

/// The machine-local status fields used by both JSON consumers and the Herdr bar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct StatusReport {
    /// Combined bus and watcher state.
    status: OverallState,
    /// Whether a bounded Redis read succeeded.
    redis: RedisState,
    /// Local watcher heartbeat state.
    watcher: WatcherState,
    /// Whether this Herdr pane has a verified lead registration.
    lead: LeadState,
}

/// Overall machine-local relay state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum OverallState {
    Connected,
    Degraded,
    Unavailable,
}

impl StatusReport {
    fn new(redis: RedisState, watcher: WatcherState, lead: LeadState) -> Self {
        let status = match (redis, watcher) {
            (RedisState::Reachable, WatcherState::Fresh) => OverallState::Connected,
            (RedisState::Unreachable | RedisState::ReadFailed, _) => OverallState::Unavailable,
            _ => OverallState::Degraded,
        };
        Self {
            status,
            redis,
            watcher,
            lead,
        }
    }

    fn compact(&self) -> &'static str {
        match (self.redis, &self.lead, self.status, self.watcher) {
            (RedisState::Unreachable | RedisState::ReadFailed, _, _, _) => "× relay unavailable",
            (RedisState::Unknown, _, _, _) => "? relay status unknown",
            (_, LeadState::NotEnrolled, _, _) => "○ relay not enrolled",
            (_, LeadState::Unknown, _, _) => "? relay identity unknown",
            (_, LeadState::Verified(_), OverallState::Connected, _) => "● relay",
            (_, LeadState::Verified(_), OverallState::Unavailable, _) => "× relay unavailable",
            (_, LeadState::Verified(_), _, WatcherState::Stale) => "! relay watcher stale",
            (_, LeadState::Verified(_), _, WatcherState::Missing) => "! relay watcher missing",
            (_, LeadState::Verified(_), _, WatcherState::Unreadable) => {
                "! relay watcher unreadable"
            }
            (_, LeadState::Verified(_), _, WatcherState::Degraded) => "! relay watcher degraded",
            (_, LeadState::Verified(_), _, WatcherState::Unknown) => "? relay status unknown",
            (_, LeadState::Verified(_), _, WatcherState::Fresh) => "! relay degraded",
        }
    }
}

/// Runs the read-only status command and emits the Herdr display line or JSON.
pub(crate) fn run(flags: CoreFlags, json: bool) -> ExitCode {
    let report = report(flags);
    let mut out = io::stdout().lock();
    let result = if json {
        serde_json::to_writer(&mut out, &report)
            .map_err(io::Error::other)
            .and_then(|()| writeln!(out))
    } else {
        writeln!(out, "{}", report.compact())
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("agent-msg status: {e}");
            ExitCode::FAILURE
        }
    }
}

fn report(flags: CoreFlags) -> StatusReport {
    let Ok(settings) = config::load_core(flags) else {
        return StatusReport::new(
            RedisState::Unknown,
            WatcherState::Unknown,
            LeadState::Unknown,
        );
    };

    let deadline = Instant::now() + STATUS_BUDGET;
    let herdr_env = HerdrEnv::from_process_env();
    let pane = herdr_env.as_ref().map(|env| env.pane_id.clone());
    let pane_session = match pane.as_ref() {
        Some(pane_id) => {
            let timeout =
                remaining(deadline).map_or(Duration::ZERO, |left| left.min(HERDR_TIMEOUT));
            if timeout.is_zero() {
                PaneSession::Unknown
            } else {
                match RealLocationHerdr::new(timeout).agent(pane_id) {
                    Ok(Some(agent)) => agent
                        .session_id
                        .map_or(PaneSession::NoAgent, PaneSession::Live),
                    Ok(None) => PaneSession::NoAgent,
                    Err(_) => PaneSession::Unknown,
                }
            }
        }
        None => PaneSession::NoPane,
    };

    let Some(timeout) = remaining(deadline) else {
        return StatusReport::new(
            RedisState::Unknown,
            WatcherState::Unknown,
            lead_unverified_or_not_enrolled(&pane_session),
        );
    };
    let mut relay = match Relay::connect(&settings.url.value, timeout) {
        Ok(relay) => relay.with_prefix(settings.prefix.value),
        Err(_) => {
            return StatusReport::new(
                RedisState::Unreachable,
                WatcherState::Unknown,
                lead_unverified_or_not_enrolled(&pane_session),
            );
        }
    };
    relay.set_scan_deadline(Some(deadline));

    let Some(timeout) = remaining(deadline) else {
        return StatusReport::new(
            RedisState::Reachable,
            WatcherState::Unknown,
            LeadState::Unknown,
        );
    };
    if relay.set_timeout(timeout).is_err() {
        return StatusReport::new(
            RedisState::ReadFailed,
            WatcherState::Unknown,
            LeadState::Unknown,
        );
    }
    let heartbeat = match relay.heartbeat(&settings.machine.value) {
        Ok(heartbeat) => heartbeat,
        Err(_) => {
            return StatusReport::new(
                RedisState::ReadFailed,
                WatcherState::Unknown,
                LeadState::Unknown,
            );
        }
    };
    let watcher = watcher_state(heartbeat.as_ref(), now_ms());

    let lead = match (pane, &pane_session, remaining(deadline)) {
        (Some(pane_id), PaneSession::Live(session), Some(timeout)) => {
            if relay.set_timeout(timeout).is_err() {
                LeadState::Unknown
            } else {
                match relay.registrations_for_pane(&PaneRef::new(settings.machine.value, pane_id)) {
                    Ok(registrations) => {
                        lead_state(&registrations, &PaneSession::Live(session.clone()))
                    }
                    Err(_) => LeadState::Unknown,
                }
            }
        }
        (_, PaneSession::Unknown, _) => LeadState::Unknown,
        (_, PaneSession::NoPane | PaneSession::NoAgent, _) => LeadState::NotEnrolled,
        (_, PaneSession::Live(_), _) => LeadState::Unknown,
    };
    StatusReport::new(RedisState::Reachable, watcher, lead)
}

fn lead_state(registrations: &[Registration], pane_session: &PaneSession) -> LeadState {
    let session = match pane_session {
        PaneSession::Live(session) => session,
        PaneSession::NoPane | PaneSession::NoAgent => return LeadState::NotEnrolled,
        PaneSession::Unknown => return LeadState::Unknown,
    };
    let mut matches = registrations
        .iter()
        .filter(|registration| registration.session_id.as_ref() == Some(session));
    let Some(registration) = matches.next() else {
        return LeadState::NotEnrolled;
    };
    if matches.next().is_some() {
        return LeadState::Unknown;
    }
    LeadState::Verified(registration.name.clone())
}

/// A live pane cannot be called unenrolled until its bus registrations were read.
fn lead_unverified_or_not_enrolled(pane_session: &PaneSession) -> LeadState {
    match pane_session {
        PaneSession::NoPane | PaneSession::NoAgent => LeadState::NotEnrolled,
        PaneSession::Live(_) | PaneSession::Unknown => LeadState::Unknown,
    }
}

fn watcher_state(heartbeat: Option<&HeartbeatRead>, now_ms: i64) -> WatcherState {
    let Some(heartbeat) = heartbeat else {
        return WatcherState::Missing;
    };
    match heartbeat {
        HeartbeatRead::Undecodable => WatcherState::Unreadable,
        HeartbeatRead::Live(heartbeat) => {
            let age_ms = now_ms.saturating_sub(heartbeat.last_tick_ms);
            let too_far_in_future = age_ms < -TICK_FUTURE_TOLERANCE_MS;
            if !heartbeat.redis_ok || !heartbeat.herdr_ok || too_far_in_future {
                WatcherState::Degraded
            } else if tick_is_stale(age_ms.max(0), heartbeat.sweep_secs) {
                WatcherState::Stale
            } else {
                WatcherState::Fresh
            }
        }
    }
}

fn remaining(deadline: Instant) -> Option<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use relay_core::{
        AgentName, Registration, SessionId,
        health::{HeartbeatRead, WatcherHeartbeat},
    };

    use super::*;

    fn heartbeat(last_tick_ms: i64, sweep_secs: u64) -> HeartbeatRead {
        HeartbeatRead::Live(Box::new(WatcherHeartbeat {
            machine: "node-a".parse().unwrap(),
            pid: 1,
            version: "0.2.1".to_owned(),
            started_at_ms: 0,
            last_tick_ms,
            last_sweep_ms: None,
            sweep_secs,
            herdr_ok: true,
            herdr_problem: None,
            herdr_blocks_delivery: false,
            redis_ok: true,
            leads_watched: 1,
            otlp: None,
            last_error: None,
        }))
    }

    fn registration(name: &str, session: &str) -> Registration {
        Registration {
            name: name.parse::<AgentName>().unwrap(),
            machine: "node-a".parse().unwrap(),
            kind: "claude".to_owned(),
            session_id: Some(session.parse::<SessionId>().unwrap()),
            cwd: None,
            herdr_pane: Some("w1:p1".parse().unwrap()),
            place: None,
            last_seen_ms: 1,
        }
    }

    #[test]
    fn watcher_states_distinguish_missing_stale_unreadable_and_fresh() {
        assert_eq!(watcher_state(None, 100_000), WatcherState::Missing);
        assert_eq!(
            watcher_state(Some(&HeartbeatRead::Undecodable), 100_000),
            WatcherState::Unreadable
        );
        assert_eq!(
            watcher_state(Some(&heartbeat(0, 2)), 100_000),
            WatcherState::Stale
        );
        assert_eq!(
            watcher_state(Some(&heartbeat(99_000, 2)), 100_000),
            WatcherState::Fresh
        );
    }

    #[test]
    fn only_the_live_herdr_session_matching_the_pane_registration_is_verified() {
        let registered = registration("qsl-lead", "session-1");
        let live: SessionId = "session-1".parse().unwrap();
        assert_eq!(
            lead_state(
                std::slice::from_ref(&registered),
                &PaneSession::Live(live.clone())
            ),
            LeadState::Verified("qsl-lead".parse().unwrap())
        );
        let other: SessionId = "session-2".parse().unwrap();
        assert_eq!(
            lead_state(std::slice::from_ref(&registered), &PaneSession::Live(other)),
            LeadState::NotEnrolled
        );
        assert_eq!(
            lead_state(&[], &PaneSession::NoAgent),
            LeadState::NotEnrolled
        );
        assert_eq!(
            lead_state(&[], &PaneSession::NoPane),
            LeadState::NotEnrolled
        );
        assert_eq!(
            lead_unverified_or_not_enrolled(&PaneSession::Live("session-1".parse().unwrap())),
            LeadState::Unknown
        );
        assert_eq!(
            lead_unverified_or_not_enrolled(&PaneSession::NoAgent),
            LeadState::NotEnrolled
        );
    }

    #[test]
    fn duplicate_live_session_registrations_are_unknown_not_guessed() {
        let registrations = [
            registration("qsl-lead", "session-1"),
            registration("other-lead", "session-1"),
        ];
        let live: SessionId = "session-1".parse().unwrap();
        assert_eq!(
            lead_state(&registrations, &PaneSession::Live(live)),
            LeadState::Unknown
        );
    }

    #[test]
    fn non_lead_panes_never_render_as_connected_leads() {
        let report = StatusReport::new(
            RedisState::Reachable,
            WatcherState::Fresh,
            LeadState::NotEnrolled,
        );
        assert_eq!(report.compact(), "○ relay not enrolled");
        assert_eq!(report.status, OverallState::Connected);
    }

    #[test]
    fn unreachable_bus_is_visible_even_when_lead_identity_is_unverified() {
        let report = StatusReport::new(
            RedisState::Unreachable,
            WatcherState::Unknown,
            LeadState::Unknown,
        );
        assert_eq!(report.compact(), "× relay unavailable");
    }

    #[test]
    fn bus_availability_distinguishes_healthy_degraded_and_unavailable() {
        let lead = LeadState::Verified("qsl-lead".parse().unwrap());
        assert_eq!(
            StatusReport::new(RedisState::Reachable, WatcherState::Fresh, lead.clone()).status,
            OverallState::Connected
        );
        assert_eq!(
            StatusReport::new(RedisState::Reachable, WatcherState::Stale, lead.clone()).status,
            OverallState::Degraded
        );
        assert_eq!(
            StatusReport::new(RedisState::Unreachable, WatcherState::Unknown, lead).status,
            OverallState::Unavailable
        );
    }

    #[test]
    fn status_json_keeps_bus_watcher_and_lead_claims_separate() {
        let report = StatusReport::new(
            RedisState::Reachable,
            WatcherState::Fresh,
            LeadState::Verified("qsl-lead".parse().unwrap()),
        );
        let value = serde_json::to_value(report).unwrap();
        assert_eq!(value["status"], "connected");
        assert_eq!(value["redis"], "reachable");
        assert_eq!(value["watcher"], "fresh");
        assert_eq!(value["lead"]["state"], "verified");
        assert_eq!(value["lead"]["name"], "qsl-lead");
    }
}
