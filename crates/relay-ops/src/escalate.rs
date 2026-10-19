//! `escalate` / `escalations` / `investigate` / `resolve` / `escalation
//! show|post-linear`: a lead asking the operator for a decision, the operator looking into
//! it, and the decision going back.
//!
//! Kept separate from `agent-msg`'s clap wiring so each command's logic can
//! be unit-tested directly against a real (throwaway-prefixed) Redis, with
//! fake [`Herdr`] and [`Linear`] implementations standing in for the two
//! external processes `investigate` and `resolve`/`escalation post-linear`
//! shell out to.
//!
//! **Nothing here prints.** Every command returns a typed outcome (or
//! [`crate::OpsError`]); `agent-msg` prints from it with the CLI's own
//! wording, and `relay-tui` reads the same typed value to decide what its
//! status line says and which UI state to move to (e.g.
//! [`ResolveOutcome::linear`], never text-sniffing a captured stdout
//! buffer). This also means a caller two layers away from a terminal (a
//! background worker thread) never risks writing raw text over whatever
//! else owns the screen.
//!
//! **No escalation text ever crosses a shell or a terminal's escape-sequence
//! interpreter.** `investigate`'s fresh-session prompt is a fixed string
//! built only from validated fields (the escalation id, the lead's
//! [`AgentName`], and the [`relay_core::TicketId`]) — never `context`,
//! `options` or `recommendation`, which are free-form lead-supplied text.
//! Reading that text back is a separate command, [`run_escalation_show`]
//! (plus the pure [`render_escalation_detail`] formatter), for whichever
//! terminal *this* process's own stdout reaches — not one herdr types into
//! another pane. `relay_core::Relay::escalate` also rejects control
//! characters in that text as defense in depth (see
//! `relay_core::escalation::validate_safe_text`), but the prompt design does
//! not depend on that check: even if it were bypassed, no escalation text
//! is ever built into a herdr argv or shell command.

use relay_core::{
    AgentName, Escalation, EscalationFiling, EscalationId, EscalationStatus, MachineName, Message,
    PaneId, Relay, SessionId, Target,
};

use crate::{
    OpsError, Result,
    herdr::Herdr,
    linear::{Linear, LinearError},
};

/// What `escalate` needs beyond the lead's own registration (read from the
/// bus, not passed in — see [`run_escalate`]).
pub struct EscalateArgs {
    /// The Linear ticket, e.g. `AGE-2014`. Validated by [`run_escalate`].
    pub ticket: String,
    /// What the lead needs decided.
    pub context: String,
    /// The choices the lead sees.
    pub options: Vec<String>,
    /// The lead's own recommendation.
    pub recommendation: String,
    /// Who to notify on the bus. Resolved by `agent-msg`'s CLI via
    /// `relay_core::config` (flag > env > file > default `operator`).
    pub human: String,
}

/// Why the best-effort notification of the human inbox failed after an
/// escalation was filed. `Display` is the text `agent-msg` prints.
#[derive(Debug, thiserror::Error)]
pub enum HumanNotifyWarning {
    /// `--human` did not parse as an agent name.
    #[error("--human {human:?}: {reason}")]
    InvalidHuman {
        /// The text supplied.
        human: String,
        /// Why it is not a valid name.
        reason: Box<relay_core::Error>,
    },
    /// The notification could not be sent to the bus.
    #[error("could not notify {human}: {reason}")]
    NotifyFailed {
        /// The human inbox that was to be notified.
        human: String,
        /// Why sending failed.
        reason: Box<relay_core::Error>,
    },
}

/// What [`run_escalate`] produced.
#[derive(Debug)]
pub struct EscalateOutcome {
    /// The newly filed escalation.
    pub escalation: Escalation,
    /// `Some` when best-effort notifying the human inbox failed (an
    /// unparsable `--human`, or the notify call itself failing) — this
    /// never fails [`run_escalate`], since the escalation is already filed
    /// by the time it runs. The caller prefixes its `Display` the way every
    /// other `agent-msg` warning is (`"agent-msg escalate: {warning}"`).
    pub human_notify_warning: Option<HumanNotifyWarning>,
}

/// Files a new escalation for `me`, signed with `me`'s own current
/// registration (machine, session id, cwd, kind), and best-effort notifies
/// `args.human` on the bus with a short pointer — never the full context,
/// options or recommendation (see [`Relay::notify_human`] and this module's
/// own doc comment on why escalation text never leaves the store this way).
///
/// # Errors
///
/// [`OpsError::NotRegistered`] when `me` has never registered on the bus,
/// [`OpsError::RegistrationUnreadable`] when its record cannot be read here,
/// [`OpsError::NoSessionId`] when `me`'s registration has no session id
/// (nothing for `investigate` to fork later). Propagates
/// [`relay_core::Error::InvalidTicket`] when `args.ticket` is not a Linear
/// key, [`relay_core::Error::InvalidSessionId`] and
/// [`relay_core::Error::UnsafeEscalationText`] from `Relay::escalate`
/// unchanged. The human-inbox notification is best-effort: a failure there
/// (e.g. `args.human` doesn't parse as an [`AgentName`]) is reported in
/// [`EscalateOutcome::human_notify_warning`], not as an `Err` — the
/// escalation itself is already filed by then.
pub fn run_escalate(
    relay: &mut Relay,
    me: &AgentName,
    args: EscalateArgs,
    now_ms: i64,
) -> Result<EscalateOutcome> {
    let ticket = args.ticket.parse()?;
    let reg = match relay.agent_slot(me)? {
        relay_core::AgentSlot::Registered(reg) => reg,
        relay_core::AgentSlot::Absent => {
            return Err(OpsError::NotRegistered { agent: me.clone() });
        }
        relay_core::AgentSlot::Unreadable => {
            return Err(OpsError::RegistrationUnreadable { agent: me.clone() });
        }
    };
    let session_id = reg
        .session_id
        .clone()
        .ok_or_else(|| OpsError::NoSessionId { agent: me.clone() })?;
    let new = EscalationFiling {
        ticket,
        lead: me.clone(),
        machine: reg.machine,
        session_id,
        cwd: reg.cwd,
        kind: reg.kind,
        context: args.context,
        options: args.options,
        recommendation: args.recommendation,
        created_at_ms: now_ms,
    };
    let esc = relay.escalate(new)?;

    let human_notify_warning = match args.human.parse::<AgentName>() {
        Ok(human) => {
            // Deliberately short and free of lead-supplied text: this line
            // is typed straight into the operator's own session, so it carries only
            // the escalation id, ticket and lead — a pointer to
            // `escalation show`, not the content itself.
            let notice = format!(
                "escalation {} filed by {me} on {}: run `agent-msg escalation show {}`",
                esc.id, esc.filing.ticket, esc.id
            );
            relay
                .notify_human(me, &human, &notice, now_ms)
                .err()
                .map(|e| HumanNotifyWarning::NotifyFailed {
                    human: args.human.clone(),
                    reason: Box::new(e),
                })
        }
        Err(e) => Some(HumanNotifyWarning::InvalidHuman {
            human: args.human.clone(),
            reason: Box::new(e),
        }),
    };

    Ok(EscalateOutcome {
        escalation: esc,
        human_notify_warning,
    })
}

