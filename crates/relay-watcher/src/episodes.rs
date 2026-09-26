//! Per-lead episodes: a condition that persists across ticks is logged once
//! when it starts and once when it ends, not on every poll.
//!
//! A lead is in at most one [`EpisodeKind`] at a time. [`Episodes`] remembers
//! which, and tells the caller what changed ([`Transition`]) so it logs only
//! that. It only ever holds leads seen this tick ([`Episodes::retain_present`]),
//! so it cannot grow past the size of the registry.

use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

use relay_core::AgentName;

/// How long to wait before retrying a blocked-lead notification that herdr
/// refused. Each retry is logged, so a broken herdr is visible but is not
/// hammered every tick (a herdr call can take up to ten seconds).
pub const NOTIFY_RETRY_EVERY: Duration = Duration::from_secs(5 * 60);

/// What a blocked lead's tick should do about telling the operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    /// The operator has not been told about this blocked stretch: notify now.
    Needed,
    /// The operator was already told for this blocked stretch: stay quiet.
    Done,
    /// A notification failed recently and the next retry is not due yet.
    RetryLater,
}

/// A condition worth logging once per stretch of consecutive ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpisodeKind {
    /// Waiting on an approval dialog; the operator was notified when this began.
    Blocked,
    /// The registered session id does not match the pane's occupant.
    Stale,
    /// The registered pane has no agent.
    NoAgent,
    /// `herdr agent get` keeps failing.
    GetFailed,
    /// The lead is blocked but herdr's notification call failed; the
    /// notification is retried each tick until it goes through.
    NotifyFailed,
    /// The lead has as much in flight as one more pop may add to, so its mail
    /// waits for a lease to free room (AGE-2032).
    InflightFull,
    /// The lead's Claude Code holds relay mail behind an approval prompt
    /// (FR-044); the operator is told once when this begins.
    SocketHeld,
}

impl EpisodeKind {
    /// What the condition is called in the end-of-episode log line.
    #[must_use]
    pub fn describe(self) -> &'static str {
        match self {
            Self::Blocked => "blocked on a dialog",
            Self::Stale => "stale registration",
            Self::NoAgent => "no agent in its pane",
            Self::GetFailed => "herdr agent get failing",
            Self::NotifyFailed => "blocked, notification failing",
            Self::InflightFull => "in-flight cap reached",
            Self::SocketHeld => "mail held by Claude Code for approval",
        }
    }
}

/// What changed for one lead between two ticks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Transition {
    /// The episode that just ended (or was replaced), if any.
    pub ended: Option<EpisodeKind>,
    /// The episode that just began, if any.
    pub started: Option<EpisodeKind>,
}

/// The episode each lead is currently in, and whether the operator has been told
/// about its blocked stretch.
///
/// "Blocked stretch" outlasts a `herdr agent get` blip: a lead seen blocked,
/// then with a failing `get`, then blocked again is one stretch and the operator is
/// told once (NFR-003-M-6). Any other outcome ends the stretch.
#[derive(Debug, Default)]
pub struct Episodes {
    active: HashMap<AgentName, EpisodeKind>,
    /// Leads the operator has been told are blocked, for their current stretch.
    notified: HashSet<AgentName>,
    /// When the last failed notification attempt for a lead's stretch ran.
    notify_failed_at: HashMap<AgentName, Instant>,
}

impl Episodes {
    /// The episode `name` is in, if any.
    #[must_use]
    pub fn kind(&self, name: &AgentName) -> Option<EpisodeKind> {
        self.active.get(name).copied()
    }

    /// Whether a blocked `name` needs telling about, as of `now`.
    #[must_use]
    pub fn notice(&self, name: &AgentName, now: Instant) -> Notice {
        if self.notified.contains(name) {
            return Notice::Done;
        }
        match self.notify_failed_at.get(name) {
            Some(failed) if now.saturating_duration_since(*failed) < NOTIFY_RETRY_EVERY => {
                Notice::RetryLater
            }
            _ => Notice::Needed,
        }
    }

