// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! The one rule that decides the state of a session (FR-061 "Session liveness evidence").
//!
//! It uses only existing evidence: the registry read with its skipped-record report, and the
//! FR-047 placement the machine's watcher stored for each registration. It applies equally to
//! a session on another machine, because both are shared relay records.

use crate::{
    address::Address,
    ids::{MachineName, SessionId},
    message::{PlacementState, Registration},
    store::AgentList,
};

/// What the evidence says about one session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionState {
    /// Exactly one registration of the session exists and its placement is `Verified`.
    Live,
    /// A successful, complete registry read shows no registration of the session.
    Absent,
    /// Absent, and the session's recorded address belongs to a registration of a different
    /// native session.
    Replaced,
    /// Every other case. Never shown as live and never counted as absent.
    Unknown,
}

impl SessionState {
    /// The state as printed.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Absent => "absent",
            Self::Replaced => "replaced",
            Self::Unknown => "unknown",
        }
    }

    /// Whether the session is gone for good as far as the evidence shows: the states in
    /// which an unforced claim succeeds and a pin is reclaimed.
    #[must_use]
    pub const fn is_gone(self) -> bool {
        matches!(self, Self::Absent | Self::Replaced)
    }
}

impl std::fmt::Display for SessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One read of the relay directory, kept so that one command classifies every session it
/// needs against the same evidence.
#[derive(Clone, Debug, Default)]
pub struct LivenessEvidence {
    registrations: Vec<Registration>,
    complete: bool,
}

impl LivenessEvidence {
    /// Evidence from a registry read: complete only when no record was skipped.
    #[must_use]
    pub fn from_registry(list: &AgentList) -> Self {
        Self {
            registrations: list.registrations.clone(),
            complete: list.skipped.is_empty(),
        }
    }

    /// Evidence from registrations known to be the whole registry, for callers and tests that
    /// hold the registrations themselves.
    #[must_use]
    pub fn complete(registrations: Vec<Registration>) -> Self {
        Self {
            registrations,
            complete: true,
        }
    }

    /// Evidence from a read that failed or was partial: no session is ever absent by it.
    #[must_use]
    pub fn unreadable() -> Self {
        Self::default()
    }

    /// The registrations the evidence holds.
    #[must_use]
    pub fn registrations(&self) -> &[Registration] {
        &self.registrations
    }

    /// The state of the session `session` on `machine`, whose address when it last wrote was
    /// `recorded_address` (`None` for a pin, which records no address).
    #[must_use]
    pub fn state_of(
        &self,
        machine: &MachineName,
        session: &SessionId,
        recorded_address: Option<&str>,
    ) -> SessionState {
        let own: Vec<&Registration> = self
            .registrations
            .iter()
            .filter(|r| r.machine == *machine && r.session_id.as_ref() == Some(session))
            .collect();
        match own.as_slice() {
            [only] => {
                if matches!(only.placement_state(), PlacementState::Verified { .. }) {
                    SessionState::Live
                } else {
                    SessionState::Unknown
                }
            }
            [] if self.complete => {
                if self.address_held_by_another(session, recorded_address) {
                    SessionState::Replaced
                } else {
                    SessionState::Absent
                }
            }
            _ => SessionState::Unknown,
        }
    }

    /// The state of the session an owner ID `<machine>/<session>` names, recorded at
    /// `address`. An ID that does not parse is unknown.
    #[must_use]
    pub fn owner_state(&self, owner_id: &str, address: &str) -> SessionState {
        let Some((machine, session)) = owner_id.split_once('/') else {
            return SessionState::Unknown;
        };
        match (machine.parse(), session.parse()) {
            (Ok(machine), Ok(session)) => self.state_of(&machine, &session, Some(address)),
            _ => SessionState::Unknown,
        }
    }

