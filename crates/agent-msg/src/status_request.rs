//! Fixed-form status requests, replies, and lane registry views.

use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
};

use clap::Args;
use relay_core::{
    AgentName, LaneState, MachineName, Message, StatusItem, StatusReply, StatusRequestId, Target,
};
use serde::Serialize;

use crate::{BoxError, Since, now_ms};

/// CLI fields for one fixed-form reply.
#[derive(Args)]
pub(super) struct StatusReplyArgs {
    /// Request ID from `status-request`.
    request_id: String,
    /// Current Linear ticket IDs; repeat for multiple tickets.
    #[arg(long = "ticket")]
    tickets: Vec<String>,
    /// Current pull request URLs or IDs; repeat for multiple PRs.
    #[arg(long = "pull-request")]
    pull_requests: Vec<String>,
    /// Blocking Linear ticket IDs; repeat for multiple blockers.
    #[arg(long = "blocker-ticket")]
    blocker_ticket_ids: Vec<String>,
    /// Lane state: active, idle, or blocked.
    #[arg(long, value_parser = ["active", "idle", "blocked"])]
    state: String,
    /// Number of coder slots currently free in this lane.
    #[arg(long)]
    spare_coder_slots: u32,
    /// Decisions required from the project owner; repeat if needed.
    #[arg(long = "owner-decision")]
    owner_decisions: Vec<String>,
}

/// Opens a request, snapshots its recipients, then enqueues one notice per lane.
pub(super) fn request(
    lanes: String,
    relay: &mut relay_core::Relay,
    machine: &MachineName,
    out: &mut impl Write,
) -> Result<(), BoxError> {
    let requester = lead_name(relay, machine)?;
    let registrations = relay.agents()?.into_registrations_reporting(|skipped| {
        eprintln!("agent-msg status-request: skipped {skipped}");
    });
    let expected_agents = if lanes.eq_ignore_ascii_case("all") {
        registrations
            .iter()
            .map(|registration| registration.name.clone())
            .filter(|agent| agent != &requester)
            .collect::<Vec<_>>()
    } else {
        let mut selected = Vec::new();
        for raw in lanes
            .split(',')
            .map(str::trim)
            .filter(|raw| !raw.is_empty())
        {
            let found = registrations.iter().find(|registration| {
                registration.name.as_str().eq_ignore_ascii_case(raw)
                    || registration
                        .full_address()
                        .is_some_and(|address| address.to_string().eq_ignore_ascii_case(raw))
            });
            let registration = found.ok_or_else(|| {
                format!("unknown lane {raw:?}; use a registered address or `all`")
            })?;
            if registration.name == requester {
                return Err("a status request cannot target its own lane".into());
            }
            if selected.contains(&registration.name) {
                return Err(format!("duplicate lane {raw:?}").into());
            }
            selected.push(registration.name.clone());
        }
        selected
    };

    let sender = sender_address(relay, &requester)?;
    let request = relay.create_status_request(requester.clone(), expected_agents.clone())?;
    let mut deliveries = Vec::with_capacity(expected_agents.len());
    for agent in &expected_agents {
        let message = Message {
            from: sender.clone(),
            to: Target::Agent(agent.clone()),
            text: format!(
                "Program review status request {}. Reply with `agent-msg status-reply {}` and the fixed form: tickets, pull requests, blocker ticket IDs, lane state, spare coder slots, and owner decisions.",
                request.id, request.id
            ),
            sent_at_ms: now_ms(),
            forwarded: false,
        };
        let outcome = match relay.send(&message) {
            Ok(_) => "queued".to_owned(),
            Err(relay_core::Error::UnknownAgent(_)) => {
                "not queued: lane is no longer registered".to_owned()
            }
            Err(error) => format!(
                "enqueue outcome unknown: {}",
                relay_core::text::sanitise_line(&error.to_string())
            ),
        };
        deliveries.push(Delivery {
            agent: agent.clone(),
            outcome,
        });
    }
    let report = RequestReport {
        request_id: request.id,
        created_at_ms: request.created_at_ms,
        requested: expected_agents,
        deliveries,
    };
    serde_json::to_writer(&mut *out, &report)?;
    writeln!(out)?;
    Ok(())
}

