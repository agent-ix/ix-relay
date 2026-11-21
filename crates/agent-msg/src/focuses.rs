// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! `agent-msg focuses`: durable, revisioned named focus definitions with one owner (FR-061).
//!
//! The family is spelled in the plural so that `agent-msg focus [text] [--clear]` (FR-057) and
//! `agent-msg work focus <summary>` keep their meanings. The commands write only relay records:
//! no inbox is consumed, no registration is changed and nothing is written to Linear.
//!
//! Reads (`list`, `show`) need no binding. Writes are made by the caller's verified live
//! binding (explicit `--lead` and `--session`, or the verified calling pane), as for
//! `agent-msg work`. `create` and `update` resolve their references through the shared ix-board
//! read service under one bounded read and write nothing until every reference resolved.

mod caller;
mod file;
mod output;
mod resolve;
#[cfg(test)]
mod tests;

use std::{
    io::{self, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    time::Duration,
};

use clap::{Args, Subcommand};
use relay_core::{
    AgentList, MachineName, Relay, SessionId, Target,
    named_focus::{
        Caller, ClaimRequest, Definition, FocusError, FocusResult, ListEntry, LivenessEvidence,
        OwnerBinding, SelectionCheck, SessionState, UnavailableReason, UpdateOutcome,
    },
};

use self::{
    caller::{CallerSource, HerdrCaller},
    output::ListRow,
    resolve::{DEFAULT_READ_TIMEOUT, LinearSnapshots, SnapshotSource},
};

/// `agent-msg focuses ...`.
#[derive(Args)]
pub(crate) struct FocusesArgs {
    /// Print one compact JSON value, or on failure one typed error object on standard error.
    #[arg(long, global = true)]
    json: bool,
    /// Writes: the caller's Herdr address; omitted means the calling pane.
    #[arg(long, global = true)]
    lead: Option<String>,
    /// Writes: the expected native session ID; refuse a different occupant.
    #[arg(long, global = true)]
    session: Option<SessionId>,
    #[command(subcommand)]
    command: FocusesCommand,
}

#[derive(Subcommand)]
enum FocusesCommand {
    /// Resolve a definition file and store revision 1; the caller becomes owner.
    Create {
        /// The definition file (JSON).
        #[arg(long)]
        file: PathBuf,
        /// Overall deadline of the snapshot read, 1 to 600 seconds (default 120).
        #[arg(long, value_name = "SECONDS", value_parser = clap::value_parser!(u64).range(1..=600))]
        read_timeout: Option<u64>,
    },
    /// Read definitions with their current revision and owner state.
    List {
        /// Only definitions of this team.
        #[arg(long)]
        team: Option<String>,
        /// Include archived definitions.
        #[arg(long)]
        archived: bool,
    },
    /// Read one definition at one kept revision (the current one by default).
    Show {
        /// The focus ID or its exact current name.
        focus: String,
        /// A kept revision; the eight newest are kept.
        #[arg(long)]
        revision: Option<u64>,
    },
    /// Write the next revision from a complete replacement of the content.
    Update {
        /// The focus ID or its exact current name.
        focus: String,
        /// The definition file (JSON).
        #[arg(long)]
        file: PathBuf,
        /// The revision this change was made against.
        #[arg(long)]
        expected_revision: u64,
        /// Overall deadline of the snapshot read, 1 to 600 seconds (default 120).
        #[arg(long, value_name = "SECONDS", value_parser = clap::value_parser!(u64).range(1..=600))]
        read_timeout: Option<u64>,
    },
    /// Archive a definition: final, releases its name, writes no revision.
    Archive {
        /// The focus ID or its exact current name.
        focus: String,
        /// The revision this change was made against.
        #[arg(long)]
        expected_revision: u64,
    },
    /// Take ownership of a definition whose owner's session is gone.
    Claim {
        /// The focus ID or its exact current name.
        focus: String,
        /// The owner epoch you observed.
        #[arg(long)]
        expected_owner_epoch: u64,
        /// Claim from an owner that is live or whose state is unknown.
        #[arg(long, requires = "reason")]
        force: bool,
        /// Why; recorded in the owner's last change.
        #[arg(long, requires = "force")]
        reason: Option<String>,
    },
    /// Hand ownership to a verified live session.
    Transfer {
        /// The focus ID or its exact current name.
        focus: String,
        /// The full server:workspace:tab address of the new owner.
        #[arg(long)]
        to: String,
        /// The owner epoch you verified.
        #[arg(long)]
        expected_owner_epoch: u64,
    },
}

/// Everything a `focuses` command reaches outside itself, so that a test can supply its own.
pub(crate) struct Env<'a> {
    pub(crate) relay: &'a mut Relay,
    pub(crate) check: &'a dyn SelectionCheck,
    pub(crate) snapshots: &'a mut dyn SnapshotSource,
    pub(crate) caller: &'a dyn CallerSource,
    pub(crate) machine: &'a MachineName,
    pub(crate) now_ms: i64,
}

