//! Keeps `last_seen_ms` true for leads whose pane is alive (FR-042).
//!
//! A hook stamps `last_seen_ms` at session start and on each prompt, so a lead
//! sitting idle between turns looks stale, and the 24-hour prune rule
//! ([`crate::PRUNE_AGE`]) and its herdr spawns (one check per lead every ten
//! minutes) run against a live lead. The watcher already knows the pane is
//! alive: it asks herdr, and when the pane still hosts the registered session it
//! moves the stamp forward with a compare-and-set
//! ([`relay_core::Relay::touch_last_seen`]), the same guard
//! [`relay_core::Relay::refresh_place`] uses.
//!
//! The refresh never brings a lead back: the compare-and-set finds nothing to
//! change once the registration is pruned, and it yields to a newer registration.
//! It vouches for a pane only when herdr shows an agent there and the session ids
//! do not differ: a different session, no agent, or a herdr that cannot answer
//! leaves the stamp alone and the pruning rule to decide. A lead that registered
//! with a session id is vouched for only by a pane that shows the same id: a pane
//! with no session (`agent_session` is optional in herdr's output) is not vouched
//! for, because it may be another agent that took the place. A lead registered
//! without an id is vouched for by whatever agent sits in the pane, as the pruning
//! rule already treats an uncertain id ("unknown means do not delete"). While
//! herdr's output is in a shape ix-relay cannot read
//! ([`crate::liveness::Liveness::delivery_blocked`]) the refresh is not run.

use std::time::{Duration, Instant};

use relay_core::{PaneId, Registration};

use crate::{herdr::Herdr, liveness::Liveness, per_lead};

/// A registration is refreshed once its `last_seen_ms` is this old: a lead is
/// therefore never more than this stale plus one check interval while its pane
/// lives, far inside the 24-hour prune age.
pub const REFRESH_AFTER: Duration = Duration::from_secs(15 * 60);

/// How often one lead's pane is looked at for this purpose: a herdr call per
/// lead per period at most, whether or not it was alive.
pub const CHECK_EVERY: Duration = Duration::from_secs(10 * 60);

impl Liveness {
    /// Whether `reg`'s registration is due a liveness check at `now`.
    #[must_use]
    pub fn touch_due(&self, reg: &Registration, now_ms: i64, now: Instant) -> bool {
        let stale = now_ms.saturating_sub(reg.last_seen_ms)
            >= i64::try_from(REFRESH_AFTER.as_millis()).unwrap_or(i64::MAX);
        stale
            && self
                .touch_checked
                .get(&reg.name)
                .is_none_or(|at| now.saturating_duration_since(*at) >= CHECK_EVERY)
    }

    /// Forgets the leads no longer watched.
    pub fn retain_touched(&mut self, seen: &std::collections::HashSet<relay_core::AgentName>) {
        self.touch_checked.retain(|name, _| seen.contains(name));
    }
}

/// Asks herdr whether `reg`'s pane still hosts its session and, if so, moves
/// `last_seen_ms` to `now_ms`. Returns whether the stamp moved.
///
/// A herdr failure is logged and counts as "not alive": an uncertain answer
/// never refreshes.
///
/// # Errors
///
/// A connection-level Redis error, so the tick ends and the connection is
/// redialled (see [`per_lead`]); any other failure is that lead's alone.
pub fn refresh_if_live(
    relay: &mut relay_core::Relay,
    herdr: &impl Herdr,
    live: &mut Liveness,
    reg: &Registration,
    pane: &PaneId,
    now_ms: i64,
    now: Instant,
) -> relay_core::Result<bool> {
    live.touch_checked.insert(reg.name.clone(), now);
    let alive = match herdr.get(pane) {
        // A lead that registered with a session id is vouched for only by a pane
        // that shows one, and the same one: a pane with no session may be another
        // agent that took the place, or the same one before its harness reported.
        Ok(Some(info)) => {
            (reg.session_id.is_none() || info.session_id.is_some())
                && !crate::positively_replaced(reg.session_id.as_ref(), info.session_id.as_ref())
        }
        Ok(None) => false,
        Err(e) => {
            eprintln!(
                "relay-watcher: {}: liveness check for its registration failed: {e}",
                reg.name
            );
            false
        }
    };
    if !alive {
        return Ok(false);
    }
    match relay.touch_last_seen(&reg.name, reg.last_seen_ms, now_ms) {
        Ok(moved) => Ok(moved),
        Err(e) => {
            per_lead(e, format_args!("{}: could not refresh last seen", reg.name)).map(|()| false)
        }
    }
}