/// Stores one lane's first response for a status request.
pub(super) fn reply(
    args: StatusReplyArgs,
    relay: &mut relay_core::Relay,
    machine: &MachineName,
    out: &mut impl Write,
) -> Result<(), BoxError> {
    let StatusReplyArgs {
        request_id,
        tickets,
        pull_requests,
        blocker_ticket_ids,
        state,
        spare_coder_slots,
        owner_decisions,
    } = args;
    let id: StatusRequestId = request_id.parse()?;
    let agent = lead_name(relay, machine)?;
    let lane_state = match state.as_str() {
        "active" => LaneState::Active,
        "idle" => LaneState::Idle,
        "blocked" => LaneState::Blocked,
        _ => return Err("state must be active, idle, or blocked".into()),
    };
    let reply = StatusReply {
        request_id: id,
        agent,
        tickets: items(tickets)?,
        pull_requests: items(pull_requests)?,
        blocker_ticket_ids: items(blocker_ticket_ids)?,
        lane_state,
        spare_coder_slots: u16::try_from(spare_coder_slots)
            .map_err(|_| "spare coder slots must be at most 65535")?,
        owner_decisions: items(owner_decisions)?,
        submitted_at_ms: 0,
    };
    let outcome = relay.submit_status_reply(reply)?;
    writeln!(
        out,
        "{}",
        match outcome {
            relay_core::ReplyOutcome::Stored => "reply recorded",
            relay_core::ReplyOutcome::AlreadySubmitted =>
                "reply already recorded; the first reply was kept",
        }
    )?;
    Ok(())
}

/// Prints the request snapshot, replies, and explicit nonresponders as JSON.
pub(super) fn replies(
    request_id: String,
    relay: &mut relay_core::Relay,
    out: &mut impl Write,
) -> Result<(), BoxError> {
    let id: StatusRequestId = request_id.parse()?;
    let Some((request, replies)) = relay.status_snapshot(&id)? else {
        return Err(format!("status request {id} was not found or has expired").into());
    };
    let answered = replies
        .iter()
        .map(|reply| reply.agent.clone())
        .collect::<BTreeSet<_>>();
    let nonresponders = request
        .expected_agents
        .iter()
        .filter(|agent| !answered.contains(*agent))
        .cloned()
        .collect();
    serde_json::to_writer(
        &mut *out,
        &RepliesReport {
            request_id: request.id,
            created_at_ms: request.created_at_ms,
            requested: request.expected_agents,
            replies,
            nonresponders,
        },
    )?;
    writeln!(out)?;
    Ok(())
}

/// Stores or clears the current lead's focus.
pub(super) fn set_focus(
    focus: Option<String>,
    relay: &mut relay_core::Relay,
    machine: &MachineName,
    out: &mut impl Write,
) -> Result<(), BoxError> {
    let agent = lead_name(relay, machine)?;
    relay.set_focus(&agent, focus.as_deref())?;
    writeln!(
        out,
        "{}",
        if focus.is_some() {
            "focus updated"
        } else {
            "focus cleared"
        }
    )?;
    Ok(())
}

/// Displays every registered lane's focus and marks old entries without hiding them.
pub(super) fn focus_list(
    relay: &mut relay_core::Relay,
    stale_after: Since,
    out: &mut impl Write,
) -> Result<(), BoxError> {
    let registrations = relay.agents()?.into_registrations_reporting(|skipped| {
        eprintln!("agent-msg who: skipped {skipped}");
    });
    let focus = relay
        .focus_records()?
        .into_iter()
        .map(|record| (record.agent.clone(), record))
        .collect::<BTreeMap<_, _>>();
    let max_age_ms = u64::try_from(stale_after.0.as_millis()).unwrap_or(u64::MAX);
    let now = now_ms();
    for registration in registrations {
        let address = registration
            .full_address()
            .map_or_else(|| registration.name.to_string(), |place| place.to_string());
        match focus.get(&registration.name) {
            Some(record) => {
                let stale = relay_core::focus_is_stale(now, record.set_at_ms, max_age_ms);
                writeln!(
                    out,
                    "{}  {}  set {}{}",
                    address,
                    relay_core::text::sanitise_line(&record.focus),
                    crate::clock(record.set_at_ms),
                    if stale { "  STALE" } else { "" }
                )?;
            }
            None => writeln!(out, "{address}  focus unset")?,
        }
    }
    Ok(())
}