    /// Records that a notification attempt for `name` failed at `now`.
    pub fn notify_failed(&mut self, name: &AgentName, now: Instant) {
        self.notify_failed_at.insert(name.clone(), now);
    }

    /// Records that this tick found `name` in `now` (`None`: in no episode)
    /// and reports what that changed.
    pub fn observe(&mut self, name: &AgentName, now: Option<EpisodeKind>) -> Transition {
        match now {
            Some(EpisodeKind::Blocked | EpisodeKind::SocketHeld) => {
                self.notified.insert(name.clone());
                self.notify_failed_at.remove(name);
            }
            // A failed notification keeps its retry clock; a failing `get`
            // says nothing about whether the lead is still blocked.
            Some(EpisodeKind::NotifyFailed | EpisodeKind::GetFailed) => {}
            Some(EpisodeKind::Stale | EpisodeKind::NoAgent | EpisodeKind::InflightFull) | None => {
                self.notified.remove(name);
                self.notify_failed_at.remove(name);
            }
        }
        let before = self.kind(name);
        if before == now {
            return Transition::default();
        }
        match now {
            Some(kind) => {
                self.active.insert(name.clone(), kind);
            }
            None => {
                self.active.remove(name);
            }
        }
        Transition {
            ended: before,
            started: now,
        }
    }

    /// Forgets every lead not in `present`, returning the episodes that ended
    /// that way. Called once per tick with the leads that tick looked at.
    pub fn retain_present(
        &mut self,
        present: &HashSet<AgentName>,
    ) -> Vec<(AgentName, EpisodeKind)> {
        self.notified.retain(|name| present.contains(name));
        self.notify_failed_at
            .retain(|name, _| present.contains(name));
        let gone: Vec<AgentName> = self
            .active
            .keys()
            .filter(|name| !present.contains(*name))
            .cloned()
            .collect();
        gone.into_iter()
            .filter_map(|name| self.active.remove(&name).map(|kind| (name, kind)))
            .collect()
    }

    /// Every lead any of the maps still holds (for tests: hygiene means this is
    /// always a subset of the registry).
    #[cfg(test)]
    pub fn tracked(&self) -> HashSet<AgentName> {
        self.active
            .keys()
            .chain(&self.notified)
            .chain(self.notify_failed_at.keys())
            .cloned()
            .collect()
    }

