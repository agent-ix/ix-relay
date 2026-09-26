//! The watcher's herdr version and shape check (FR-036), run at start-up and
//! again when it is due, telling the operator once per failing episode.
//!
//! What a failing check does depends on what it found ([`HerdrCheck`]):
//!
//! - A shape ix-relay cannot read ([`HerdrCheck::blocks_delivery`]) fails
//!   closed: a member typing cannot work without (the pane, the agent's status
//!   and change counter) does not decode, so the watcher types into no pane.
//!   Mail stays queued for the lead's own hook, and the check is re-run every
//!   [`crate::PLACE_REFRESH_EVERY`], so delivery resumes by itself when herdr
//!   answers in a shape ix-relay can read. A member herdr's schema makes optional
//!   (`agent_session`) never counts: typing is guarded per lead by the session
//!   comparison, which refuses that lead when herdr shows no id, so one pane
//!   without a session cannot stop delivery to the others.
//! - A herdr that is missing, or older than [`relay_ops::herdr_check::MIN_HERDR_VERSION`]
//!   with a good shape, is advisory: one notification, shown in the heartbeat and
//!   in `agent-msg doctor`, and the watcher keeps delivering.
//! - A check that could not run (a timeout, a herdr server not up yet) is
//!   unknown: nothing is announced, and it is re-checked.
//!
//! Either way the watcher never exits, and says it once per episode: one
//! notification and one log line, not one per re-check.

use std::time::{Duration, Instant};

use relay_ops::herdr_check::HerdrCheck;

/// The fewest time between two checks a decode failure asks for, so a herdr that
/// fails every call costs a check every few seconds, not every tick.
const MIN_RECHECK_GAP: Duration = Duration::from_secs(5);

/// A notification for the operator: a title and a body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// The notification's title.
    pub title: String,
    /// What is wrong.
    pub body: String,
}

/// Where the last check stands, and whether this episode was announced.
#[derive(Debug)]
pub struct HerdrGate {
    status: HerdrCheck,
    checked_at: Option<Instant>,
    /// Whether the current failing episode has notified. Reset when a check
    /// passes, so the next failure is a new episode.
    notified: bool,
    /// The last line logged for this episode, so a re-check that finds the same
    /// thing logs nothing.
    logged: Option<String>,
    /// How long a check that did not pass waits before it is run again.
    recheck_every: Duration,
}

impl HerdrGate {
    /// A gate that has not checked yet and re-checks anything but a pass every
    /// `recheck_every`.
    #[must_use]
    pub fn new(recheck_every: Duration) -> Self {
        Self {
            status: HerdrCheck::NotChecked,
            checked_at: None,
            notified: false,
            logged: None,
            recheck_every,
        }
    }

    /// The result of the last check.
    #[must_use]
    pub fn status(&self) -> &HerdrCheck {
        &self.status
    }

    /// Whether the watcher must type into no pane right now.
    #[must_use]
    pub fn blocks_delivery(&self) -> bool {
        self.status.blocks_delivery()
    }

    /// Whether a check is due at `now`: none has run yet, one that did not pass
    /// is a re-check period old, or a decode failure was seen (rate-limited).
    #[must_use]
    pub fn due(&self, now: Instant, decode_failed: bool) -> bool {
        let Some(at) = self.checked_at else {
            return true;
        };
        let age = now.saturating_duration_since(at);
        (!self.status.is_ok() && age >= self.recheck_every)
            || (decode_failed && age >= MIN_RECHECK_GAP)
    }

    /// Records a finished check's `result` (run at `now`): what it found, the log
    /// line once per episode, and the notification when it starts one.
    pub fn record(&mut self, now: Instant, result: HerdrCheck) -> Option<Notice> {
        let was_ok = self.status.is_ok();
        self.status = result;
        self.checked_at = Some(now);
        let Some(problem) = self.status.problem() else {
            if !was_ok {
                eprintln!("relay-watcher: herdr passes its version and shape check again");
            }
            self.notified = false;
            self.logged = None;
            return None;
        };
        if self.logged.as_deref() != Some(problem.as_str()) {
            eprintln!("relay-watcher: {problem}");
            self.logged = Some(problem.clone());
        }
        let first = self.status.is_problem() && !self.notified;
        self.notified |= self.status.is_problem();
        let consequence = if self.status.blocks_delivery() {
            "The watcher types into no pane until this passes; mail waits for the lead's own hook"
        } else {
            "The watcher keeps running and hooks still deliver"
        };
        first.then(|| Notice {
            title: "ix-relay: herdr check failed".to_owned(),
            body: format!("{problem}. {consequence}; see `agent-msg doctor`."),
        })
    }
}

#[cfg(test)]
mod tests {
    use relay_ops::herdr_check::MIN_HERDR_VERSION;

    use super::*;

    /// What one look at the gate did.
    struct Observation {
        ran: bool,
        notice: Option<Notice>,
    }

    /// Runs `check` if one is due at `now`, as the watcher's loop does when a
    /// finished check comes back.
    fn observe(
        gate: &mut HerdrGate,
        now: Instant,
        decode_failed: bool,
        check: impl FnOnce() -> HerdrCheck,
    ) -> Observation {
        if !gate.due(now, decode_failed) {
            return Observation {
                ran: false,
                notice: None,
            };
        }
        Observation {
            ran: true,
            notice: gate.record(now, check()),
        }
    }