/// What [`run_escalations`] found.
pub struct EscalationsOutcome {
    /// Escalations ready for the caller, oldest first, optionally filtered
    /// to one status (whatever `run_escalations` was asked for).
    pub escalations: Vec<Escalation>,
    /// `(id, why)` for every entry that failed to decode. The caller
    /// decides how to surface them (`agent-msg escalations` prints them to
    /// stderr).
    pub rejected: Vec<(String, relay_core::Error)>,
}

/// Lists escalations, oldest first, optionally filtered to one `status`.
pub fn run_escalations(
    relay: &mut Relay,
    status: Option<EscalationStatus>,
) -> Result<EscalationsOutcome> {
    let listed = relay.list(status)?;
    Ok(EscalationsOutcome {
        escalations: listed.escalations,
        rejected: listed.rejected,
    })
}

/// Fetches one escalation's full detail — everything
/// [`render_escalation_detail`] formats, and everything `investigate
/// --fresh`'s fixed prompt points a fresh session at instead of embedding
/// directly (see this module's own doc comment).
pub fn run_escalation_show(relay: &mut Relay, id: &EscalationId) -> Result<Escalation> {
    Ok(relay.get(id)?)
}

/// Renders one escalation's full detail as `agent-msg escalation show` has
/// always printed it: ticket, lead, machine, cwd, kind, status, filed,
/// context, options, recommendation and — once resolved — the decision.
/// Pure formatting, no I/O: the caller (`agent-msg`, `relay-tui`) decides
/// where this text goes, from a terminal that already has the escalation's
/// own free-form text in hand (see this module's own doc comment on why
/// that text is never typed into a *different* pane).
#[must_use]
pub fn render_escalation_detail(esc: &Escalation) -> String {
    use relay_core::text::{sanitise, sanitise_line};
    use std::fmt::Write as _;
    // Everything here was written by a lead (or by hand into Redis) and goes
    // to the operator's terminal: none of it is trusted to be free of escape codes.
    let mut out = String::new();
    let _ = writeln!(out, "id:             {}", esc.id);
    let _ = writeln!(out, "ticket:         {}", esc.filing.ticket);
    let _ = writeln!(out, "lead:           {}", esc.filing.lead);
    let _ = writeln!(
        out,
        "machine:        {}",
        // A `MachineName` holds no control or invisible character.
        esc.filing.machine
    );
    let _ = writeln!(
        out,
        "cwd:            {}",
        sanitise_line(esc.filing.cwd.as_deref().unwrap_or("-"))
    );
    let _ = writeln!(out, "kind:           {}", sanitise_line(&esc.filing.kind));
    let _ = writeln!(out, "status:         {}", esc.status);
    let _ = writeln!(
        out,
        "filed:          {}",
        crate::clock(esc.filing.created_at_ms)
    );
    let _ = writeln!(out, "context:");
    let _ = writeln!(
        out,
        "  {}",
        sanitise(&esc.filing.context).replace('\n', "\n  ")
    );
    let _ = writeln!(out, "options:");
    for option in &esc.filing.options {
        let _ = writeln!(out, "  - {}", sanitise_line(option));
    }
    let _ = writeln!(out, "recommendation:");
    let _ = writeln!(
        out,
        "  {}",
        sanitise(&esc.filing.recommendation).replace('\n', "\n  ")
    );
    if let Some(decision) = &esc.decision {
        let _ = writeln!(out, "decision:");
        let _ = writeln!(out, "  {}", sanitise(decision).replace('\n', "\n  "));
    }
    out
}

/// Retries posting `id`'s resolution to its Linear ticket — for when
/// `resolve`'s own attempt failed (see [`run_resolve`]'s doc comment).
/// Returns the escalation on success, so the caller can print its ticket.
///
/// # Errors
///
/// [`OpsError::NoDecisionYet`] when the escalation has no decision yet (not
/// `Resolved`). Otherwise propagates [`relay_core::Error`] and
/// [`LinearError`] unchanged — unlike [`run_resolve`], posting is this
/// command's whole job, so its failure is this command's error.
pub fn run_escalation_post_linear(
    relay: &mut Relay,
    linear: &impl Linear,
    id: &EscalationId,
) -> Result<Escalation> {
    let esc = relay.get(id)?;
    let Some(decision) = &esc.decision else {
        return Err(OpsError::NoDecisionYet {
            id: id.clone(),
            status: esc.status,
        });
    };
    let body = render_linear_comment(&esc, decision);
    linear.comment(esc.filing.ticket.as_str(), &body)?;
    Ok(esc)
}

/// What [`run_investigate`] produced, for the caller to print or display.
#[derive(Debug)]
pub struct InvestigateOutcome {
    /// The escalation that was investigated.
    pub escalation_id: String,
    /// `true` for a fresh session (`--fresh`); `false` for a fork.
    pub fresh: bool,
    /// The lead's harness kind (e.g. `claude`).
    pub kind: String,
    /// The machine the session was opened on.
    pub machine: MachineName,
    /// The pane hosting the new session.
    pub pane_id: PaneId,
    /// The lead's session id that was forked. Only meaningful when
    /// `!fresh`; present either way for a uniform outcome shape.
    pub session_id: SessionId,
}