    /// The leads currently in an episode (for tests: production code only
    /// ever asks about one lead).
    #[cfg(test)]
    pub fn names(&self) -> impl Iterator<Item = &AgentName> {
        self.active.keys()
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn name(n: u8) -> AgentName {
        format!("lead-{n}").parse().unwrap()
    }

    fn kind_strategy() -> impl Strategy<Value = Option<EpisodeKind>> {
        prop_oneof![
            Just(None),
            Just(Some(EpisodeKind::Blocked)),
            Just(Some(EpisodeKind::Stale)),
            Just(Some(EpisodeKind::NoAgent)),
            Just(Some(EpisodeKind::GetFailed)),
            Just(Some(EpisodeKind::NotifyFailed)),
            Just(Some(EpisodeKind::SocketHeld)),
        ]
    }

    #[test]
    fn a_repeated_episode_is_silent_and_a_change_reports_both_ends() {
        let mut episodes = Episodes::default();
        let lead = name(1);
        let start = episodes.observe(&lead, Some(EpisodeKind::NoAgent));
        assert_eq!(start.started, Some(EpisodeKind::NoAgent));
        assert_eq!(start.ended, None);
        assert_eq!(
            episodes.observe(&lead, Some(EpisodeKind::NoAgent)),
            Transition::default()
        );
        let change = episodes.observe(&lead, Some(EpisodeKind::Blocked));
        assert_eq!(change.ended, Some(EpisodeKind::NoAgent));
        assert_eq!(change.started, Some(EpisodeKind::Blocked));
        let end = episodes.observe(&lead, None);
        assert_eq!(end.ended, Some(EpisodeKind::Blocked));
        assert_eq!(end.started, None);
        assert_eq!(episodes.observe(&lead, None), Transition::default());
    }

    /// NFR-003-M-6: a blocked lead whose `herdr agent get` fails for a tick and
    /// then recovers is one blocked stretch, so the operator is told once.
    /// Tracing: TC-N003-M-06 (NFR-003-M-6). Evidence: unit test.
    #[test]
    fn a_get_failure_blip_does_not_renotify_a_blocked_lead() {
        let (mut episodes, lead, t0) = (Episodes::default(), name(1), Instant::now());
        assert_eq!(episodes.notice(&lead, t0), Notice::Needed);
        episodes.observe(&lead, Some(EpisodeKind::Blocked));
        episodes.observe(&lead, Some(EpisodeKind::GetFailed));
        assert_eq!(episodes.notice(&lead, t0), Notice::Done);
        episodes.observe(&lead, Some(EpisodeKind::Blocked));
        assert_eq!(episodes.notice(&lead, t0), Notice::Done);
        // Working again ends the stretch; the next block is a new one.
        episodes.observe(&lead, None);
        assert_eq!(episodes.notice(&lead, t0), Notice::Needed);
    }

    /// Tracing: TC-N003-M-06 (NFR-003-M-6). Evidence: unit test.
    #[test]
    fn a_failed_notification_is_retried_only_after_the_retry_interval() {
        let (mut episodes, lead, t0) = (Episodes::default(), name(1), Instant::now());
        episodes.observe(&lead, Some(EpisodeKind::NotifyFailed));
        episodes.notify_failed(&lead, t0);
        assert_eq!(
            episodes.notice(&lead, t0 + Duration::from_secs(2)),
            Notice::RetryLater
        );
        assert_eq!(
            episodes.notice(&lead, t0 + NOTIFY_RETRY_EVERY),
            Notice::Needed
        );
        // Once it goes through, the stretch is notified for good.
        episodes.observe(&lead, Some(EpisodeKind::Blocked));
        assert_eq!(
            episodes.notice(&lead, t0 + NOTIFY_RETRY_EVERY * 3),
            Notice::Done
        );
    }

    proptest! {
        /// After `retain_present`, no lead outside the present set is tracked,
        /// whatever was observed before.
        #[test]
        fn tracked_names_are_always_a_subset_of_the_present_registry(
            observed in proptest::collection::vec((0u8..12, kind_strategy()), 0..60),
            present in proptest::collection::hash_set(0u8..12, 0..12),
        ) {
            let mut episodes = Episodes::default();
            for (lead, kind) in observed {
                episodes.observe(&name(lead), kind);
            }
            let present: HashSet<AgentName> = present.into_iter().map(name).collect();
            let ended = episodes.retain_present(&present);
            prop_assert!(episodes.names().all(|n| present.contains(n)));
            prop_assert!(episodes.tracked().iter().all(|n| present.contains(n)));
            prop_assert!(ended.iter().all(|(n, _)| !present.contains(n)));
        }

        /// One start per stretch of the same kind: the number of started
        /// transitions equals the number of times the kind changed to `Some`.
        #[test]
        fn a_lead_logs_only_when_its_episode_changes(
            kinds in proptest::collection::vec(kind_strategy(), 0..60)
        ) {
            let mut episodes = Episodes::default();
            let lead = name(0);
            let started = kinds
                .iter()
                .filter(|kind| episodes.observe(&lead, **kind).started.is_some())
                .count();
            let mut expected = 0;
            let mut previous = None;
            for kind in &kinds {
                if kind.is_some() && *kind != previous {
                    expected += 1;
                }
                previous = *kind;
            }
            prop_assert_eq!(started, expected);
        }
    }
}