    fn address_held_by_another(&self, session: &SessionId, recorded: Option<&str>) -> bool {
        let Some(recorded) = recorded.and_then(|text| text.parse::<Address>().ok()) else {
            return false;
        };
        self.registrations.iter().any(|r| {
            r.session_id.as_ref().is_some_and(|other| other != session)
                && r.full_address().is_some_and(|address| address == recorded)
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "test fixtures")]

    use ix_trace_rs::trace;

    use super::*;
    use crate::{
        address::{HerdrPlaceIds, Place},
        name::AgentName,
    };

    fn registration(machine: &str, session: &str, tab: &str, verified: bool) -> Registration {
        let machine: MachineName = machine.parse().unwrap();
        let session: SessionId = session.parse().unwrap();
        Registration {
            name: AgentName::for_session(&machine, &session),
            machine,
            kind: "codex".into(),
            session_id: Some(session.clone()),
            cwd: None,
            herdr_pane: Some("w1:p1".parse().unwrap()),
            place: verified.then(|| Place {
                workspace: "QSL".into(),
                tab: tab.into(),
                herdr_ids: Some(HerdrPlaceIds {
                    workspace: "w1".parse().unwrap(),
                    tab: "w1:t1".parse().unwrap(),
                    pane: "w1:p1".parse().unwrap(),
                    session_id: session,
                    verified_at_ms: 1,
                }),
            }),
            last_seen_ms: 1,
        }
    }

    fn state(evidence: &LivenessEvidence, machine: &str, session: &str) -> SessionState {
        evidence.state_of(
            &machine.parse().unwrap(),
            &session.parse().unwrap(),
            Some("terra:QSL:a"),
        )
    }

    #[trace("FR-061-AC-6", "FR-061-AC-7")]
    #[test]
    fn exactly_one_verified_registration_is_live() {
        let evidence = LivenessEvidence::complete(vec![registration("terra", "a", "a", true)]);
        assert_eq!(state(&evidence, "terra", "a"), SessionState::Live);
        assert!(!SessionState::Live.is_gone());
    }

    #[trace("FR-061-AC-7")]
    #[test]
    fn a_complete_read_without_the_session_is_absent_or_replaced() {
        let none = LivenessEvidence::complete(Vec::new());
        assert_eq!(state(&none, "terra", "a"), SessionState::Absent);
        let elsewhere = LivenessEvidence::complete(vec![registration("terra", "z", "z", true)]);
        assert_eq!(state(&elsewhere, "terra", "a"), SessionState::Absent);
        // Another native session now holds the recorded address.
        let taken = LivenessEvidence::complete(vec![registration("terra", "b", "a", true)]);
        assert_eq!(state(&taken, "terra", "a"), SessionState::Replaced);
        assert!(SessionState::Absent.is_gone() && SessionState::Replaced.is_gone());
        // A session on another machine is absent here too: the machine is part of the identity.
        let other_machine = LivenessEvidence::complete(vec![registration("luna", "a", "a", true)]);
        assert_eq!(state(&other_machine, "terra", "a"), SessionState::Absent);
        assert_eq!(state(&other_machine, "luna", "a"), SessionState::Live);
    }

    #[trace("FR-061-AC-16")]
    #[test]
    fn every_other_case_is_unknown_and_never_gone() {
        let unreadable = LivenessEvidence::unreadable();
        assert_eq!(state(&unreadable, "terra", "a"), SessionState::Unknown);
        let unverified = LivenessEvidence::complete(vec![registration("terra", "a", "a", false)]);
        assert_eq!(state(&unverified, "terra", "a"), SessionState::Unknown);
        let twice = LivenessEvidence::complete(vec![
            registration("terra", "a", "a", true),
            registration("terra", "a", "b", true),
        ]);
        assert_eq!(state(&twice, "terra", "a"), SessionState::Unknown);
        assert!(!SessionState::Unknown.is_gone());
        let incomplete = LivenessEvidence {
            registrations: vec![registration("terra", "z", "z", true)],
            complete: false,
        };
        assert_eq!(state(&incomplete, "terra", "a"), SessionState::Unknown);
    }

    #[trace("FR-061-AC-6")]
    #[test]
    fn an_owner_id_that_does_not_parse_is_unknown() {
        let none = LivenessEvidence::complete(Vec::new());
        assert_eq!(none.owner_state("terra/a", "terra:QSL:a"), SessionState::Absent);
        for bad in ["", "terra", "/a", "terra/"] {
            assert_eq!(none.owner_state(bad, "terra:QSL:a"), SessionState::Unknown, "{bad:?}");
        }
        assert_eq!(SessionState::Replaced.to_string(), "replaced");
    }
}
