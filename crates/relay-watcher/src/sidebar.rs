//! Best-effort short-lived Herdr sidebar metadata for relay leads (SWM-46).

use std::time::Duration;

use relay_core::{PaneId, Registration, SessionId};

use crate::herdr::{AgentGet, Herdr};

/// Token rendered by user-configured `ui.sidebar.agents.rows`.
pub const TOKEN: &str = "ix_relay";

/// Returns the presentation TTL: long enough to survive a few default watcher
/// sweeps, but short enough that a stopped watcher cannot leave a stale badge.
pub fn ttl(sweep: Duration) -> Duration {
    sweep
        .saturating_mul(3)
        .max(Duration::from_secs(45))
        .min(Duration::from_secs(900))
}

/// Formats only state and a pending count; an unknown read is never a zero.
pub fn value(state: &str, pending: Option<usize>) -> String {
    match pending {
        Some(count) => format!("{state} · {count} pending"),
        None => format!("{state} · pending unknown"),
    }
}

/// Publishes metadata only after Herdr confirms this exact registered session
/// still occupies the registered pane. Does not modify semantic agent state.
pub fn report_if_verified(
    herdr: &dyn Herdr,
    registration: &Registration,
    pane: &PaneId,
    display: &str,
    ttl: Duration,
) -> bool {
    let Some(expected) = registration.session_id.as_ref() else {
        return false;
    };
    let Ok(Some(current)) = herdr.get(pane) else {
        return false;
    };
    if !session_matches(expected, &current) {
        return false;
    }
    let ttl_ms = u64::try_from(ttl.as_millis())
        .unwrap_or(u64::MAX)
        .clamp(1, 86_400_000);
    herdr.report_relay_sidebar(pane, display, ttl_ms);
    true
}

fn session_matches(expected: &SessionId, current: &AgentGet) -> bool {
    current.session_id.as_ref() == Some(expected)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use relay_core::Registration;

    use super::*;
    use crate::{
        deliver::Deliverer,
        herdr::{AgentStatus, ProcessError},
    };

    struct FakeHerdr {
        current: Option<SessionId>,
        reported: RefCell<Vec<(String, String, u64)>>,
    }

    impl Herdr for FakeHerdr {
        fn get(&self, _: &PaneId) -> Result<Option<AgentGet>, ProcessError> {
            Ok(self.current.clone().map(|session_id| AgentGet {
                status: AgentStatus::Idle,
                state_change_seq: 1,
                session_id: Some(session_id),
            }))
        }
        fn notify(&self, _: &str, _: &str) -> Result<(), ProcessError> {
            Ok(())
        }
        fn report_relay_sidebar(&self, pane: &PaneId, value: &str, ttl_ms: u64) {
            self.reported
                .borrow_mut()
                .push((pane.to_string(), value.into(), ttl_ms));
        }
    }

    impl Deliverer for FakeHerdr {
        fn deliver(&self, _: &PaneId, _: &str) -> Result<(), crate::deliver::DeliverError> {
            Ok(())
        }
    }

    fn registration(session: Option<&str>) -> Registration {
        Registration {
            name: "work-lead".parse().unwrap(),
            machine: "hub".parse().unwrap(),
            kind: "codex".into(),
            session_id: session.map(|s| s.parse().unwrap()),
            cwd: None,
            herdr_pane: Some("w1:p1".parse().unwrap()),
            place: None,
            last_seen_ms: 1,
        }
    }

    #[test]
    /// Trace: FR-052-AC-3
    fn token_values_keep_unknown_separate_from_zero() {
        assert_eq!(value("connected", Some(0)), "connected · 0 pending");
        assert_eq!(value("unavailable", None), "unavailable · pending unknown");
        assert_eq!(ttl(Duration::from_secs(15)), Duration::from_secs(45));
    }

    #[test]
    /// Trace: FR-052-AC-1, FR-052-AC-3
    fn metadata_is_reported_only_for_the_exact_registered_session() {
        let matching = FakeHerdr {
            current: Some("session-a".parse().unwrap()),
            reported: RefCell::default(),
        };
        assert!(report_if_verified(
            &matching,
            &registration(Some("session-a")),
            &"w1:p1".parse().unwrap(),
            "connected · 2 pending",
            Duration::from_secs(45)
        ));
        assert_eq!(
            matching.reported.borrow()[0],
            ("w1:p1".into(), "connected · 2 pending".into(), 45_000)
        );

        let replaced = FakeHerdr {
            current: Some("session-b".parse().unwrap()),
            reported: RefCell::default(),
        };
        assert!(!report_if_verified(
            &replaced,
            &registration(Some("session-a")),
            &"w1:p1".parse().unwrap(),
            "connected · 2 pending",
            Duration::from_secs(45)
        ));
        assert!(replaced.reported.borrow().is_empty());

        let unidentified = FakeHerdr {
            current: None,
            reported: RefCell::default(),
        };
        assert!(!report_if_verified(
            &unidentified,
            &registration(None),
            &"w1:p1".parse().unwrap(),
            "connected · 0 pending",
            Duration::from_secs(45)
        ));
    }
}
