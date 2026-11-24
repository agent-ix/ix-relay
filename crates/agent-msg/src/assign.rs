// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! `agent-msg assign`: write and read the ordered per-lane assignment list (FR-072).
//! Writes need the orchestrator role of `[assign]` in `config.toml`; reads need nothing.

use std::{io::Write, str::FromStr};

use clap::{Args, Subcommand};
use relay_core::{
    Relay,
    assignment::{AssignWrite, AssignWriter, AssignedTicket},
};

use crate::BoxError;

#[derive(Args)]
pub(crate) struct AssignArgs {
    #[command(subcommand)]
    command: AssignCommand,
}

/// A ticket given as `ID=REF`: its id and the text reference that locates it.
#[derive(Clone)]
pub(crate) struct TicketArg(AssignedTicket);

impl FromStr for TicketArg {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        let (id, reference) = text
            .split_once('=')
            .ok_or("expected ID=REF, e.g. SWM-259=https://linear.app/...")?;
        Ok(Self(AssignedTicket {
            id: id.to_owned(),
            reference: reference.to_owned(),
        }))
    }
}

#[derive(Subcommand)]
enum AssignCommand {
    /// Replace a lane's list (orchestrator only).
    Set {
        /// Lane address server:workspace:tab.
        lane: String,
        /// A ticket as ID=REF, in order; repeatable. None clears the list.
        #[arg(long = "ticket", value_name = "ID=REF")]
        tickets: Vec<TicketArg>,
        /// The order that carries this write, stored in the transition.
        #[arg(long, value_name = "ID")]
        order_id: Option<String>,
    },
    /// Add tickets after the last one (orchestrator only).
    Append {
        /// Lane address server:workspace:tab.
        lane: String,
        /// A ticket as ID=REF; repeatable.
        #[arg(long = "ticket", value_name = "ID=REF", required = true)]
        tickets: Vec<TicketArg>,
        /// The order that carries this write, stored in the transition.
        #[arg(long, value_name = "ID")]
        order_id: Option<String>,
    },
    /// Remove one ticket (orchestrator only).
    Remove {
        /// Lane address server:workspace:tab.
        lane: String,
        /// The ticket id.
        id: String,
        /// The order that carries this write, stored in the transition.
        #[arg(long, value_name = "ID")]
        order_id: Option<String>,
    },
    /// Put the tickets in a new order: every id of the list, once (orchestrator only).
    Reorder {
        /// Lane address server:workspace:tab.
        lane: String,
        /// The ticket ids in the new order.
        #[arg(required = true, num_args = 1..)]
        ids: Vec<String>,
        /// The order that carries this write, stored in the transition.
        #[arg(long, value_name = "ID")]
        order_id: Option<String>,
    },
    /// Print one lane's list, or every lane's, as JSON. Changes nothing.
    Show {
        /// Lane address server:workspace:tab; omitted means all lanes.
        lane: Option<String>,
    },
    /// Print the bounded transition history as JSON. Changes nothing.
    History {
        /// Exclusive cursor from the previous page.
        #[arg(long, default_value = "0-0")]
        after: String,
        /// Most transitions in the page (1..1000).
        #[arg(long, default_value_t = 500)]
        limit: usize,
    },
}

pub(crate) fn run(
    args: AssignArgs,
    relay: &mut Relay,
    settings: &relay_core::config::CoreSettings,
    out: &mut impl Write,
) -> Result<(), BoxError> {
    let (lane, write, order_id) = match args.command {
        AssignCommand::Show { lane } => {
            match lane {
                Some(lane) => crate::json::write_line(out, &relay.assignment(&lane)?)?,
                None => crate::json::write_line(out, &relay.assignments()?)?,
            }
            return Ok(());
        }
        AssignCommand::History { after, limit } => {
            crate::json::write_line(out, &relay.assignment_events(&after, limit)?)?;
            return Ok(());
        }
        AssignCommand::Set {
            lane,
            tickets,
            order_id,
        } => (
            lane,
            AssignWrite::Set(tickets.into_iter().map(|t| t.0).collect()),
            order_id,
        ),
        AssignCommand::Append {
            lane,
            tickets,
            order_id,
        } => (
            lane,
            AssignWrite::Append(tickets.into_iter().map(|t| t.0).collect()),
            order_id,
        ),
        AssignCommand::Remove { lane, id, order_id } => (lane, AssignWrite::Remove(id), order_id),
        AssignCommand::Reorder {
            lane,
            ids,
            order_id,
        } => (lane, AssignWrite::Reorder(ids), order_id),
    };
    let writer = writer(relay, settings)?;
    let list = relay.assign(&writer, &lane, &write, order_id.as_deref())?;
    crate::json::write_line(out, &list)?;
    Ok(())
}

/// The caller as a writer: the role from `[assign]`, and the caller's own lane when its
/// session is a registered lead with a verified place.
fn writer(
    relay: &mut Relay,
    settings: &relay_core::config::CoreSettings,
) -> Result<AssignWriter, BoxError> {
    let file =
        relay_core::config::load_file()?.unwrap_or(relay_core::config::FileConfig::default());
    let machine = &settings.machine.value;
    let caller = if file.assign.is_orchestrator()? {
        crate::derive_or_hint(relay, machine)?.and_then(|id| {
            relay
                .agents()
                .ok()?
                .registrations
                .into_iter()
                .find(|r| r.name == id.name)
                .and_then(|r| r.verified_place().map(|p| p.address(&r.machine)))
        })
    } else {
        None
    };
    Ok(AssignWriter::from_config(&file.assign, machine, caller)?)
}