/// Stores operator-entered capacity for a build server.
pub(super) fn set_capacity(
    server: String,
    cores: u32,
    ram_gib: u32,
    max_agents: Option<u32>,
    owner_rules: Vec<String>,
    relay: &mut relay_core::Relay,
    out: &mut impl Write,
) -> Result<(), BoxError> {
    let machine: MachineName = server.parse()?;
    let ram_mib = u64::from(ram_gib)
        .checked_mul(1_024)
        .ok_or("RAM value overflow")?;
    let capacity = relay.set_server_capacity(&machine, cores, ram_mib, max_agents, owner_rules)?;
    writeln!(
        out,
        "capacity recorded for {} ({} cores, {} MiB RAM)",
        capacity.machine, capacity.cores, capacity.ram_mib
    )?;
    Ok(())
}

/// Clears operator-entered capacity for one server.
pub(super) fn clear_capacity(
    server: String,
    relay: &mut relay_core::Relay,
    out: &mut impl Write,
) -> Result<(), BoxError> {
    let machine: MachineName = server.parse()?;
    relay.clear_server_capacity(&machine)?;
    writeln!(out, "capacity cleared for {machine}")?;
    Ok(())
}

/// Groups registrations by server and joins explicit capacity records.
pub(super) fn server_list(
    relay: &mut relay_core::Relay,
    out: &mut impl Write,
) -> Result<(), BoxError> {
    let registrations = relay.agents()?.into_registrations_reporting(|skipped| {
        eprintln!("agent-msg who: skipped {skipped}");
    });
    let capacities = relay
        .server_capacities()?
        .into_iter()
        .map(|capacity| (capacity.machine.clone(), capacity))
        .collect::<BTreeMap<_, _>>();
    let mut lanes = BTreeMap::<MachineName, Vec<AgentName>>::new();
    for registration in registrations {
        lanes
            .entry(registration.machine)
            .or_default()
            .push(registration.name);
    }
    let servers = lanes
        .keys()
        .chain(capacities.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    for server in servers {
        let names = lanes.get(&server).map(Vec::as_slice).unwrap_or_default();
        let lane_list = if names.is_empty() {
            "none".to_owned()
        } else {
            names
                .iter()
                .map(AgentName::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        };
        match capacities.get(&server) {
            Some(capacity) => {
                let max = capacity
                    .max_agents
                    .map_or_else(|| "unset".to_owned(), |n| n.to_string());
                let rules = if capacity.owner_rules.is_empty() {
                    "none".to_owned()
                } else {
                    capacity
                        .owner_rules
                        .iter()
                        .map(|rule| relay_core::text::sanitise_line(rule))
                        .collect::<Vec<_>>()
                        .join("; ")
                };
                writeln!(
                    out,
                    "{server}: cores={}, RAM={} MiB, max-agents={max}, rules={rules}, lanes=[{lane_list}]",
                    capacity.cores, capacity.ram_mib
                )?;
            }
            None => writeln!(out, "{server}: capacity unset, lanes=[{lane_list}]")?,
        }
    }
    Ok(())
}

fn lead_name(relay: &mut relay_core::Relay, machine: &MachineName) -> Result<AgentName, BoxError> {
    Ok(crate::identity::derive(relay, machine)?
        .ok_or("no verified lead location; run this session in Herdr")?
        .name)
}

fn sender_address(relay: &mut relay_core::Relay, agent: &AgentName) -> Result<AgentName, BoxError> {
    relay
        .agent(agent)?
        .and_then(|record| record.full_address())
        .ok_or("sender has no verified tab address")?
        .to_string()
        .parse()
        .map_err(Into::into)
}

fn items(values: Vec<String>) -> Result<Vec<StatusItem>, BoxError> {
    values
        .into_iter()
        .map(StatusItem::new)
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

#[derive(Serialize)]
struct Delivery {
    agent: AgentName,
    outcome: String,
}

#[derive(Serialize)]
struct RequestReport {
    request_id: StatusRequestId,
    created_at_ms: u64,
    requested: Vec<AgentName>,
    deliveries: Vec<Delivery>,
}

#[derive(Serialize)]
struct RepliesReport {
    request_id: StatusRequestId,
    created_at_ms: u64,
    requested: Vec<AgentName>,
    replies: Vec<StatusReply>,
    nonresponders: Vec<AgentName>,
}
