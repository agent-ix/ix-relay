// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! Explicit session work reporting CLI; no message delivery or tracker writes.

use crate::BoxError;
use clap::{Args, Subcommand};
use ix_board::{LinearGraphQlSource, ReadService, ScopeSelection};
use relay_core::{
    Relay, SessionId, TicketId,
    work::{AgentWork, WorkFocus, WorkStatus},
};
use relay_ops::work::{resolve_work_scope, resolve_work_target};
use std::io::Write;

#[derive(Args)]
pub(crate) struct WorkArgs {
    /// Target lead's Herdr address; omitted means the calling pane.
    #[arg(long)]
    lead: Option<String>,
    /// Expected native session ID; refuse a different occupant.
    #[arg(long)]
    session: Option<SessionId>,
    #[command(subcommand)]
    command: WorkCommand,
}

#[derive(Subcommand)]
enum WorkCommand {
    /// Set the assigned scope, resolving selectors against Linear.
    Scope {
        /// Initiative UUID or exact name; repeatable.
        #[arg(long = "initiative")]
        initiatives: Vec<String>,
        /// Project UUID, identifier or exact name; repeatable.
        #[arg(long = "project")]
        projects: Vec<String>,
        /// Ticket UUID or identifier, including descendants; repeatable.
        #[arg(long = "ticket")]
        tickets: Vec<String>,
    },
    /// Report the lead's current focus and optionally related tickets.
    Focus {
        /// Short plain-language action.
        summary: String,
        /// Ticket identifier; repeatable.
        #[arg(long = "ticket")]
        tickets: Vec<TicketId>,
    },
    /// Report one agent's assignment or completion without marking a ticket done.
    Agent {
        /// Native agent ID, or stable assignment ID for a pane worker.
        id: String,
        /// Display name; defaults to the assignment ID.
        #[arg(long)]
        name: Option<String>,
        /// Ticket identifier.
        #[arg(long)]
        ticket: TicketId,
        /// Role such as coder, reviewer or researcher.
        #[arg(long)]
        role: String,
        /// Reported status: running, waiting, done, failed or canceled.
        #[arg(long, value_parser = parse_status)]
        status: WorkStatus,
        /// Herdr address for a separate pane worker; captures its native session.
        #[arg(long)]
        worker: Option<String>,
    },
    /// Remove a finished assignment from this session's bounded report.
    Remove {
        /// Agent assignment ID to remove.
        id: String,
    },
    /// Print the current session-qualified report as structured JSON.
    Show,
}

fn parse_status(value: &str) -> Result<WorkStatus, String> {
    serde_json::from_value(serde_json::Value::String(value.to_owned()))
        .map_err(|error| error.to_string())
}

fn source_snapshot() -> Result<ix_board::WorkspaceSnapshot, BoxError> {
    let mut service = ReadService::open_default(LinearGraphQlSource::from_env()?)?;
    Ok(service.workspace_snapshot()?)
}

fn verify_tickets(tickets: &[TicketId]) -> Result<(), BoxError> {
    if tickets.is_empty() {
        return Ok(());
    }
    ix_board::select_scope(
        &source_snapshot()?,
        &ScopeSelection {
            tickets: tickets.iter().map(|t| t.as_str().to_owned()).collect(),
            ..Default::default()
        },
    )?;
    Ok(())
}

pub(crate) fn run(
    args: WorkArgs,
    relay: &mut Relay,
    settings: &relay_core::config::CoreSettings,
    out: &mut impl Write,
) -> Result<(), BoxError> {
    let runner = herdr_cli::RealProcessRunner;
    let observation = resolve_work_target(&runner, args.lead.as_deref(), &settings.machine.value)?;
    let binding = observation.binding;
    if args.session.is_some_and(|id| id != binding.session) {
        return Err(relay_ops::OpsError::WorkSessionReplaced.into());
    }
    let now = crate::now_ms();
    match args.command {
        WorkCommand::Scope {
            initiatives,
            projects,
            tickets,
        } => {
            let scope = resolve_work_scope(
                &source_snapshot()?,
                &ScopeSelection {
                    initiatives,
                    projects,
                    tickets,
                },
                now,
            )?;
            relay.report_work_scope(&binding, &scope)?;
        }
        WorkCommand::Focus { summary, tickets } => {
            verify_tickets(&tickets)?;
            relay.report_work_focus(
                &binding,
                &WorkFocus {
                    summary,
                    tickets,
                    reported_at_ms: now,
                },
            )?;
        }
        WorkCommand::Agent {
            id,
            name,
            ticket,
            role,
            status,
            worker,
        } => {
            verify_tickets(std::slice::from_ref(&ticket))?;
            let worker = worker
                .as_deref()
                .map(|target| resolve_work_target(&runner, Some(target), &settings.machine.value))
                .transpose()?;
            if worker
                .as_ref()
                .is_some_and(|w| w.binding.machine != binding.machine)
            {
                return Err(relay_ops::OpsError::WorkWorkerMachine.into());
            }
            let agent = AgentWork {
                name: name.unwrap_or_else(|| id.clone()),
                agent_id: id,
                ticket,
                role,
                status,
                pane: worker.as_ref().map(|w| w.binding.pane.clone()),
                session: worker.map(|w| w.binding.session),
                reported_at_ms: now,
            };
            relay.report_work_agent(&binding, &agent)?;
        }
        WorkCommand::Remove { id } => relay.remove_work_agent(&binding, &id)?,
        WorkCommand::Show => {
            serde_json::to_writer(&mut *out, &relay.work_report(&binding)?)?;
            writeln!(out)?;
            return Ok(());
        }
    }
    writeln!(
        out,
        "reported for {} ({})",
        binding.address, binding.session
    )?;
    Ok(())
}