/// Opens a Claude Code session on an escalation's lead's machine, in its cwd:
/// forked from the lead's own session by default, or fresh with a brief
/// prompt when `fresh` is set. Marks the escalation `Investigating` only
/// once herdr reports the session started; if starting it fails after the
/// tab was created, the tab is closed again rather than left dangling.
///
/// The herdr agent name (and tab label) includes an attempt number from
/// [`Relay::next_investigation_attempt`] (`esc-<id>-<attempt>`), so a second
/// `investigate` on the same escalation — allowed, see
/// [`Relay::set_investigating`] — never collides with a still-open pane from
/// the first attempt: herdr agent names must be unique among live agents.
///
/// # Errors
///
/// [`OpsError::NothingToInvestigate`] when the escalation is already resolved, [`OpsError::NoRecordedCwd`] when
/// it has no recorded `cwd`, or (fork only) [`OpsError::ForkNeedsClaude`]
/// when its lead's harness `kind` isn't `claude`.
/// [`OpsError::InvestigateCleanupFailed`] when starting the session fails
/// *and* closing the tab `tab_create` had just opened also fails — both
/// failures named, neither silently dropped. Otherwise propagates
/// [`relay_core::Error`] (escalation not found) and
/// [`crate::herdr::ProcessError`] (herdr itself failed, tab already closed
/// cleanly) unchanged.
pub fn run_investigate(
    relay: &mut Relay,
    herdr: &impl Herdr,
    this_machine: &MachineName,
    id: &EscalationId,
    fresh: bool,
) -> Result<InvestigateOutcome> {
    let esc = relay.get(id)?;
    if esc.status == EscalationStatus::Resolved {
        return Err(OpsError::NothingToInvestigate { id: id.clone() });
    }
    let cwd = esc
        .filing
        .cwd
        .clone()
        .ok_or_else(|| OpsError::NoRecordedCwd { id: id.clone() })?;
    if !fresh && esc.filing.kind != "claude" {
        return Err(OpsError::ForkNeedsClaude {
            id: id.clone(),
            kind: esc.filing.kind,
        });
    }
    let machine = if esc.filing.machine == *this_machine {
        None
    } else {
        Some(esc.filing.machine.as_str())
    };

    let attempt = relay.next_investigation_attempt(id)?;
    let label = format!("esc-{id}-{attempt}");
    let created = herdr.tab_create(machine, &cwd, &label)?;

    let start_result = if fresh {
        // Fixed string, built only from validated fields (the generated id,
        // the lead's AgentName, the TicketId) — see this module's own doc
        // comment on why escalation text (context/options/recommendation)
        // never appears here.
        let prompt = format!(
            "Escalation {id} from {} on {}. Run `agent-msg escalation show {id}` and help the operator decide.",
            esc.filing.lead, esc.filing.ticket
        );
        herdr.agent_start(
            machine,
            &label,
            &esc.filing.kind,
            &created.pane_id,
            &[prompt.as_str()],
        )
    } else {
        herdr.agent_start(
            machine,
            &label,
            "claude",
            &created.pane_id,
            &["--resume", esc.filing.session_id.as_str(), "--fork-session"],
        )
    };

    if let Err(e) = start_result {
        if let Err(close_err) = herdr.tab_close(machine, &created.tab_id) {
            return Err(OpsError::InvestigateCleanupFailed {
                start: Box::new(e),
                tab_id: created.tab_id,
                cleanup: Box::new(close_err),
            });
        }
        return Err(OpsError::Herdr(e));
    }

    relay.set_investigating(id)?;
    Ok(InvestigateOutcome {
        escalation_id: id.to_string(),
        fresh,
        kind: esc.filing.kind,
        machine: esc.filing.machine,
        pane_id: created.pane_id,
        session_id: esc.filing.session_id,
    })
}

/// Whether posting the resolution to Linear succeeded, as part of a
/// [`ResolveOutcome`].
#[derive(Debug)]
pub enum LinearOutcome {
    /// The comment landed on the ticket.
    Posted,
    /// It didn't. Not fatal to [`run_resolve`] — the escalation is already
    /// resolved and the lead already notified by the time this runs; the
    /// caller decides how to surface it (`agent-msg` prints a warning and
    /// the `escalation post-linear` retry command).
    Failed(LinearError),
}

/// What [`run_resolve`] produced.
#[derive(Debug)]
pub struct ResolveOutcome {
    /// The now-resolved escalation, decision included.
    pub escalation: Escalation,
    /// Whether the lead was successfully told over the bus. `false` only if
    /// sending the notification itself failed (e.g. the lead deregistered
    /// between escalating and being resolved) — the resolution itself has
    /// already happened by then, so this is reported, not propagated as an
    /// error.
    pub lead_notified: bool,
    /// Whether the decision was also posted to the Linear ticket.
    pub linear: LinearOutcome,
}

/// Resolves an escalation with `decision` and tells the lead over the bus
/// (signed by `human`) and Linear about it.
///
/// Validates *before* calling [`Relay::resolve`]: an oversized decision, or
/// a `human` that doesn't parse as an [`AgentName`], is refused with the
/// escalation left exactly as it was (`Waiting` or `Investigating`), rather
/// than resolved with no one told.
///
/// Once the state transition itself succeeds, resolving is the only part
/// that can't be undone or retried — notifying the lead and posting to
/// Linear are both best-effort from that point on, each reported in
/// [`ResolveOutcome`] rather than failing the whole call: a bus hiccup or a
/// Linear outage must never strand an escalation that's already resolved in
/// an error state with no visible outcome.
///
/// # Errors
///
/// [`OpsError::InvalidHuman`] when `human` doesn't parse, or
/// [`OpsError::DecisionTooLong`] when the rendered
/// lead notification would exceed [`relay_core::MAX_MESSAGE_BYTES`], and
/// [`relay_core::Error::UnsafeMessageText`] (as [`OpsError::Relay`]) when the
/// decision holds a character [`relay_core::text::is_unsafe_char`] rejects
/// (a CR from a CRLF `$(cat file)` included) — both checked before any
/// write. Otherwise propagates [`relay_core::Error`]
/// (not found / already resolved) unchanged.
pub fn run_resolve(
    relay: &mut Relay,
    linear: &impl Linear,
    id: &EscalationId,
    decision: &str,
    human: &str,
    now_ms: i64,
) -> Result<ResolveOutcome> {
    let esc = relay.get(id)?;
    let human_name: AgentName = human.parse().map_err(|reason| OpsError::InvalidHuman {
        human: human.to_owned(),
        reason: Box::new(reason),
    })?;
    let notify_text = format!("escalation {} resolved: {decision}", esc.id);
    if notify_text.len() > relay_core::MAX_MESSAGE_BYTES {
        return Err(OpsError::DecisionTooLong {
            lead: esc.filing.lead,
            bytes: notify_text.len(),
        });
    }

    // Checked before the irreversible transition, like the size: `send` would
    // refuse this text later and the lead would silently never be told.
    if let Some((byte_offset, c)) = relay_core::text::first_unsafe(&notify_text) {
        return Err(relay_core::Error::UnsafeMessageText {
            codepoint: u32::from(c),
            byte_offset,
        }
        .into());
    }

    let esc = relay.resolve(id, decision)?;
    let notify = Message {
        from: human_name,
        to: Target::Agent(esc.filing.lead.clone()),
        text: notify_text,
        sent_at_ms: now_ms,
        forwarded: false,
    };
    let lead_notified = relay.send(&notify).is_ok();

    let body = render_linear_comment(&esc, decision);
    let linear = match linear.comment(esc.filing.ticket.as_str(), &body) {
        Ok(()) => LinearOutcome::Posted,
        Err(e) => LinearOutcome::Failed(e),
    };

    Ok(ResolveOutcome {
        escalation: esc,
        lead_notified,
        linear,
    })
}

/// Prefix used by [`resolution_comment_marker`] for machine-readable resolution comments.
pub const RESOLUTION_COMMENT_MARKER_PREFIX: &str = "<!-- ix-relay-resolution:";

/// Returns the stable marker identifying a Linear resolution comment for `id`.
///
/// The marker is stable for one [`EscalationId`] and is appended to comments
/// posted by [`run_resolve`] and [`run_escalation_post_linear`]. Escalation ids
/// are unique within one Relay key prefix; consumers aggregating multiple
/// buses must also scope matching by source bus.
#[must_use]
pub fn resolution_comment_marker(id: &EscalationId) -> String {
    format!("{RESOLUTION_COMMENT_MARKER_PREFIX}{id} -->")
}