/// The check of a stored selection by the pinned ix-board decoder.
pub(crate) struct IxBoardCheck;

impl SelectionCheck for IxBoardCheck {
    fn schema_version(&self) -> u32 {
        ix_board::NAMED_SELECTION_SCHEMA_VERSION
    }

    fn check(&self, selection: &[u8]) -> Result<(), UnavailableReason> {
        match ix_board::decode_named_selection(selection) {
            Ok(_) => Ok(()),
            Err(ix_board::Error::UnsupportedSchemaVersion { .. }) => {
                Err(UnavailableReason::UnsupportedSchema)
            }
            Err(_) => Err(UnavailableReason::Undecodable),
        }
    }
}

/// The entry point of the command: connects, runs, and reports a failure as a typed error.
pub(crate) fn main(args: &FocusesArgs, settings: &relay_core::config::CoreSettings) -> ExitCode {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    let mut relay = match Relay::connect(&settings.url.value, crate::CLI_TIMEOUT) {
        Ok(relay) => relay.with_prefix(settings.prefix.value.clone()),
        Err(error) => {
            output::failure(&mut stderr, args.json, &FocusError::Store(error));
            return ExitCode::FAILURE;
        }
    };
    let mut snapshots = LinearSnapshots;
    let caller = HerdrCaller {
        machine: &settings.machine.value,
    };
    let mut env = Env {
        relay: &mut relay,
        check: &IxBoardCheck,
        snapshots: &mut snapshots,
        caller: &caller,
        machine: &settings.machine.value,
        now_ms: crate::now_ms(),
    };
    match run(args, &mut env, &mut stdout) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            output::failure(&mut stderr, args.json, &error);
            ExitCode::FAILURE
        }
    }
}

fn io_error(error: &io::Error) -> FocusError {
    FocusError::invalid("output", error.to_string())
}

/// Runs one command, printing its result on `out`.
pub(crate) fn run<W: Write>(args: &FocusesArgs, env: &mut Env<'_>, out: &mut W) -> FocusResult<()> {
    let json = args.json;
    match &args.command {
        FocusesCommand::Create { file, read_timeout } => {
            create(args, env, file, timeout(*read_timeout), out)
        }
        FocusesCommand::List { team, archived } => list(env, json, team.as_deref(), *archived, out),
        FocusesCommand::Show { focus, revision } => show(env, json, focus, *revision, out),
        FocusesCommand::Update {
            focus,
            file,
            expected_revision,
            read_timeout,
        } => update(
            args,
            env,
            (focus.as_str(), file.as_path(), *expected_revision),
            timeout(*read_timeout),
            out,
        ),
        FocusesCommand::Archive {
            focus,
            expected_revision,
        } => archive(args, env, focus, *expected_revision, out),
        FocusesCommand::Claim {
            focus,
            expected_owner_epoch,
            force,
            reason,
        } => claim(
            args,
            env,
            focus,
            *expected_owner_epoch,
            reason.as_deref().filter(|_| *force),
            out,
        ),
        FocusesCommand::Transfer {
            focus,
            to,
            expected_owner_epoch,
        } => transfer(args, env, focus, to, *expected_owner_epoch, out),
    }
}

fn timeout(seconds: Option<u64>) -> Duration {
    seconds.map_or(DEFAULT_READ_TIMEOUT, Duration::from_secs)
}

fn verify(args: &FocusesArgs, env: &Env<'_>) -> FocusResult<OwnerBinding> {
    env.caller
        .verify(args.lead.as_deref(), args.session.as_ref(), env.now_ms)
}

