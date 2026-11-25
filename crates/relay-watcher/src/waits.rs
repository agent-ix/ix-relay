//! Mail the inbox socket has not got to its lead (FR-044).
//!
//! A lead with a socket is never typed into, so mail its socket cannot take waits
//! in the inbox. The operator is told once per stretch, by notification, after it
//! has waited past a bound; the notification says why.

use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

use relay_core::{AgentName, text::sanitise_line};

/// How long mail waits for a socket before the operator is told.
pub const WAIT_NOTICE_AFTER: Duration = Duration::from_secs(5 * 60);

/// How long after a failed notification the next one is tried.
const NOTICE_RETRY: Duration = Duration::from_secs(60);

#[derive(Debug)]
struct Wait {
    since: Instant,
    why: String,
    told: bool,
    retry_at: Option<Instant>,
    /// A socket has the mail and its consumption is awaited: nothing to announce
    /// unless it comes back.
    posted: bool,
}

/// A notification that is due.
#[derive(Debug, PartialEq, Eq)]
pub struct WaitNotice {
    /// The lead whose mail waits.
    pub lead: AgentName,
    /// The notification's title.
    pub title: String,
    /// The notification's body.
    pub body: String,
}

/// The leads whose mail is waiting for their socket, one entry per stretch.
#[derive(Debug)]
pub struct Waits {
    after: Duration,
    waits: HashMap<AgentName, Wait>,
}

impl Default for Waits {
    fn default() -> Self {
        Self::new(WAIT_NOTICE_AFTER)
    }
}

impl Waits {
    /// Waits that are announced once they are `after` old.
    #[must_use]
    pub fn new(after: Duration) -> Self {
        Self {
            after,
            waits: HashMap::new(),
        }
    }

    /// Mail for `lead` did not get through its socket at `now`, for `why`. The
    /// stretch keeps its start; the newest reason is the one reported.
    pub fn undelivered(&mut self, lead: &AgentName, why: &str, now: Instant) {
        let wait = self.waits.entry(lead.clone()).or_insert_with(|| Wait {
            since: now,
            why: String::new(),
            told: false,
            retry_at: None,
            posted: false,
        });
        wait.why = sanitise_line(why);
        wait.posted = false;
    }

    /// A socket took the mail of `lead` and has not consumed it yet. A stretch under
    /// way is paused, not ended: if the mail comes back it has waited since the start.
    pub fn posted(&mut self, lead: &AgentName) {
        if let Some(wait) = self.waits.get_mut(lead) {
            wait.posted = true;
        }
    }

    /// Ends the stretch of `lead`: its mail got through, or there is none left.
    pub fn delivered(&mut self, lead: &AgentName) {
        self.waits.remove(lead);
    }

    /// The notifications due at `now`. Each is returned once, unless
    /// [`Waits::failed`] says it could not be shown.
    pub fn due(&mut self, now: Instant) -> Vec<WaitNotice> {
        let after = self.after;
        let mut due: Vec<WaitNotice> = self
            .waits
            .iter_mut()
            .filter(|(_, w)| {
                !w.posted
                    && !w.told
                    && now.saturating_duration_since(w.since) >= after
                    && w.retry_at.is_none_or(|at| now >= at)
            })
            .map(|(lead, wait)| {
                wait.told = true;
                let minutes = now.saturating_duration_since(wait.since).as_secs() / 60;
                WaitNotice {
                    lead: lead.clone(),
                    title: format!("ix-relay: mail for {lead} is waiting"),
                    body: format!(
                        "It has waited {minutes} minute(s) for the lead's inbox socket: {}. \
                         Nothing is typed; the mail stays queued. See `agent-msg doctor`.",
                        wait.why
                    ),
                }
            })
            .collect();
        due.sort_by(|a, b| a.lead.cmp(&b.lead));
        due
    }

    /// The notification about `lead` could not be shown at `now`: it is due again
    /// later.
    pub fn failed(&mut self, lead: &AgentName, now: Instant) {
        if let Some(wait) = self.waits.get_mut(lead) {
            wait.told = false;
            wait.retry_at = Some(now + NOTICE_RETRY);
        }
    }

    /// Forgets the leads no longer watched.
    pub fn retain(&mut self, seen: &HashSet<AgentName>) {
        self.waits.retain(|lead, _| seen.contains(lead));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tracing: TC-044-21
    /// ACs: FR-044-AC-21
    #[test]
    fn tc_044_21_one_notification_per_stretch_after_the_bound_and_again_only_if_it_failed() {
        let lead: AgentName = "ea".parse().unwrap();
        let mut waits = Waits::new(Duration::from_secs(300));
        let start = Instant::now();
        waits.undelivered(&lead, "the socket refused", start);
        assert!(waits.due(start + Duration::from_secs(299)).is_empty());
        waits.undelivered(
            &lead,
            "the session is parked",
            start + Duration::from_secs(200),
        );
        let due = waits.due(start + Duration::from_secs(300));
        assert_eq!(due.len(), 1);
        assert!(due[0].body.contains("the session is parked"), "{due:?}");
        assert!(due[0].body.contains("5 minute"), "{due:?}");
        assert!(waits.due(start + Duration::from_secs(900)).is_empty());
        waits.failed(&lead, start + Duration::from_secs(900));
        assert!(waits.due(start + Duration::from_secs(930)).is_empty());
        assert_eq!(waits.due(start + Duration::from_secs(960)).len(), 1);
        // Delivered: the next wait is a new stretch, timed from its own start.
        waits.delivered(&lead);
        waits.undelivered(&lead, "again", start + Duration::from_secs(1000));
        assert!(waits.due(start + Duration::from_secs(1200)).is_empty());
        assert_eq!(waits.due(start + Duration::from_secs(1300)).len(), 1);
        // Posted and refused later, again and again: one stretch, timed from its start.
        waits.delivered(&lead);
        waits.undelivered(&lead, "refused", start);
        waits.posted(&lead);
        assert!(waits.due(start + Duration::from_secs(600)).is_empty());
        waits.undelivered(&lead, "refused", start + Duration::from_secs(600));
        assert_eq!(waits.due(start + Duration::from_secs(600)).len(), 1);
        waits.retain(&HashSet::new());
        assert!(waits.due(start + Duration::from_secs(9000)).is_empty());
    }
}
