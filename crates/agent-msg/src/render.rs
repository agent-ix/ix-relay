//! Pure formatting for the CLI lines `agent-msg`'s escalation commands
//! print, given the typed outcome `relay_ops::escalate`'s functions return.
//!
//! Kept separate from `main.rs`'s command dispatch — and separate from
//! `relay_ops` itself, which prints nothing at all (AGE-2015 PR review) —
//! so the exact wording these commands have always used is unit-tested
//! directly against constructed outcome values, with no Redis, herdr or
//! `linear` process involved.

use relay_ops::escalate::{
    EscalateOutcome, EscalationsOutcome, InvestigateOutcome, LinearOutcome, ResolveOutcome,
};

/// `agent-msg escalate`'s stdout line: just the new escalation's id.
#[must_use]
pub fn escalate_id_line(outcome: &EscalateOutcome) -> String {
    outcome.escalation.id.to_string()
}

/// `agent-msg escalate`'s stderr warning, if the best-effort human-inbox
/// notification failed — `None` when it succeeded. Includes the same
/// `"agent-msg escalate: "` prefix every other warning from this command
/// carries.
#[must_use]
pub fn escalate_warning_line(outcome: &EscalateOutcome) -> Option<String> {
    outcome
        .human_notify_warning
        .as_ref()
        .map(|w| format!("agent-msg escalate: {w}"))
}

/// `agent-msg escalations`' stdout lines: one line per escalation
/// (`id  status  ticket  lead  filed`), or `"no escalations"` when there are
/// none.
#[must_use]
pub fn escalations_lines(outcome: &EscalationsOutcome) -> Vec<String> {
    if outcome.escalations.is_empty() {
        return vec!["no escalations".to_owned()];
    }
    outcome
        .escalations
        .iter()
        .map(|e| {
            format!(
                "{}  {}  {}  {}  {}",
                e.id,
                e.status,
                e.filing.ticket,
                e.filing.lead,
                relay_ops::clock(e.filing.created_at_ms)
            )
        })
        .collect()
}

/// `agent-msg escalations`' stderr warnings for entries that failed to
/// decode.
#[must_use]
pub fn escalations_rejected_lines(outcome: &EscalationsOutcome) -> Vec<String> {
    outcome
        .rejected
        .iter()
        .map(|(id, e)| format!("agent-msg escalations: dropped corrupt entry {id}: {e}"))
        .collect()
}

/// `agent-msg investigate`'s stdout line.
#[must_use]
pub fn investigate_line(outcome: &InvestigateOutcome) -> String {
    if outcome.fresh {
        format!(
            "started a fresh {} session for {} on {} in pane {}",
            outcome.kind, outcome.escalation_id, outcome.machine, outcome.pane_id
        )
    } else {
        format!(
            "forked {} into escalation {} on {} in pane {}",
            outcome.session_id, outcome.escalation_id, outcome.machine, outcome.pane_id
        )
    }
}

/// `agent-msg resolve`'s stdout lines: whether the lead was notified, and —
/// only when the Linear post succeeded — that it was posted. A failed
/// Linear post is [`resolve_warning_lines`] instead, on stderr.
#[must_use]
pub fn resolve_lines(outcome: &ResolveOutcome) -> Vec<String> {
    let mut lines = vec![if outcome.lead_notified {
        format!(
            "resolved {} and notified {}",
            outcome.escalation.id, outcome.escalation.filing.lead
        )
    } else {
        format!(
            "resolved {} but could not notify {}",
            outcome.escalation.id, outcome.escalation.filing.lead
        )
    }];
    if matches!(outcome.linear, LinearOutcome::Posted) {
        lines.push(format!(
            "posted a comment on {}",
            outcome.escalation.filing.ticket
        ));
    }
    lines
}

/// `agent-msg resolve`'s stderr warning and retry hint, only when the
/// Linear post failed.
#[must_use]
pub fn resolve_warning_lines(outcome: &ResolveOutcome) -> Vec<String> {
    let LinearOutcome::Failed(e) = &outcome.linear else {
        return Vec::new();
    };
    vec![
        format!(
            "agent-msg resolve: could not post to {}: {e}",
            outcome.escalation.filing.ticket
        ),
        format!(
            "agent-msg resolve: retry with: agent-msg escalation post-linear {}",
            outcome.escalation.id
        ),
    ]
}

/// `agent-msg escalation post-linear`'s stdout line.
#[must_use]
pub fn post_linear_line(ticket: &relay_core::TicketId) -> String {
    format!("posted a comment on {ticket}")
}

#[cfg(test)]
mod tests {
    use relay_core::{AgentName, Escalation, EscalationFiling, EscalationStatus, TicketId};
    use relay_ops::{escalate::HumanNotifyWarning, linear::LinearError};

    use super::*;

    fn esc(id: &str, status: EscalationStatus) -> Escalation {
        Escalation {
            id: id.parse().unwrap(),
            filing: EscalationFiling {
                ticket: "AGE-2015".parse::<TicketId>().unwrap(),
                lead: "qsl-lead".parse::<AgentName>().unwrap(),
                machine: "node-a".parse().unwrap(),
                session_id: "sid-1".parse().unwrap(),
                cwd: Some("/work".into()),
                kind: "claude".into(),
                context: "ctx".into(),
                options: vec!["a".into()],
                recommendation: "a".into(),
                created_at_ms: 1,
            },
            status,
            decision: None,
        }
    }