/// Refuses early a caller that is not the stored owner, before any snapshot is read.
fn ensure_owner(current: &Definition, binding: &OwnerBinding) -> FocusResult<()> {
    let owner = &current.summary.owner;
    if owner.owner_id == binding.owner_id() {
        return Ok(());
    }
    Err(FocusError::NotOwner {
        detail: format!(
            "the verified caller {} is not the owner of {} (the owner is {})",
            binding.owner_id(),
            current.summary.meta.focus_id,
            owner.owner_id
        ),
        focus: current.summary.meta.focus_id.clone(),
        owner_id: owner.owner_id.clone(),
        epoch: owner.epoch,
    })
}

fn create<W: Write>(
    args: &FocusesArgs,
    env: &mut Env<'_>,
    file: &Path,
    read_timeout: Duration,
    out: &mut W,
) -> FocusResult<()> {
    let definition = file::read_definition(file)?;
    let binding = verify(args, env)?;
    let content = resolve::resolve(&definition, &mut *env.snapshots, read_timeout)?;
    let evidence = env.relay.focus_evidence();
    let created = env
        .relay
        .focus_create(env.check, content, &binding, env.now_ms, &evidence)?;
    output::created(
        out,
        args.json,
        &created.meta,
        &created.owner,
        created.removed.as_ref(),
    )
    .map_err(|error| io_error(&error))
}

fn update<W: Write>(
    args: &FocusesArgs,
    env: &mut Env<'_>,
    (focus, file, expected_revision): (&str, &Path, u64),
    read_timeout: Duration,
    out: &mut W,
) -> FocusResult<()> {
    let definition = file::read_definition(file)?;
    let binding = verify(args, env)?;
    let target = env.relay.focus_resolve(focus)?;
    let current = env.relay.focus_get(env.check, &target.id)?;
    ensure_owner(&current, &binding)?;
    let content = resolve::resolve(&definition, &mut *env.snapshots, read_timeout)?;
    let evidence = env.relay.focus_evidence();
    let caller = Caller {
        binding: &binding,
        expected_epoch: current.summary.owner.epoch,
        now_ms: env.now_ms,
    };
    let (revision, changed) = match env.relay.focus_update(
        env.check,
        &target,
        expected_revision,
        &content,
        &caller,
        &evidence,
    )? {
        UpdateOutcome::Written(revision) => (revision, true),
        UpdateOutcome::Unchanged(revision) => (revision, false),
    };
    output::updated(out, args.json, &target.id, revision.revision, changed)
        .map_err(|error| io_error(&error))
}

fn archive<W: Write>(
    args: &FocusesArgs,
    env: &mut Env<'_>,
    focus: &str,
    expected_revision: u64,
    out: &mut W,
) -> FocusResult<()> {
    let binding = verify(args, env)?;
    let target = env.relay.focus_resolve(focus)?;
    let current = env.relay.focus_get(env.check, &target.id)?;
    ensure_owner(&current, &binding)?;
    let caller = Caller {
        binding: &binding,
        expected_epoch: current.summary.owner.epoch,
        now_ms: env.now_ms,
    };
    let meta = env
        .relay
        .focus_archive(env.check, &target, expected_revision, &caller)?;
    output::archived(out, args.json, &meta).map_err(|error| io_error(&error))
}

fn claim<W: Write>(
    args: &FocusesArgs,
    env: &mut Env<'_>,
    focus: &str,
    expected_epoch: u64,
    force_reason: Option<&str>,
    out: &mut W,
) -> FocusResult<()> {
    let binding = verify(args, env)?;
    let target = env.relay.focus_resolve(focus)?;
    let observed = env.relay.focus_get(env.check, &target.id)?;
    let observed_owner_id = observed.summary.owner.owner_id.clone();
    let evidence = env.relay.focus_evidence();
    let caller = Caller {
        binding: &binding,
        expected_epoch,
        now_ms: env.now_ms,
    };
    let owner = env.relay.focus_claim(
        env.check,
        &target,
        &caller,
        &ClaimRequest {
            observed_owner_id: &observed_owner_id,
            force_reason,
        },
        &evidence,
    )?;
    output::owned(out, args.json, &target.id, &owner).map_err(|error| io_error(&error))
}

