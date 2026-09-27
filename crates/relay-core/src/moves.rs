//! What the watcher records when a lead's pane moves or its place is renamed,
//! in types both binaries share (FR-043).
//!
//! Two records, both written by `relay-watcher` and read by `agent-msg doctor`
//! and `agent-msg who`:
//!
//! - [`MoveEvent`], one entry of the capped stream `<prefix>health:moves`: a
//!   registration was relinked to the pane that now hosts its session, or its
//!   place label changed. One event per change, written by the same script that
//!   changes the registration, so a change without its event (or an event without
//!   its change) cannot happen.
//! - [`PaneReplaced`], a field of the hash `<prefix>health:replaced:<lead>`: the pane a
//!   registration names now hosts another session and its own session is
//!   nowhere on the machine.
//!
//! Every text in either comes from herdr (a workspace or tab label) or from a
//! harness (a session id): render it neutralised ([`crate::text`]). A reader
//! tolerates members it does not know and a kind it does not know.

use serde::{Deserialize, Serialize};

use crate::{
    address::Place,
    ids::{MachineName, PaneId, SessionId},
    name::AgentName,
};

/// What changed for a lead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MoveKind {
    /// The lead's session is now hosted by another pane: its registration was
    /// pointed at that pane (`previous_pane` to `pane`) and given its place.
    Relink,
    /// The lead's pane is the same and its workspace or tab label changed.
    Rename,
    /// A kind a newer watcher wrote that this binary does not know.
    #[serde(other)]
    Unknown,
}

/// One entry of `<prefix>health:moves`, stored as JSON under the field `rec`
/// beside the plain fields `machine` and `lead`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveEvent {
    /// The lead's bus name, kept across the change.
    pub lead: AgentName,
    /// The machine whose herdr issued the pane ids.
    pub machine: MachineName,
    /// What changed.
    pub kind: MoveKind,
    /// The harness session id, the join key a relink follows.
    pub session_id: Option<SessionId>,
    /// The pane the registration named before: set on a relink only.
    pub previous_pane: Option<PaneId>,
    /// The pane the registration names now (the same one on a rename).
    pub pane: Option<PaneId>,
    /// The place recorded before the change.
    pub old_place: Option<Place>,
    /// The place recorded after it.
    pub new_place: Option<Place>,
    /// When the watcher made the change, Unix milliseconds.
    pub at_ms: i64,
}

/// A registration whose pane no longer hosts its session, as the watcher found it
/// (FR-032): the field of `<prefix>health:replaced:<lead>` under the lead's bus name.
///
/// It describes one registration only: a reader shows it while the registration
/// still names `pane` and still has `seen_last_seen_ms`, so registering again (a
/// hook stamps a fresh `last_seen_ms`) ends it without anyone deleting it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneReplaced {
    /// The pane the registration names, which now hosts another session.
    pub pane: PaneId,
    /// The registration's `last_seen_ms` when the watcher found this.
    pub seen_last_seen_ms: i64,
    /// When the watcher first found it, Unix milliseconds.
    pub since_ms: i64,
}

impl PaneReplaced {
    /// Whether this finding is about `reg` as it is stored now: the registration
    /// still names the pane and still has the stamp the watcher saw. A lead that
    /// registered again, or was relinked, is not described by an old finding.
    #[must_use]
    pub fn describes(&self, reg: &crate::Registration) -> bool {
        reg.herdr_pane.as_ref() == Some(&self.pane) && reg.last_seen_ms == self.seen_last_seen_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> MoveEvent {
        MoveEvent {
            lead: "lead-a".parse().unwrap(),
            machine: "hub".parse().unwrap(),
            kind: MoveKind::Relink,
            session_id: Some("sess-1".parse().unwrap()),
            previous_pane: Some("w1:p3".parse().unwrap()),
            pane: Some("w2:p2".parse().unwrap()),
            old_place: None,
            new_place: Some(Place {
                workspace: "wsB".into(),
                tab: "1".into(),
                herdr_ids: None,
            }),
            at_ms: 9,
        }
    }

    /// Every member of the event is a documented wire member: adding or renaming
    /// one fails here and asks for the spec (FR-043) to change too.
    #[test]
    fn the_event_has_exactly_the_documented_fields() {
        let value = serde_json::to_value(sample()).unwrap();
        let mut keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "at_ms",
                "kind",
                "lead",
                "machine",
                "new_place",
                "old_place",
                "pane",
                "previous_pane",
                "session_id"
            ]
        );
        assert_eq!(value["kind"], "relink");
    }

    #[test]
    fn the_replaced_marker_has_exactly_the_documented_fields() {
        let marker = PaneReplaced {
            pane: "w1:p1".parse().unwrap(),
            seen_last_seen_ms: 1,
            since_ms: 2,
        };
        let value = serde_json::to_value(marker).unwrap();
        let mut keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["pane", "seen_last_seen_ms", "since_ms"]);
    }

    /// Tracing: TC-032-04
    /// ACs: FR-032-AC-4
    #[test]
    fn a_finding_describes_only_the_registration_it_saw() {
        let found = PaneReplaced {
            pane: "w1:p1".parse().unwrap(),
            seen_last_seen_ms: 10,
            since_ms: 20,
        };
        let mut reg = crate::Registration {
            name: "lead-a".parse().unwrap(),
            machine: "hub".parse().unwrap(),
            kind: "claude".into(),
            session_id: None,
            cwd: None,
            herdr_pane: Some("w1:p1".parse().unwrap()),
            place: None,
            last_seen_ms: 10,
        };
        assert!(found.describes(&reg));
        reg.last_seen_ms = 11;
        assert!(!found.describes(&reg), "it registered again");
        reg.last_seen_ms = 10;
        reg.herdr_pane = Some("w1:p2".parse().unwrap());
        assert!(!found.describes(&reg), "it was relinked");
        reg.herdr_pane = None;
        assert!(!found.describes(&reg));
    }

    /// A newer watcher's event, with a member and a kind this binary has never
    /// heard of, still reads.
    #[test]
    fn a_newer_event_with_extra_members_and_an_unknown_kind_still_reads() {
        let mut value = serde_json::to_value(sample()).unwrap();
        value["added_later"] = serde_json::json!(1);
        value["kind"] = serde_json::json!("a_kind_from_the_future");
        let read: MoveEvent = serde_json::from_value(value).unwrap();
        assert_eq!(read.kind, MoveKind::Unknown);
    }
}