#[cfg(test)]
mod tests {
    use relay_core::{Registration, Relay};
    use relay_test_support::Bus;

    use super::*;
    use crate::{
        deliver::{DeliverError, Deliverer},
        herdr::{AgentGet, AgentStatus, ProcessError},
        supervise::Wake,
    };
    use relay_core::PaneId;

    const DAY_MS: i64 = 24 * 60 * 60 * 1000;

    /// A herdr that reports the same agent in every pane, or none.
    struct Panes(Option<&'static str>);

    impl Herdr for Panes {
        fn get(&self, _: &PaneId) -> Result<Option<AgentGet>, ProcessError> {
            Ok(Some(AgentGet {
                status: AgentStatus::Idle,
                state_change_seq: 1,
                session_id: self.0.map(|s| s.parse().unwrap()),
            }))
        }
        fn notify(&self, _: &str, _: &str) -> Result<(), ProcessError> {
            Ok(())
        }
    }

    impl Deliverer for Panes {
        fn deliver(&self, _: &PaneId, _: &str) -> Result<(), DeliverError> {
            Ok(())
        }
    }

    /// A herdr that reports no agent in any pane.
    struct Empty;

    impl Herdr for Empty {
        fn get(&self, _: &PaneId) -> Result<Option<AgentGet>, ProcessError> {
            Ok(None)
        }
        fn notify(&self, _: &str, _: &str) -> Result<(), ProcessError> {
            Ok(())
        }
    }

    impl Deliverer for Empty {
        fn deliver(&self, _: &PaneId, _: &str) -> Result<(), DeliverError> {
            Ok(())
        }
    }

    fn register(bus: &mut Bus, name: &str, session: &str, last_seen_ms: i64) -> Registration {
        let reg = Registration {
            name: name.parse().unwrap(),
            machine: "node-a".parse().unwrap(),
            kind: "claude".into(),
            session_id: Some(session.parse().unwrap()),
            cwd: None,
            herdr_pane: Some("w1:p1".parse().unwrap()),
            place: None,
            last_seen_ms,
        };
        bus.relay.register(&reg).unwrap();
        reg
    }

    /// Tracing: TC-042-01
    /// ACs: FR-042-AC-1
    ///
    /// An idle lead whose pane is alive is refreshed once its stamp is old, and
    /// only its stamp changes.
    #[test]
    fn tc_042_01_a_live_idle_lead_stays_fresh() {
        let Some(mut bus) = Bus::connect("touch_live") else {
            return;
        };
        let old = crate::now_ms() - 2 * REFRESH_AFTER.as_millis() as i64;
        let reg = register(&mut bus, "idle", "s1", old);
        let mut live = Liveness::default();
        assert!(live.touch_due(&reg, crate::now_ms(), Instant::now()));
        let moved = refresh_if_live(
            &mut bus.relay,
            &Panes(Some("s1")),
            &mut live,
            &reg,
            &"w1:p1".parse().unwrap(),
            crate::now_ms(),
            Instant::now(),
        )
        .unwrap();
        assert!(moved);
        let after = bus.relay.agent(&reg.name).unwrap().unwrap();
        assert!(after.last_seen_ms > old);
        assert_eq!(
            Registration {
                last_seen_ms: old,
                ..after
            },
            reg
        );
        assert!(
            !live.touch_due(&reg, crate::now_ms(), Instant::now()),
            "one herdr look per lead per period"
        );
    }

    /// Tracing: TC-042-02
    /// ACs: FR-042-AC-2
    ///
    /// A pane that hosts another session, has no agent, or cannot be asked is not
    /// vouched for, and a lead that was pruned or registered again meanwhile is
    /// neither brought back nor overwritten.
    #[test]
    fn tc_042_02_the_refresh_never_vouches_for_a_dead_pane_or_resurrects_a_lead() {
        let Some(mut bus) = Bus::connect("touch_never") else {
            return;
        };
        let old = crate::now_ms() - DAY_MS;
        let reg = register(&mut bus, "stale", "s1", old);
        let now = crate::now_ms();
        let mut live = Liveness::default();
        for herdr in [&Panes(Some("someone-else")) as &dyn Answer, &Empty] {
            assert!(!herdr.refresh(&mut bus.relay, &mut live, &reg, now));
        }
        assert_eq!(
            bus.relay.agent(&reg.name).unwrap().unwrap().last_seen_ms,
            old
        );

        // Registered again meanwhile: the fresher registration wins.
        let newer = Registration {
            last_seen_ms: now - 1,
            ..reg.clone()
        };
        bus.relay.register(&newer).unwrap();
        assert!(!Panes(Some("s1")).refresh(&mut bus.relay, &mut live, &reg, now));
        assert_eq!(
            bus.relay.agent(&reg.name).unwrap().unwrap().last_seen_ms,
            now - 1
        );

        // Pruned meanwhile: it stays gone.
        bus.relay.prune(&reg.name, now - 1).unwrap().unwrap();
        assert!(!Panes(Some("s1")).refresh(&mut bus.relay, &mut live, &newer, now));
        assert!(bus.relay.agent(&reg.name).unwrap().is_none());
    }

    /// What both fakes share, so one loop can drive them.
    trait Answer {
        fn refresh(
            &self,
            relay: &mut Relay,
            live: &mut Liveness,
            reg: &Registration,
            now: i64,
        ) -> bool;
    }

    impl<H: Herdr> Answer for H {
        fn refresh(
            &self,
            relay: &mut Relay,
            live: &mut Liveness,
            reg: &Registration,
            now: i64,
        ) -> bool {
            refresh_if_live(
                relay,
                self,
                live,
                reg,
                &"w1:p1".parse().unwrap(),
                now,
                Instant::now(),
            )
            .unwrap()
        }
    }

    /// Tracing: TC-042-03
    /// ACs: FR-042-AC-3
    ///
    /// Through a whole tick: an old registration whose pane is alive is refreshed
    /// and not pruned, and an old one whose pane is gone is still pruned.
    #[test]
    fn tc_042_03_a_tick_refreshes_a_live_lead_and_still_prunes_a_dead_one() {
        let Some(mut bus) = Bus::connect("touch_tick") else {
            return;
        };
        let old = crate::now_ms() - 2 * DAY_MS;
        let reg = register(&mut bus, "alive", "s1", old);
        let mut state = crate::WatcherState {
            wake: Wake::Sweep,
            ..crate::WatcherState::default()
        };
        crate::tick(
            &mut bus.relay,
            &Panes(Some("s1")),
            &"node-a".parse().unwrap(),
            &mut state,
            &mut |_| {},
        )
        .unwrap();
        let after = bus
            .relay
            .agent(&reg.name)
            .unwrap()
            .expect("a live lead is not pruned");
        assert!(after.last_seen_ms > old + DAY_MS);

        let gone = register(&mut bus, "gone", "s2", old);
        let mut state = crate::WatcherState::default();
        crate::tick(
            &mut bus.relay,
            &Empty,
            &"node-a".parse().unwrap(),
            &mut state,
            &mut |_| {},
        )
        .unwrap();
        assert!(
            bus.relay.agent(&gone.name).unwrap().is_none(),
            "a dead pane is still pruned"
        );
    }

    /// Tracing: TC-042-06
    /// ACs: FR-042-AC-6
    ///
    /// A registration without an id is vouched for by any agent in the pane (as
    /// the pruning rule does); one with an id needs a pane that shows the same
    /// id, so a pane with none, or another, does not vouch.
    #[test]
    fn tc_042_06_a_registered_id_needs_the_pane_to_show_the_same_id() {
        let Some(mut bus) = Bus::connect("touch_ids") else {
            return;
        };
        let now = crate::now_ms();
        let old = now - DAY_MS;
        let mut live = Liveness::default();
        let mut case =
            |bus: &mut Bus, name: &str, registered: Option<&str>, shown: Option<&'static str>| {
                let mut reg = register(bus, name, "x", old);
                reg.session_id = registered.map(|s| s.parse().unwrap());
                bus.relay.register(&reg).unwrap();
                refresh_if_live(
                    &mut bus.relay,
                    &Panes(shown),
                    &mut live,
                    &reg,
                    &"w1:p1".parse().unwrap(),
                    now,
                    Instant::now(),
                )
                .unwrap()
            };
        assert!(case(&mut bus, "none-none", None, None), "neither has an id");
        assert!(
            !case(&mut bus, "reg-none", Some("s1"), None),
            "herdr reports no id for a lead that registered one"
        );
        assert!(
            case(&mut bus, "none-shown", None, Some("s9")),
            "the lead has no id"
        );
        assert!(case(&mut bus, "same", Some("s1"), Some("s1")));
        assert!(
            !case(&mut bus, "differ", Some("s1"), Some("s2")),
            "two ids that differ"
        );
    }
}