fn transfer<W: Write>(
    args: &FocusesArgs,
    env: &mut Env<'_>,
    focus: &str,
    to: &str,
    expected_epoch: u64,
    out: &mut W,
) -> FocusResult<()> {
    let binding = verify(args, env)?;
    let target = env.relay.focus_resolve(focus)?;
    let current = env.relay.focus_get(env.check, &target.id)?;
    ensure_owner(&current, &binding)?;
    let registry = env.relay.agents()?;
    let new_owner = transfer_target(&registry, to, env.machine)?;
    let caller = Caller {
        binding: &binding,
        expected_epoch,
        now_ms: env.now_ms,
    };
    let owner = env
        .relay
        .focus_transfer(env.check, &target, &caller, &new_owner)?;
    output::owned(out, args.json, &target.id, &owner).map_err(|error| io_error(&error))
}

/// The session `to` names, which must resolve to exactly one verified live registration.
fn transfer_target(
    registry: &AgentList,
    to: &str,
    machine: &MachineName,
) -> FocusResult<OwnerBinding> {
    let parts: Vec<&str> = to.split(':').collect();
    if parts.len() != 3 || parts.iter().any(|part| part.is_empty()) {
        return Err(FocusError::invalid(
            "to",
            "needs a full server:workspace:tab address",
        ));
    }
    let target = relay_core::resolve_target(to, machine, &registry.registrations)
        .map_err(|error| FocusError::invalid("to", error.to_string()))?;
    let Target::Agent(name) = target else {
        return Err(FocusError::invalid("to", "must name one session, not all"));
    };
    let registration = registry
        .registrations
        .iter()
        .find(|registration| registration.name == name)
        .ok_or_else(|| FocusError::invalid("to", "the target is no longer registered"))?;
    let (Some(session), Some(address)) = (
        registration.session_id.as_ref(),
        registration.full_address(),
    ) else {
        return Err(FocusError::invalid(
            "to",
            "the target has no verified session placement",
        ));
    };
    let address = address.to_string();
    let evidence = LivenessEvidence::from_registry(registry);
    if evidence.state_of(&registration.machine, session, Some(&address)) != SessionState::Live {
        return Err(FocusError::invalid(
            "to",
            "the target is not exactly one verified live session",
        ));
    }
    let verified_at_ms = registration
        .place
        .as_ref()
        .and_then(|place| place.herdr_ids.as_ref())
        .map_or(0, |ids| ids.verified_at_ms);
    Ok(OwnerBinding::new(
        &registration.machine,
        session,
        None,
        &address,
        verified_at_ms,
    ))
}

fn list<W: Write>(
    env: &mut Env<'_>,
    json: bool,
    team: Option<&str>,
    archived: bool,
    out: &mut W,
) -> FocusResult<()> {
    let entries = env.relay.focus_list(env.check)?;
    let evidence = env.relay.focus_evidence();
    let rows: Vec<ListRow<'_>> = entries
        .iter()
        .filter_map(|entry| match entry {
            ListEntry::Available(summary) => {
                let wanted = team.is_none_or(|team| summary.meta.team.eq_ignore_ascii_case(team))
                    && (archived
                        || summary.meta.state
                            != relay_core::named_focus::DefinitionState::Archived);
                wanted.then(|| {
                    ListRow::Available(
                        summary,
                        evidence
                            .owner_state(&summary.owner.owner_id, &summary.owner.binding.address),
                    )
                })
            }
            ListEntry::Unavailable { focus, reason } => {
                Some(ListRow::Unavailable(focus, reason.as_str()))
            }
        })
        .collect();
    output::list(out, json, &rows).map_err(|error| io_error(&error))
}

fn show<W: Write>(
    env: &mut Env<'_>,
    json: bool,
    focus: &str,
    revision: Option<u64>,
    out: &mut W,
) -> FocusResult<()> {
    let target = env.relay.focus_resolve(focus)?;
    let definition = env.relay.focus_get(env.check, &target.id)?;
    let revision = match revision {
        Some(number) if number != definition.summary.meta.latest_revision => {
            env.relay.focus_revision(env.check, &target.id, number)?
        }
        _ => definition.summary.latest.clone(),
    };
    let evidence = env.relay.focus_evidence();
    let owner = &definition.summary.owner;
    let state = evidence.owner_state(&owner.owner_id, &owner.binding.address);
    output::show(
        out,
        json,
        &definition.summary,
        &definition.kept_revisions,
        &revision,
        state,
    )
    .map_err(|error| io_error(&error))
}