    fn ok() -> HerdrCheck {
        HerdrCheck::Ok {
            version: MIN_HERDR_VERSION,
            shape_verified: true,
        }
    }

    fn missing() -> HerdrCheck {
        HerdrCheck::Missing {
            reason: "not found".into(),
        }
    }

    fn shape() -> HerdrCheck {
        HerdrCheck::ShapeMismatch {
            command: "agent get",
            detail: "missing `state_change_seq`".into(),
        }
    }

    /// Tracing: TC-036-05
    /// ACs: FR-036-AC-5
    ///
    /// Five polls of a failing herdr raise exactly one notification, and a
    /// re-check that finds another problem in the same episode raises none.
    #[test]
    fn tc_036_05_a_failing_episode_notifies_once_across_many_polls() {
        let mut gate = HerdrGate::new(Duration::from_secs(30));
        let start = Instant::now();
        let mut notices = 0;
        for poll in 0..5u64 {
            let at = start + Duration::from_secs(31 * poll);
            let found = if poll == 3 {
                HerdrCheck::VersionUnreadable { output: "?".into() }
            } else {
                missing()
            };
            notices += usize::from(observe(&mut gate, at, false, || found).notice.is_some());
        }
        assert_eq!(notices, 1);
        assert!(!gate.status().is_ok());
    }

    /// A check that could not run is not announced, and is re-checked; it does
    /// not spend the episode's notification.
    #[test]
    fn an_unknown_check_notifies_nothing_and_does_not_spend_the_episode() {
        let mut gate = HerdrGate::new(Duration::from_secs(30));
        let start = Instant::now();
        let failed = || HerdrCheck::Failed {
            reason: "timed out".into(),
        };
        assert!(observe(&mut gate, start, false, failed).notice.is_none());
        assert!(observe(&mut gate, start + Duration::from_secs(31), false, failed).ran);
        // The first real finding still notifies.
        let found = observe(&mut gate, start + Duration::from_secs(62), false, missing);
        assert!(found.notice.is_some());
    }

    /// Tracing: TC-036-03
    /// ACs: FR-036-AC-3
    ///
    /// A failing check is re-run every re-check period, and a pass ends the
    /// episode without a restart: the status recovers, a passing herdr is not
    /// re-checked, and a later failure notifies again.
    #[test]
    fn tc_036_03_a_recovered_herdr_is_seen_without_a_restart_and_a_new_failure_notifies() {
        let mut gate = HerdrGate::new(Duration::from_secs(30));
        let start = Instant::now();
        let at = |secs| start + Duration::from_secs(secs);
        assert!(observe(&mut gate, start, false, missing).notice.is_some());
        let mut ran = false;
        let soon = observe(&mut gate, at(10), false, || {
            ran = true;
            ok()
        });
        assert!(!soon.ran && !ran, "too soon after a failure");
        assert!(observe(&mut gate, at(31), false, ok).ran);
        assert!(gate.status().is_ok());
        assert!(
            !observe(&mut gate, at(400), false, ok).ran,
            "a pass is not re-checked"
        );
        // A decode failure re-checks it, and this failure is a new episode.
        let again = observe(&mut gate, at(401), true, missing);
        assert!(again.ran && again.notice.is_some());
    }

    /// Tracing: TC-036-04
    /// ACs: FR-036-AC-4
    #[test]
    fn tc_036_04_a_supported_herdr_notifies_nothing() {
        let mut gate = HerdrGate::new(Duration::from_secs(30));
        let seen = observe(&mut gate, Instant::now(), false, ok);
        assert!(seen.ran && seen.notice.is_none());
        assert!(gate.status().is_ok());
        assert!(!gate.blocks_delivery());
    }

    /// Tracing: TC-036-07
    /// ACs: FR-036-AC-6
    ///
    /// A shape ix-relay cannot read blocks delivery and says so in its one
    /// notification; the block lifts by itself when a re-check passes. An
    /// advisory finding blocks nothing.
    #[test]
    fn tc_036_07_a_bad_shape_blocks_delivery_until_a_recheck_passes() {
        let mut gate = HerdrGate::new(Duration::from_secs(30));
        let start = Instant::now();
        let first = observe(&mut gate, start, false, shape);
        assert!(gate.blocks_delivery());
        let body = first.notice.unwrap().body;
        assert!(body.contains("types into no pane"), "{body}");
        assert!(
            observe(&mut gate, start + Duration::from_secs(31), false, shape)
                .notice
                .is_none()
        );
        assert!(gate.blocks_delivery(), "still blocked, still running");
        observe(&mut gate, start + Duration::from_secs(62), false, ok);
        assert!(!gate.blocks_delivery());

        let mut advisory = HerdrGate::new(Duration::from_secs(30));
        let note = observe(&mut advisory, start, false, missing)
            .notice
            .unwrap();
        assert!(note.body.contains("keeps running"), "{}", note.body);
        assert!(!advisory.blocks_delivery());
    }

    #[test]
    fn a_decode_failure_rechecks_a_passing_herdr_but_not_more_than_every_few_seconds() {
        let mut gate = HerdrGate::new(Duration::from_secs(30));
        let start = Instant::now();
        assert!(observe(&mut gate, start, false, ok).ran);
        assert!(!observe(&mut gate, start + Duration::from_secs(1), true, ok).ran);
        assert!(observe(&mut gate, start + Duration::from_secs(6), true, ok).ran);
    }
}