    #[test]
    fn escalate_prints_only_the_id_when_the_human_notify_succeeds() {
        let outcome = EscalateOutcome {
            escalation: esc("e7", EscalationStatus::Waiting),
            human_notify_warning: None,
        };
        assert_eq!(escalate_id_line(&outcome), "e7");
        assert_eq!(escalate_warning_line(&outcome), None);
    }

    #[test]
    fn escalate_prefixes_a_human_notify_warning_the_same_way_every_other_one_is() {
        let outcome = EscalateOutcome {
            escalation: esc("e7", EscalationStatus::Waiting),
            human_notify_warning: Some(HumanNotifyWarning::NotifyFailed {
                human: "operator".into(),
                reason: Box::new(relay_core::Error::UnknownAgent("operator".parse().unwrap())),
            }),
        };
        assert_eq!(
            escalate_warning_line(&outcome).unwrap(),
            "agent-msg escalate: could not notify operator: unknown agent operator: it has not \
             registered on the bus (see `agent-msg who`)"
        );
    }

    #[test]
    fn escalations_with_none_prints_the_no_escalations_line() {
        let outcome = EscalationsOutcome {
            escalations: Vec::new(),
            rejected: Vec::new(),
        };
        assert_eq!(escalations_lines(&outcome), vec!["no escalations"]);
    }

    #[test]
    fn escalations_lists_id_status_ticket_and_lead() {
        let outcome = EscalationsOutcome {
            escalations: vec![esc("e7", EscalationStatus::Waiting)],
            rejected: Vec::new(),
        };
        let lines = escalations_lines(&outcome);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("e7"));
        assert!(lines[0].contains("waiting"));
        assert!(lines[0].contains("AGE-2015"));
        assert!(lines[0].contains("qsl-lead"));
    }

    #[test]
    fn escalations_rejected_entries_are_reported_with_the_command_prefix() {
        let outcome = EscalationsOutcome {
            escalations: Vec::new(),
            rejected: vec![(
                "e9".to_owned(),
                relay_core::Error::EscalationNotFound("e9".into()),
            )],
        };
        let lines = escalations_rejected_lines(&outcome);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with("agent-msg escalations: dropped corrupt entry e9:"));
    }

    #[test]
    fn investigate_fork_wording_names_the_session_id() {
        let outcome = InvestigateOutcome {
            escalation_id: "e7".into(),
            fresh: false,
            kind: "claude".into(),
            machine: "node-a".parse().unwrap(),
            pane_id: "w1:p1".parse().unwrap(),
            session_id: "sid-fork".parse().unwrap(),
        };
        assert_eq!(
            investigate_line(&outcome),
            "forked sid-fork into escalation e7 on node-a in pane w1:p1"
        );
    }

    #[test]
    fn investigate_fresh_wording_names_the_kind_not_the_session_id() {
        let outcome = InvestigateOutcome {
            escalation_id: "e7".into(),
            fresh: true,
            kind: "codex".into(),
            machine: "node-a".parse().unwrap(),
            pane_id: "w1:p1".parse().unwrap(),
            session_id: "sid-fork".parse().unwrap(),
        };
        assert_eq!(
            investigate_line(&outcome),
            "started a fresh codex session for e7 on node-a in pane w1:p1"
        );
    }

    #[test]
    fn resolve_reports_notified_and_posted_on_a_clean_run() {
        let outcome = ResolveOutcome {
            escalation: esc("e7", EscalationStatus::Resolved),
            lead_notified: true,
            linear: LinearOutcome::Posted,
        };
        assert_eq!(
            resolve_lines(&outcome),
            vec![
                "resolved e7 and notified qsl-lead",
                "posted a comment on AGE-2015",
            ]
        );
        assert!(resolve_warning_lines(&outcome).is_empty());
    }

    #[test]
    fn resolve_reports_a_failed_linear_post_as_a_warning_with_the_retry_command() {
        let outcome = ResolveOutcome {
            escalation: esc("e7", EscalationStatus::Resolved),
            lead_notified: true,
            linear: LinearOutcome::Failed(LinearError::Failed {
                status: 1,
                stderr: "not logged in".into(),
            }),
        };
        assert_eq!(
            resolve_lines(&outcome),
            vec!["resolved e7 and notified qsl-lead"]
        );
        let warnings = resolve_warning_lines(&outcome);
        assert_eq!(warnings.len(), 2);
        assert!(warnings[0].starts_with("agent-msg resolve: could not post to AGE-2015:"));
        assert_eq!(
            warnings[1],
            "agent-msg resolve: retry with: agent-msg escalation post-linear e7"
        );
    }

    #[test]
    fn resolve_reports_a_failed_lead_notify_without_erroring() {
        let outcome = ResolveOutcome {
            escalation: esc("e7", EscalationStatus::Resolved),
            lead_notified: false,
            linear: LinearOutcome::Posted,
        };
        assert_eq!(
            resolve_lines(&outcome),
            vec![
                "resolved e7 but could not notify qsl-lead",
                "posted a comment on AGE-2015",
            ]
        );
    }

    #[test]
    fn post_linear_names_the_ticket() {
        let ticket = "AGE-2015".parse::<TicketId>().unwrap();
        assert_eq!(post_linear_line(&ticket), "posted a comment on AGE-2015");
    }
}