/// The Markdown comment `resolve`/`escalation post-linear` posts to the
/// Linear ticket: the escalation id and decision up front, the fuller
/// context in a collapsible section so the ticket's comment thread stays
/// scannable.
fn render_linear_comment(esc: &Escalation, decision: &str) -> String {
    format!(
        "**Escalation {} resolved:** {decision}\n\n<details>\n<summary>Context (from {})</summary>\n\n{}\n\n**Options:** {}\n\n**Recommendation:** {}\n\n</details>\n{}\n",
        esc.id,
        esc.filing.lead,
        esc.filing.context,
        esc.filing.options.join(", "),
        esc.filing.recommendation,
        resolution_comment_marker(&esc.id)
    )
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, time::Duration};

    use relay_core::{Registration, TabId};

    use super::*;
    use crate::herdr::{CreatedTab, ProcessError};

    fn relay(test: &str) -> Option<Relay> {
        let url = std::env::var("IX_RELAY_TEST_URL").ok()?;
        let relay = Relay::connect(&url, Duration::from_secs(2)).expect("connect to test redis");
        Some(relay.with_prefix(format!("test:agent-msg:{}:{test}:", std::process::id())))
    }

    fn name(s: &str) -> AgentName {
        s.parse().unwrap()
    }

    fn register(
        relay: &mut Relay,
        n: &str,
        machine: &str,
        session_id: Option<&str>,
        cwd: Option<&str>,
    ) {
        relay
            .register(&Registration {
                name: name(n),
                machine: machine.parse().unwrap(),
                kind: "claude".into(),
                session_id: session_id.map(|s| s.parse().unwrap()),
                cwd: cwd.map(str::to_owned),
                herdr_pane: Some("w1:p1".parse().unwrap()),
                place: None,
                last_seen_ms: 1,
            })
            .unwrap();
    }

    fn escalate_args() -> EscalateArgs {
        EscalateArgs {
            ticket: "AGE-2014".into(),
            context: "which eviction policy?".into(),
            options: vec!["noeviction".into(), "allkeys-lru".into()],
            recommendation: "noeviction".into(),
            human: "operator".into(),
        }
    }

    // ---- escalate -----------------------------------------------------

    #[test]
    fn escalate_refuses_an_unregistered_lead() {
        let Some(mut r) = relay("escalate-unregistered") else {
            return;
        };
        let err = run_escalate(&mut r, &name("qsl-lead"), escalate_args(), 1).unwrap_err();
        assert!(
            matches!(&err, OpsError::NotRegistered { agent } if agent.as_str() == "qsl-lead"),
            "{err:?}"
        );
    }

    #[test]
    fn escalate_refuses_a_lead_with_no_session_id() {
        let Some(mut r) = relay("escalate-no-session") else {
            return;
        };
        register(&mut r, "qsl-lead", "node-a", None, Some("/work"));
        let err = run_escalate(&mut r, &name("qsl-lead"), escalate_args(), 1).unwrap_err();
        assert!(
            matches!(&err, OpsError::NoSessionId { agent } if agent.as_str() == "qsl-lead"),
            "{err:?}"
        );
    }

    /// A lead whose own record this binary cannot read gets a clear message, not
    /// a corrupt-record error (AGE-2027 second-round review).
    #[test]
    fn escalate_with_an_unreadable_own_record_says_so() {
        let Some(mut r) = relay("escalate-unreadable") else {
            return;
        };
        let url = std::env::var("IX_RELAY_TEST_URL").expect("set: relay returned Some");
        let mut raw = redis::Client::open(url)
            .and_then(|c| c.get_connection())
            .expect("raw connection");
        redis::cmd("HSET")
            .arg(format!("test:agent-msg:{}:escalate-unreadable:agents", std::process::id()))
            .arg("qsl-lead")
            .arg(r#"{"name":"qsl-lead","machine":"node-a","kind":"claude","session_id":"","cwd":null,"herdr_pane":null,"place":null,"last_seen_ms":1}"#)
            .exec(&mut raw)
            .expect("plant");
        let err = run_escalate(&mut r, &name("qsl-lead"), escalate_args(), 1).unwrap_err();
        assert!(
            matches!(&err, OpsError::RegistrationUnreadable { agent } if agent.as_str() == "qsl-lead"),
            "{err:?}"
        );
        assert!(err.to_string().contains("unreadable"), "{err}");
    }

    #[test]
    fn escalate_stores_the_lead_snapshot_and_sends_a_short_notice_only() {
        let Some(mut r) = relay("escalate-ok") else {
            return;
        };
        register(&mut r, "qsl-lead", "node-a", Some("sid-1"), Some("/work"));
        register(&mut r, "operator", "node-a", Some("sid-human"), None);
        let outcome = run_escalate(&mut r, &name("qsl-lead"), escalate_args(), 1).unwrap();
        assert!(outcome.human_notify_warning.is_none());
        let id = outcome.escalation.id.clone();
        assert!(id.as_str().starts_with('e'), "{id:?}");

        let esc = r.get(&id).unwrap();
        assert_eq!(esc.filing.machine.as_str(), "node-a");
        assert_eq!(esc.filing.session_id.as_str(), "sid-1");
        assert_eq!(esc.filing.cwd.as_deref(), Some("/work"));
        assert_eq!(esc.status, EscalationStatus::Waiting);

        let popped = r
            .pop_inbox(&name("operator"), std::num::NonZeroUsize::new(10).unwrap())
            .unwrap();
        assert_eq!(popped.messages.len(), 1);
        let text = &popped.messages[0].text;
        assert!(text.contains(id.as_str()));
        assert!(text.contains("escalation show"));
        // The notice never carries the escalation's own free-form text.
        assert!(!text.contains("eviction policy"));
        assert!(!text.contains("noeviction"));
    }

    #[test]
    fn escalate_still_succeeds_even_if_the_human_inbox_is_not_registered() {
        let Some(mut r) = relay("escalate-no-human") else {
            return;
        };
        register(&mut r, "qsl-lead", "node-a", Some("sid-1"), Some("/work"));
        // "operator" was never registered: notify_human bypasses the
        // registered-recipient check `send` would otherwise enforce, so
        // this must not fail the whole command — but the human inbox
        // registration doesn't gate notify_human at all, only the target
        // AgentName parsing does, so this is still expected to succeed with
        // no warning and an actual delivery below.
        let outcome = run_escalate(&mut r, &name("qsl-lead"), escalate_args(), 1).unwrap();
        assert!(outcome.human_notify_warning.is_none());
        assert_eq!(
            r.get(&outcome.escalation.id).unwrap().status,
            EscalationStatus::Waiting
        );
        // And it actually was delivered, unlike `send` to an unknown agent.
        let popped = r
            .pop_inbox(&name("operator"), std::num::NonZeroUsize::new(10).unwrap())
            .unwrap();
        assert_eq!(popped.messages.len(), 1);
    }

    #[test]
    fn escalate_reports_an_unparsable_human_as_a_warning_not_an_error() {
        let Some(mut r) = relay("escalate-bad-human") else {
            return;
        };
        register(&mut r, "qsl-lead", "node-a", Some("sid-1"), Some("/work"));
        let mut args = escalate_args();
        args.human = "Not Valid".into();
        let outcome = run_escalate(&mut r, &name("qsl-lead"), args, 1).unwrap();
        let warning = outcome
            .human_notify_warning
            .expect("a warning, not an error");
        assert!(
            matches!(&warning, HumanNotifyWarning::InvalidHuman { human, reason }
                if human == "Not Valid"
                    && matches!(**reason, relay_core::Error::InvalidName { .. })),
            "{warning:?}"
        );
    }

    // ---- escalations ----------------------------------------------------

    #[test]
    fn escalations_lists_ids_status_and_ticket() {
        let Some(mut r) = relay("escalations-list") else {
            return;
        };
        register(&mut r, "qsl-lead", "node-a", Some("sid-1"), Some("/work"));
        run_escalate(&mut r, &name("qsl-lead"), escalate_args(), 1).unwrap();
        let outcome = run_escalations(&mut r, None).unwrap();
        assert!(outcome.rejected.is_empty());
        assert_eq!(outcome.escalations.len(), 1);
        assert_eq!(outcome.escalations[0].filing.ticket.as_str(), "AGE-2014");
        assert_eq!(outcome.escalations[0].filing.lead, name("qsl-lead"));
    }

    #[test]
    fn escalations_with_no_matches_is_an_empty_list_not_an_error() {
        let Some(mut r) = relay("escalations-empty") else {
            return;
        };
        let outcome = run_escalations(&mut r, Some(EscalationStatus::Resolved)).unwrap();
        assert!(outcome.escalations.is_empty());
        assert!(outcome.rejected.is_empty());
    }

    // ---- escalation show / post-linear -----------------------------------

    #[test]
    fn escalation_show_and_render_detail_include_context_options_and_recommendation() {
        let Some(mut r) = relay("escalation-show") else {
            return;
        };
        register(&mut r, "qsl-lead", "node-a", Some("sid-1"), Some("/work"));
        run_escalate(&mut r, &name("qsl-lead"), escalate_args(), 1).unwrap();
        let id = r.list(None).unwrap().escalations[0].id.clone();

        let esc = run_escalation_show(&mut r, &id).unwrap();
        let printed = render_escalation_detail(&esc);
        assert!(printed.contains("which eviction policy?"));
        assert!(printed.contains("noeviction"));
        assert!(printed.contains("allkeys-lru"));
        assert!(printed.contains("AGE-2014"));
        assert!(printed.contains("qsl-lead"));
    }

    #[test]
    fn post_linear_refuses_an_unresolved_escalation() {
        let Some(mut r) = relay("post-linear-unresolved") else {
            return;
        };
        let esc = escalate_for_resolve(&mut r);
        let linear = FakeLinear {
            fail: false,
            calls: RefCell::default(),
        };
        let err = run_escalation_post_linear(&mut r, &linear, &esc.id).unwrap_err();
        assert!(
            matches!(&err, OpsError::NoDecisionYet { id, status } if *id == esc.id && *status == EscalationStatus::Waiting),
            "{err:?}"
        );
        assert!(linear.calls.borrow().is_empty());
    }

    #[test]
    fn post_linear_retries_a_resolved_escalations_comment() {
        let Some(mut r) = relay("post-linear-retry") else {
            return;
        };
        let esc = escalate_for_resolve(&mut r);
        r.resolve(&esc.id, "go with a").unwrap();
        let linear = FakeLinear {
            fail: false,
            calls: RefCell::default(),
        };
        let posted = run_escalation_post_linear(&mut r, &linear, &esc.id).unwrap();
        assert_eq!(posted.id, esc.id);
        let calls = linear.calls.borrow();
        assert_eq!(calls[0].0, "AGE-2014");
        assert!(calls[0].1.contains("go with a"));
        assert!(
            calls[0]
                .1
                .ends_with(&format!("{}\n", resolution_comment_marker(&esc.id)))
        );
    }

    #[test]
    fn resolution_comment_marker_is_stable_and_machine_readable() {
        let id: EscalationId = "e123".parse().unwrap();
        assert_eq!(
            resolution_comment_marker(&id),
            "<!-- ix-relay-resolution:e123 -->"
        );
        assert_eq!(
            resolution_comment_marker(&id),
            resolution_comment_marker(&id)
        );
    }

    // ---- investigate ------------------------------------------------------

    /// `(machine, name, kind, pane, args)` recorded from one `agent_start` call.
    type AgentStartCall = (Option<String>, String, String, String, Vec<String>);

    #[derive(Default)]
    struct FakeHerdr {
        fail_tab_create: bool,
        fail_agent_start: bool,
        fail_tab_close: bool,
        tab_create_calls: RefCell<Vec<(Option<String>, String, String)>>,
        agent_start_calls: RefCell<Vec<AgentStartCall>>,
        tab_close_calls: RefCell<Vec<(Option<String>, String)>>,
        next_tab_id: RefCell<u32>,
    }

    impl Herdr for FakeHerdr {
        fn tab_create(
            &self,
            machine: Option<&str>,
            cwd: &str,
            label: &str,
        ) -> std::result::Result<CreatedTab, ProcessError> {
            self.tab_create_calls.borrow_mut().push((
                machine.map(str::to_owned),
                cwd.to_owned(),
                label.to_owned(),
            ));
            if self.fail_tab_create {
                return Err(ProcessError::Reported {
                    code: "repository_not_trusted".into(),
                    message: "untrusted".into(),
                });
            }
            let mut n = self.next_tab_id.borrow_mut();
            *n += 1;
            Ok(CreatedTab {
                pane_id: format!("w1:p{n}").parse().unwrap(),
                tab_id: format!("w1:t{n}").parse().unwrap(),
            })
        }

        fn agent_start(
            &self,
            machine: Option<&str>,
            name: &str,
            kind: &str,
            pane: &PaneId,
            args: &[&str],
        ) -> std::result::Result<(), ProcessError> {
            self.agent_start_calls.borrow_mut().push((
                machine.map(str::to_owned),
                name.to_owned(),
                kind.to_owned(),
                pane.to_string(),
                args.iter().map(|s| (*s).to_owned()).collect(),
            ));
            if self.fail_agent_start {
                return Err(ProcessError::Reported {
                    code: "agent_not_ready".into(),
                    message: "blocked".into(),
                });
            }
            Ok(())
        }

        fn tab_close(
            &self,
            machine: Option<&str>,
            tab_id: &TabId,
        ) -> std::result::Result<(), ProcessError> {
            self.tab_close_calls
                .borrow_mut()
                .push((machine.map(str::to_owned), tab_id.to_string()));
            if self.fail_tab_close {
                return Err(ProcessError::Reported {
                    code: "tab_not_found".into(),
                    message: "already gone".into(),
                });
            }
            Ok(())
        }
    }

    fn escalate_for_investigate(
        r: &mut Relay,
        lead: &str,
        machine: &str,
        kind: &str,
    ) -> Escalation {
        r.register(&Registration {
            name: name(lead),
            machine: machine.parse().unwrap(),
            kind: kind.into(),
            session_id: Some("sid-fork".parse().unwrap()),
            cwd: Some("/work/lead".into()),
            herdr_pane: Some("w1:p1".parse().unwrap()),
            place: None,
            last_seen_ms: 1,
        })
        .unwrap();
        r.escalate(EscalationFiling {
            ticket: "AGE-2014".parse().unwrap(),
            lead: name(lead),
            machine: machine.parse().unwrap(),
            session_id: "sid-fork".parse().unwrap(),
            cwd: Some("/work/lead".into()),
            kind: kind.into(),
            context: "which-lead-supplied-context-text".into(),
            options: vec!["a".into(), "b".into()],
            recommendation: "lead-supplied-recommendation-text".into(),
            created_at_ms: 1,
        })
        .unwrap()
    }

    #[test]
    fn investigate_forks_on_the_local_machine_with_no_machine_flag_and_sets_investigating() {
        let Some(mut r) = relay("investigate-local-fork") else {
            return;
        };
        let esc = escalate_for_investigate(&mut r, "qsl-lead", "node-a", "claude");
        let herdr = FakeHerdr::default();
        let outcome =
            run_investigate(&mut r, &herdr, &"node-a".parse().unwrap(), &esc.id, false).unwrap();
        assert_eq!(outcome.escalation_id, esc.id.as_str());
        assert!(!outcome.fresh);
        assert_eq!(outcome.session_id.as_str(), "sid-fork");
        assert_eq!(outcome.machine.as_str(), "node-a");

        let tab_calls = herdr.tab_create_calls.borrow();
        assert_eq!(tab_calls[0].0, None, "same machine: no --machine flag");
        assert_eq!(tab_calls[0].1, "/work/lead");
        assert_eq!(
            tab_calls[0].2,
            format!("esc-{}-1", esc.id),
            "name includes the attempt number"
        );

        let start_calls = herdr.agent_start_calls.borrow();
        assert_eq!(start_calls[0].0, None);
        assert_eq!(start_calls[0].2, "claude");
        assert_eq!(
            start_calls[0].4,
            vec!["--resume", "sid-fork", "--fork-session"]
        );

        assert_eq!(
            r.get(&esc.id).unwrap().status,
            EscalationStatus::Investigating
        );
        assert!(
            herdr.tab_close_calls.borrow().is_empty(),
            "no cleanup on success"
        );
    }

    #[test]
    fn investigate_forks_on_a_remote_machine_with_the_machine_flag() {
        let Some(mut r) = relay("investigate-remote-fork") else {
            return;
        };
        let esc = escalate_for_investigate(&mut r, "qsl-lead", "hub", "claude");
        let herdr = FakeHerdr::default();
        run_investigate(&mut r, &herdr, &"node-a".parse().unwrap(), &esc.id, false).unwrap();

        assert_eq!(herdr.tab_create_calls.borrow()[0].0.as_deref(), Some("hub"));
        assert_eq!(
            herdr.agent_start_calls.borrow()[0].0.as_deref(),
            Some("hub")
        );
    }

    #[test]
    fn investigate_refuses_to_fork_a_non_claude_lead_without_creating_a_tab_but_fresh_still_works()
    {
        let Some(mut r) = relay("investigate-non-claude") else {
            return;
        };
        let esc = escalate_for_investigate(&mut r, "codex-lead", "node-a", "codex");
        let herdr = FakeHerdr::default();

        let err = run_investigate(&mut r, &herdr, &"node-a".parse().unwrap(), &esc.id, false)
            .unwrap_err();
        assert!(
            matches!(&err, OpsError::ForkNeedsClaude { id, kind } if *id == esc.id && kind != "claude"),
            "{err:?}"
        );
        assert!(
            herdr.tab_create_calls.borrow().is_empty(),
            "refused before opening any tab"
        );
        assert_eq!(r.get(&esc.id).unwrap().status, EscalationStatus::Waiting);

        let outcome =
            run_investigate(&mut r, &herdr, &"node-a".parse().unwrap(), &esc.id, true).unwrap();
        assert!(outcome.fresh);
        assert_eq!(outcome.kind, "codex");
        assert_eq!(herdr.agent_start_calls.borrow()[0].2, "codex");
        assert_eq!(
            r.get(&esc.id).unwrap().status,
            EscalationStatus::Investigating
        );
    }

    #[test]
    fn investigate_fresh_prompt_contains_only_validated_fields_never_lead_supplied_text() {
        let Some(mut r) = relay("investigate-fresh") else {
            return;
        };
        let esc = escalate_for_investigate(&mut r, "qsl-lead", "node-a", "claude");
        let herdr = FakeHerdr::default();
        run_investigate(&mut r, &herdr, &"node-a".parse().unwrap(), &esc.id, true).unwrap();

        // Nothing is ever written into the lead's cwd or typed through a
        // pane for --fresh: tab_create is the only herdr call besides
        // agent_start.
        let start_calls = herdr.agent_start_calls.borrow();
        assert_eq!(start_calls[0].4.len(), 1, "one prompt arg");
        let prompt = &start_calls[0].4[0];
        assert!(prompt.contains(esc.id.as_str()));
        assert!(prompt.contains("qsl-lead"));
        assert!(prompt.contains("AGE-2014"));
        assert!(prompt.contains("escalation show"));
        // The lead's own free-form text never appears in the prompt.
        assert!(!prompt.contains(esc.filing.context.as_str()));
        assert!(!prompt.contains(esc.filing.recommendation.as_str()));
    }

    #[test]
    fn investigate_does_not_mark_investigating_when_herdr_fails_and_closes_the_tab() {
        let Some(mut r) = relay("investigate-herdr-fails") else {
            return;
        };
        let esc = escalate_for_investigate(&mut r, "qsl-lead", "node-a", "claude");
        let herdr = FakeHerdr {
            fail_agent_start: true,
            ..FakeHerdr::default()
        };
        let err = run_investigate(&mut r, &herdr, &"node-a".parse().unwrap(), &esc.id, false)
            .unwrap_err();
        assert!(matches!(err, OpsError::Herdr(_)), "{err:?}");
        assert_eq!(r.get(&esc.id).unwrap().status, EscalationStatus::Waiting);
        assert_eq!(
            herdr.tab_close_calls.borrow().len(),
            1,
            "the dangling tab is cleaned up"
        );
        assert_eq!(herdr.tab_close_calls.borrow()[0].1, "w1:t1");
    }

    /// AGE-2015 PR review: a failed `tab_close` after a failed `agent_start`
    /// must not be silently swallowed behind the original error — both are
    /// named in one [`OpsError::InvestigateCleanupFailed`].
    #[test]
    fn investigate_reports_both_failures_when_cleanup_also_fails() {
        let Some(mut r) = relay("investigate-cleanup-also-fails") else {
            return;
        };
        let esc = escalate_for_investigate(&mut r, "qsl-lead", "node-a", "claude");
        let herdr = FakeHerdr {
            fail_agent_start: true,
            fail_tab_close: true,
            ..FakeHerdr::default()
        };
        let err = run_investigate(&mut r, &herdr, &"node-a".parse().unwrap(), &esc.id, false)
            .unwrap_err();
        let OpsError::InvestigateCleanupFailed {
            start,
            tab_id,
            cleanup,
        } = &err
        else {
            panic!("expected InvestigateCleanupFailed, got {err:?}");
        };
        assert!(
            matches!(**start, ProcessError::Reported { ref code, .. } if code == "agent_not_ready")
        );
        assert_eq!(tab_id.as_str(), "w1:t1");
        assert!(
            matches!(**cleanup, ProcessError::Reported { ref code, .. } if code == "tab_not_found")
        );
        let rendered = err.to_string();
        assert!(rendered.contains("agent_not_ready") || rendered.contains("blocked"));
        assert!(rendered.contains("tab_not_found") || rendered.contains("already gone"));
    }

    #[test]
    fn investigate_names_are_unique_across_repeated_attempts() {
        let Some(mut r) = relay("investigate-unique-names") else {
            return;
        };
        let esc = escalate_for_investigate(&mut r, "qsl-lead", "node-a", "claude");
        let herdr = FakeHerdr::default();
        run_investigate(&mut r, &herdr, &"node-a".parse().unwrap(), &esc.id, false).unwrap();
        run_investigate(&mut r, &herdr, &"node-a".parse().unwrap(), &esc.id, false).unwrap();

        let starts = herdr.agent_start_calls.borrow();
        assert_eq!(starts.len(), 2);
        assert_eq!(starts[0].1, format!("esc-{}-1", esc.id));
        assert_eq!(starts[1].1, format!("esc-{}-2", esc.id));
        assert_ne!(starts[0].1, starts[1].1);
    }

    #[test]
    fn investigate_refuses_an_escalation_with_no_recorded_cwd() {
        let Some(mut r) = relay("investigate-no-cwd") else {
            return;
        };
        let esc = r
            .escalate(EscalationFiling {
                ticket: "AGE-2014".parse().unwrap(),
                lead: name("qsl-lead"),
                machine: "node-a".parse().unwrap(),
                session_id: "sid-fork".parse().unwrap(),
                cwd: None,
                kind: "claude".into(),
                context: "c".into(),
                options: vec![],
                recommendation: "r".into(),
                created_at_ms: 1,
            })
            .unwrap();
        let herdr = FakeHerdr::default();
        let err = run_investigate(&mut r, &herdr, &"node-a".parse().unwrap(), &esc.id, false)
            .unwrap_err();
        assert!(
            matches!(&err, OpsError::NoRecordedCwd { id } if *id == esc.id),
            "{err:?}"
        );
        assert!(herdr.tab_create_calls.borrow().is_empty());
    }

    #[test]
    fn resolve_refuses_an_unparsable_human_before_any_write() {
        let Some(mut r) = relay("resolve-bad-human") else {
            return;
        };
        let esc = escalate_for_resolve(&mut r);
        let linear = FakeLinear {
            fail: false,
            calls: RefCell::default(),
        };
        let err = run_resolve(&mut r, &linear, &esc.id, "ok", "Not Valid", 2).unwrap_err();
        assert!(
            matches!(&err, OpsError::InvalidHuman { human, reason }
                if human == "Not Valid"
                    && matches!(**reason, relay_core::Error::InvalidName { .. })),
            "{err:?}"
        );
        assert_eq!(r.get(&esc.id).unwrap().status, EscalationStatus::Waiting);
    }

    #[test]
    fn investigate_refuses_an_already_resolved_escalation() {
        let Some(mut r) = relay("investigate-resolved") else {
            return;
        };
        let esc = escalate_for_investigate(&mut r, "qsl-lead", "node-a", "claude");
        r.resolve(&esc.id, "decision").unwrap();
        let herdr = FakeHerdr::default();
        let err = run_investigate(&mut r, &herdr, &"node-a".parse().unwrap(), &esc.id, false)
            .unwrap_err();
        assert!(
            matches!(&err, OpsError::NothingToInvestigate { id } if *id == esc.id),
            "{err:?}"
        );
        assert!(herdr.tab_create_calls.borrow().is_empty());
    }

    // ---- resolve --------------------------------------------------------

    struct FakeLinear {
        fail: bool,
        calls: RefCell<Vec<(String, String)>>,
    }

    impl Linear for FakeLinear {
        fn comment(&self, ticket: &str, body: &str) -> std::result::Result<(), LinearError> {
            self.calls
                .borrow_mut()
                .push((ticket.to_owned(), body.to_owned()));
            if self.fail {
                Err(LinearError::Failed {
                    status: 1,
                    stderr: "not logged in".into(),
                })
            } else {
                Ok(())
            }
        }
    }

    fn escalate_for_resolve(r: &mut Relay) -> Escalation {
        register(r, "qsl-lead", "node-a", Some("sid-1"), Some("/work"));
        r.escalate(EscalationFiling {
            ticket: "AGE-2014".parse().unwrap(),
            lead: name("qsl-lead"),
            machine: "node-a".parse().unwrap(),
            session_id: "sid-1".parse().unwrap(),
            cwd: Some("/work".into()),
            kind: "claude".into(),
            context: "ctx".into(),
            options: vec!["a".into()],
            recommendation: "a".into(),
            created_at_ms: 1,
        })
        .unwrap()
    }

    #[test]
    fn resolve_marks_resolved_notifies_the_lead_and_posts_to_linear() {
        let Some(mut r) = relay("resolve-ok") else {
            return;
        };
        let esc = escalate_for_resolve(&mut r);
        let linear = FakeLinear {
            fail: false,
            calls: RefCell::default(),
        };
        let outcome = run_resolve(&mut r, &linear, &esc.id, "go with a", "operator", 2).unwrap();
        assert!(outcome.lead_notified);
        assert!(matches!(outcome.linear, LinearOutcome::Posted));

        assert_eq!(r.get(&esc.id).unwrap().status, EscalationStatus::Resolved);
        let popped = r
            .pop_inbox(&name("qsl-lead"), std::num::NonZeroUsize::new(10).unwrap())
            .unwrap();
        assert_eq!(popped.messages.len(), 1);
        assert_eq!(popped.messages[0].from, name("operator"));
        assert!(popped.messages[0].text.contains("go with a"));

        let calls = linear.calls.borrow();
        assert_eq!(calls[0].0, "AGE-2014");
        assert!(calls[0].1.contains(esc.id.as_str()));
        assert!(calls[0].1.contains("go with a"));
        assert!(calls[0].1.contains("<details>"), "context is collapsible");
    }

    #[test]
    fn resolve_stays_resolved_and_still_notifies_but_reports_a_failed_linear_post() {
        let Some(mut r) = relay("resolve-linear-fails") else {
            return;
        };
        let esc = escalate_for_resolve(&mut r);
        let linear = FakeLinear {
            fail: true,
            calls: RefCell::default(),
        };
        // Per this module's own doc comment on `run_resolve`: a Linear
        // failure is reported in the outcome, not a command failure, once
        // the resolution and the lead notification have already succeeded.
        let outcome = run_resolve(&mut r, &linear, &esc.id, "go with a", "operator", 2).unwrap();
        assert!(outcome.lead_notified);
        assert!(matches!(outcome.linear, LinearOutcome::Failed(_)));

        assert_eq!(r.get(&esc.id).unwrap().status, EscalationStatus::Resolved);
        let popped = r
            .pop_inbox(&name("qsl-lead"), std::num::NonZeroUsize::new(10).unwrap())
            .unwrap();
        assert_eq!(
            popped.messages.len(),
            1,
            "the lead is told regardless of the Linear failure"
        );
    }

    /// AGE-2015 PR review: notifying the lead is best-effort too, once the
    /// escalation itself is resolved — an unregistered/deregistered lead
    /// must not turn an already-mutated resolve into a reported failure.
    #[test]
    fn resolve_stays_resolved_and_reports_a_failed_notify_when_the_lead_is_unregistered() {
        let Some(mut r) = relay("resolve-notify-fails") else {
            return;
        };
        let ticket = "AGE-2014".parse().unwrap();
        // File the escalation the normal way, then remove the lead's own
        // registration before resolving — `Relay::send` refuses an unknown
        // target, exactly what a deregistered lead would look like.
        let esc = r
            .escalate(EscalationFiling {
                ticket,
                lead: name("qsl-lead"),
                machine: "node-a".parse().unwrap(),
                session_id: "sid-1".parse().unwrap(),
                cwd: Some("/work".into()),
                kind: "claude".into(),
                context: "ctx".into(),
                options: vec!["a".into()],
                recommendation: "a".into(),
                created_at_ms: 1,
            })
            .unwrap();
        // Never registered "qsl-lead" on this Relay/prefix at all.
        let linear = FakeLinear {
            fail: false,
            calls: RefCell::default(),
        };
        let outcome = run_resolve(&mut r, &linear, &esc.id, "go with a", "operator", 2).unwrap();
        assert!(!outcome.lead_notified);
        assert!(matches!(outcome.linear, LinearOutcome::Posted));
        assert_eq!(
            r.get(&esc.id).unwrap().status,
            EscalationStatus::Resolved,
            "already resolved even though the lead couldn't be told"
        );
    }

    #[test]
    fn resolving_twice_the_second_call_is_refused_and_does_not_renotify() {
        let Some(mut r) = relay("resolve-twice") else {
            return;
        };
        let esc = escalate_for_resolve(&mut r);
        let linear = FakeLinear {
            fail: false,
            calls: RefCell::default(),
        };
        run_resolve(&mut r, &linear, &esc.id, "first", "operator", 2).unwrap();
        r.pop_inbox(&name("qsl-lead"), std::num::NonZeroUsize::new(10).unwrap())
            .unwrap();

        let err = run_resolve(&mut r, &linear, &esc.id, "second", "operator", 3).unwrap_err();
        assert!(
            matches!(&err, OpsError::Relay(relay_core::Error::EscalationAlreadyResolved(id)) if id == esc.id.as_str()),
            "{err:?}"
        );
        assert_eq!(linear.calls.borrow().len(), 1, "no second Linear post");
        assert!(
            r.pop_inbox(&name("qsl-lead"), std::num::NonZeroUsize::new(10).unwrap())
                .unwrap()
                .messages
                .is_empty(),
            "no second bus message either"
        );
    }

    /// An oversized decision is refused before `Relay::resolve` is ever
    /// called, so the escalation is left exactly as it was — not silently
    /// resolved with no one told.
    #[test]
    fn resolve_refuses_an_oversized_decision_and_leaves_the_escalation_unresolved() {
        let Some(mut r) = relay("resolve-oversized") else {
            return;
        };
        let esc = escalate_for_resolve(&mut r);
        let linear = FakeLinear {
            fail: false,
            calls: RefCell::default(),
        };
        let oversized = "x".repeat(relay_core::MAX_MESSAGE_BYTES + 1);
        let err = run_resolve(&mut r, &linear, &esc.id, &oversized, "operator", 2).unwrap_err();
        assert!(
            matches!(&err, OpsError::DecisionTooLong { bytes, lead } if *bytes > relay_core::MAX_MESSAGE_BYTES && lead.as_str() == "qsl-lead"),
            "{err:?}"
        );

        assert_eq!(r.get(&esc.id).unwrap().status, EscalationStatus::Waiting);
        assert!(linear.calls.borrow().is_empty());
        assert!(
            r.pop_inbox(&name("qsl-lead"), std::num::NonZeroUsize::new(10).unwrap())
                .unwrap()
                .messages
                .is_empty()
        );
    }

    #[test]
    fn resolve_also_leaves_investigating_escalations_unresolved_on_oversize() {
        let Some(mut r) = relay("resolve-oversized-investigating") else {
            return;
        };
        let esc = escalate_for_resolve(&mut r);
        r.set_investigating(&esc.id).unwrap();
        let linear = FakeLinear {
            fail: false,
            calls: RefCell::default(),
        };
        let oversized = "x".repeat(relay_core::MAX_MESSAGE_BYTES + 1);
        assert!(run_resolve(&mut r, &linear, &esc.id, &oversized, "operator", 2).is_err());
        assert_eq!(
            r.get(&esc.id).unwrap().status,
            EscalationStatus::Investigating
        );
    }

    /// AGE-2026: a decision `send` would refuse (a CR from a CRLF file, ESC)
    /// is refused before the irreversible transition, not swallowed after it.
    #[test]
    fn resolve_refuses_an_unsafe_decision_before_the_status_changes() {
        let Some(mut r) = relay("resolve-unsafe") else {
            return;
        };
        let esc = escalate_for_resolve(&mut r);
        let linear = FakeLinear {
            fail: false,
            calls: RefCell::default(),
        };
        for decision in ["go with a\r\n", "a\u{1b}[2J", "a\u{202e}b"] {
            let err = run_resolve(&mut r, &linear, &esc.id, decision, "operator", 2).unwrap_err();
            assert!(
                matches!(
                    err,
                    OpsError::Relay(relay_core::Error::UnsafeMessageText { .. })
                ),
                "{decision:?}: {err:?}"
            );
        }
        assert_eq!(r.get(&esc.id).unwrap().status, EscalationStatus::Waiting);
        assert!(linear.calls.borrow().is_empty());
    }

    fn sample_escalation_for_render() -> Escalation {
        Escalation {
            id: "e1".parse().unwrap(),
            filing: EscalationFiling {
                ticket: "AGE-1".parse().unwrap(),
                lead: name("qsl-lead"),
                machine: "node-a".parse().unwrap(),
                session_id: "sid-1".parse().unwrap(),
                cwd: Some("/work".into()),
                kind: "claude".into(),
                context: "ctx".into(),
                options: vec!["a".into()],
                recommendation: "a".into(),
                created_at_ms: 1,
            },
            status: EscalationStatus::Waiting,
            decision: None,
        }
    }

    #[test]
    fn the_detail_rendering_neutralises_escape_and_bidi_in_every_field() {
        let mut esc = sample_escalation_for_render();
        esc.filing.cwd = Some("/w\u{202e}ork\n[forged]".into());
        esc.filing.kind = "cl\u{200b}aude".into();
        esc.filing.context = "c\u{1b}]0;x\u{7}\nline".into();
        esc.filing.options = vec!["o\u{85}pt".into()];
        esc.filing.recommendation = "r\u{202e}ec".into();
        esc.decision = Some("d\u{1b}ec".into());
        let out = render_escalation_detail(&esc);
        assert!(relay_core::text::first_unsafe(&out).is_none(), "{out:?}");
        assert!(out.contains("machine:        node-a"), "{out}");
        assert!(
            out.contains("cwd:            /w\u{fffd}ork\u{fffd}[forged]"),
            "{out}"
        );
    }
}
