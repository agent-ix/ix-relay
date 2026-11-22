// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! Explicit session work reporting CLI; no message delivery or tracker writes.

use crate::BoxError;
use clap::{Args, Subcommand};
use ix_board::{
    Clock, LinearGraphQlSource, ReadPolicy, ReadService, ScopeSelection, WorkspaceSnapshot,
    WorkspaceSource,
};
use relay_core::{
    Relay, SessionId, TicketId,
    work::{AgentWork, WorkFocus, WorkScope, WorkStatus},
};
use relay_ops::work::{resolve_work_scope, resolve_work_target, resolve_work_target_for_session};
use std::{io::Write, time::Duration};

#[derive(Args)]
pub(crate) struct WorkArgs {
    /// Target lead's Herdr address; omitted means the calling pane.
    #[arg(long)]
    lead: Option<String>,
    /// Select this native session within a shared tab and refuse a replacement occupant.
    #[arg(long)]
    session: Option<SessionId>,
    #[command(subcommand)]
    command: WorkCommand,
}

#[derive(Subcommand)]
enum WorkCommand {
    /// Set whole-team scope or resolve narrower selectors against Linear.
    Scope {
        /// Clear narrower selectors and monitor the lead's whole team.
        #[arg(long, conflicts_with_all = ["initiatives", "projects", "tickets"], required_unless_present_any = ["initiatives", "projects", "tickets"])]
        all_team: bool,
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

/// Oldest source snapshot scope resolution and ticket verification accept, measured from when
/// its source read began.
const SNAPSHOT_MAX_AGE: Duration = Duration::from_secs(300);

/// Bound on the whole read, from the moment the credentials are ready.
const READ_DEADLINE: Duration = Duration::from_secs(120);

fn source_snapshot() -> Result<WorkspaceSnapshot, BoxError> {
    // The credentials are resolved before the deadline starts: obtaining them can wait for
    // the user's keychain.
    let source = LinearGraphQlSource::from_env()?;
    let mut service = ReadService::open_default(source)?;
    Ok(fresh_snapshot(&mut service)?)
}

/// A snapshot read within [`READ_DEADLINE`] that is at most [`SNAPSHOT_MAX_AGE`] old. When
/// that cannot be established, because the refresh failed or the deadline passed, the error is
/// returned and no older snapshot stands in: a scope or ticket is never written against
/// evidence of unknown age.
fn fresh_snapshot<S: WorkspaceSource, C: Clock>(
    service: &mut ReadService<S, C>,
) -> Result<WorkspaceSnapshot, ix_board::Error> {
    service.workspace_snapshot_bounded(&ReadPolicy {
        max_age: SNAPSHOT_MAX_AGE,
        deadline: READ_DEADLINE,
    })
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
    let observation = resolve_work_target_for_session(
        &runner,
        args.lead.as_deref(),
        args.session.as_ref(),
        &settings.machine.value,
    )?;
    let binding = observation.binding;
    let now = crate::now_ms();
    match args.command {
        WorkCommand::Scope {
            all_team,
            initiatives,
            projects,
            tickets,
        } => {
            let scope = if all_team {
                WorkScope {
                    reported_at_ms: now,
                    ..Default::default()
                }
            } else {
                resolve_work_scope(
                    &source_snapshot()?,
                    &ScopeSelection {
                        initiatives,
                        projects,
                        tickets,
                        ..Default::default()
                    },
                    now,
                )?
            };
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
            crate::json::write_line(out, &relay.work_report(&binding)?)?;
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

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering},
    };

    use clap::Parser;
    use ix_board::{Clock, Deadline, Error, ReadService, WorkspaceSnapshot, WorkspaceSource};
    use ix_trace_rs::trace;

    use super::{READ_DEADLINE, SNAPSHOT_MAX_AGE, fresh_snapshot};

    const START: i64 = 1_800_000_000;

    /// A wall clock that moves only when a test says so.
    #[derive(Clone)]
    struct EpochClock(Arc<AtomicI64>);

    impl Clock for EpochClock {
        fn now_epoch_seconds(&self) -> i64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    /// A workspace source that honors the bounded read, counts its reads and can be down.
    struct BoundedSource {
        down: Arc<AtomicBool>,
        reads: Arc<AtomicUsize>,
    }

    impl WorkspaceSource for BoundedSource {
        fn fetch_workspace(&self) -> Result<WorkspaceSnapshot, Error> {
            Err(Error::BoundUnsupported)
        }

        fn supports_bounded_read(&self) -> bool {
            true
        }

        fn fetch_workspace_bounded(
            &self,
            _deadline: &Deadline<'_>,
        ) -> Result<WorkspaceSnapshot, Error> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            if self.down.load(Ordering::SeqCst) {
                Err(Error::GraphQl {
                    messages: "down".into(),
                })
            } else {
                Ok(WorkspaceSnapshot::default())
            }
        }
    }

    /// A real read service over a private cache, already holding one snapshot read at START.
    struct Seeded {
        _directory: tempfile::TempDir,
        service: ReadService<BoundedSource, EpochClock>,
        clock: EpochClock,
        down: Arc<AtomicBool>,
        reads: Arc<AtomicUsize>,
    }

    impl Seeded {
        fn new() -> Self {
            let directory = tempfile::tempdir().unwrap();
            let clock = EpochClock(Arc::new(AtomicI64::new(START)));
            let down = Arc::new(AtomicBool::new(false));
            let reads = Arc::new(AtomicUsize::new(0));
            let source = BoundedSource {
                down: Arc::clone(&down),
                reads: Arc::clone(&reads),
            };
            let mut service = ReadService::with_clock(
                source,
                &directory.path().join("cache.sqlite3"),
                clock.clone(),
            )
            .unwrap();
            fresh_snapshot(&mut service).expect("the seeding read succeeds");
            assert_eq!(reads.load(Ordering::SeqCst), 1);
            Self {
                _directory: directory,
                service,
                clock,
                down,
                reads,
            }
        }

        fn pass(&self, seconds: i64) {
            self.clock.0.fetch_add(seconds, Ordering::SeqCst);
        }

        fn reads(&self) -> usize {
            self.reads.load(Ordering::SeqCst)
        }
    }

    /// The contract is five minutes of age and a 120-second read, whatever the code derives.
    /// A snapshot read four minutes ago is inside the bound and is used without another
    /// source read, even while the source is down.
    #[trace("FR-048-AC-3")]
    #[test]
    fn work_accepts_a_snapshot_read_within_five_minutes_without_a_refresh() {
        assert_eq!(SNAPSHOT_MAX_AGE, std::time::Duration::from_secs(300));
        assert_eq!(READ_DEADLINE, std::time::Duration::from_secs(120));
        let mut seeded = Seeded::new();
        seeded.pass(240);
        seeded.down.store(true, Ordering::SeqCst);
        fresh_snapshot(&mut seeded.service).expect("a fresh cache entry is accepted");
        assert_eq!(seeded.reads(), 1, "no refresh was needed");
    }

    /// Past five minutes the snapshot must be refreshed. When that refresh fails the error is
    /// returned and the retained snapshot is not used, so `work scope`, `work focus` and
    /// `work agent` stop before they resolve a scope or verify a ticket and write no report.
    #[trace("FR-048-AC-3")]
    #[test]
    fn work_fails_closed_when_a_snapshot_older_than_five_minutes_cannot_be_refreshed() {
        let mut seeded = Seeded::new();
        seeded.pass(301);
        seeded.down.store(true, Ordering::SeqCst);
        let refused = fresh_snapshot(&mut seeded.service).expect_err("no stale stand-in");
        assert!(matches!(refused, Error::GraphQl { .. }), "{refused}");
        assert_eq!(seeded.reads(), 2, "a refresh was attempted once");

        seeded.down.store(false, Ordering::SeqCst);
        fresh_snapshot(&mut seeded.service).expect("a healthy source refreshes it");
        assert_eq!(seeded.reads(), 3);
    }

    /// Trace: FR-048-AC-9
    #[test]
    fn whole_team_scope_requires_an_explicit_exclusive_choice() {
        assert!(crate::Cli::try_parse_from(["agent-msg", "work", "scope"]).is_err());
        assert!(
            crate::Cli::try_parse_from([
                "agent-msg",
                "work",
                "scope",
                "--all-team",
                "--project",
                "p"
            ])
            .is_err()
        );
        let parsed =
            crate::Cli::try_parse_from(["agent-msg", "work", "scope", "--all-team"]).unwrap();
        assert!(matches!(
            parsed.cmd,
            crate::Cmd::Work(super::WorkArgs {
                command: super::WorkCommand::Scope { all_team: true, .. },
                ..
            })
        ));
    }
}
