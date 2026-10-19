//! `agent-msg`: how team leads talk to each other over the ix-relay bus.
//!
//! One binary serves agents typing commands and the hooks their harness runs.
//! It connects to Redis at a URL (relay-redis on hub over WireGuard, set per
//! machine; README.md "Transport"), signs as `AGENT_MSG_NAME`, and keys are prefixed with a Redis key
//! prefix. The URL, prefix, machine name and human inbox name come from
//! `~/.ix/relay/config.toml` via [`relay_core::config`] (AGE-2017) — flag
//! beats env beats the config file beats the compiled-in default.
//! `AGENT_MSG_NAME` stays env-only: one machine can run several leads, so it
//! can't live in the machine-level config file.

mod budget;
mod doctor;
mod find;
mod hook;
mod hook_run;
mod identity;
mod inbox_report;
mod render;
mod report;
mod retry;
mod send_state;
mod since;
mod spool;
mod stdin;
mod work;

use std::{
    io::{self, Write},
    num::NonZeroUsize,
    process::ExitCode,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use clap::{Parser, Subcommand, ValueEnum};
use relay_core::{
    AddressBook, AgentName, EscalationStatus, MachineName, Message, PaneId, Registration, Relay,
};
use relay_ops::{
    clock,
    escalate::{self, EscalateArgs},
    herdr::RealHerdr,
    linear::RealLinear,
};

use crate::{
    budget::SystemClock,
    hook_run::{HookCtx, RelayBus},
    inbox_report::InboxReport,
    since::Since,
    spool::{ErrorKind, Spool, Stage},
};

/// Connect/read/write timeout for commands an agent runs.
const CLI_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Parser)]
#[command(
    name = "agent-msg",
    version,
    about = "Send and read messages between team leads."
)]
struct Cli {
    /// Redis URL of the bus. Defaults come from `~/.ix/relay/config.toml`,
    /// then `IX_RELAY_URL`, then `redis://127.0.0.1:6379`.
    #[arg(long, global = true)]
    url: Option<String>,
    /// Redis key prefix. Lets tests (and, in principle, separate buses) share one
    /// Redis without colliding. Defaults come from `~/.ix/relay/config.toml`,
    /// then `IX_RELAY_PREFIX`, then [`relay_core::DEFAULT_PREFIX`].
    #[arg(long, global = true)]
    prefix: Option<String>,
    /// The human inbox `escalate`/`resolve` notify/sign as. Defaults come
    /// from `~/.ix/relay/config.toml`'s `human` key, then `IX_RELAY_HUMAN`,
    /// then `operator`.
    #[arg(long, global = true)]
    human: Option<String>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Report session scope, focus and agent assignments for the dashboard.
    Work(work::WorkArgs),
    /// Send a message to one lead, or to `all`.
    Send {
        /// Full server:workspace:tab, :workspace:tab, :tab, or all.
        /// The relay discovers and enrolls the live Herdr lead automatically.
        to: String,
        /// The message.
        #[arg(required = true, num_args = 1..)]
        text: Vec<String>,
    },
    /// Print and remove your pending messages.
    Inbox {
        /// Most messages to take.
        #[arg(long, default_value = "50")]
        max: NonZeroUsize,
        /// Print `{"messages": [...], "rejected": [...]}` instead of plain
        /// text, for callers that parse the output (e.g. the opencode
        /// plugin). Still pops: use `--count` for a non-destructive peek.
        #[arg(long, conflicts_with = "count")]
        json: bool,
        /// Print the number of pending messages without removing anything,
        /// so a caller can decide whether it's worth starting a turn to read
        /// them (the opencode plugin's idle path does this).
        #[arg(long)]
        count: bool,
        /// Print your dead letters (messages whose redeliveries ran out, FR-040)
        /// instead of popping, oldest first, and leave them where they are.
        #[arg(long, conflicts_with_all = ["count", "json"])]
        dead: bool,
        /// Print, for every lead that has some, how many messages are in flight
        /// (popped, not yet acknowledged) and how many are dead letters, found by
        /// scanning the keys: it lists leads that were pruned or renamed too, whose
        /// stranded batches nothing else shows. Looks at no more than 1,000 keys of
        /// each kind and says when it stopped there. Changes nothing.
        #[arg(long, conflicts_with_all = ["count", "json", "dead"])]
        in_flight: bool,
    },
    /// List registered leads.
    Who,
    /// Look up a live lead by address, workspace/tab label, session name,
    /// pane id or `server:workspace[:tab[:pane]]` address, joining the bus registry
    /// against live herdr state on this machine and every saved one
    /// (AGE-2018). Ends with the address(es) to use with `send`. Not needed
    /// before `send`: `send` resolves an address itself.
    Find {
        /// Case-insensitive substring to match; omit to list everyone.
        query: Option<String>,
        /// Print one JSON object per matching row instead of plain text.
        #[arg(long)]
        json: bool,
    },
    /// Check the relay's health: Redis, this machine's watcher, hook failures,
    /// herdr, registrations, queue depth and the watcher's unit. Read-only; works
    /// with Redis down. Exits 1 if anything is wrong.
    Doctor {
        /// Print one JSON object `{"ok": ..., "checks": [...]}` instead of text.
        /// Each check has a stable `code`.
        #[arg(long)]
        json: bool,
        /// Exit 1 on a warning too (the default fails only on a failure).
        #[arg(long)]
        strict: bool,
    },
    /// Show recent message history.
    Log {
        /// How far back, e.g. 30m, 1h, 2d.
        #[arg(long, default_value = "1h")]
        since: Since,
        /// Most messages to show.
        #[arg(long, default_value_t = 100)]
        max: usize,
    },
    /// Register yourself by hand. Hooks do this on session start.
    Register {
        /// Harness kind.
        #[arg(long, default_value = "manual")]
        kind: String,
    },
    /// Print this session's verified server:workspace:tab identity.
    /// An unverified session prints not a lead; aliases never supply identity.
    Whoami {
        /// Print the canonical address and location source, or {"lead":false}.
        #[arg(long)]
        json: bool,
    },
    /// Hook handler: reads hook JSON on stdin, prints hook output. Never fails the hook.
    Hook {
        /// Which harness is calling.
        kind: HookKind,
    },
    /// File an escalation: a decision only the operator can make. Prints its id.
    Escalate {
        /// The Linear ticket this decision belongs to, e.g. `AGE-2014`.
        ticket: String,
        /// What needs deciding.
        #[arg(long)]
        context: String,
        /// A choice the operator could make. Repeatable.
        #[arg(long = "option")]
        option: Vec<String>,
        /// Your own recommendation.
        #[arg(long)]
        recommend: String,
    },
    /// List escalations.
    Escalations {
        /// Only escalations in this state. Defaults to every state.
        #[arg(long)]
        status: Option<StatusArg>,
    },
    /// Open a Claude Code session on an escalation's lead's machine, in its
    /// cwd, to look into it.
    Investigate {
        /// The escalation id, e.g. `e7`.
        id: relay_core::EscalationId,
        /// Start a fresh session with a brief prompt instead of forking the
        /// lead's own. The full context, options and recommendation are read
        /// with `agent-msg escalation show <id>` from inside that session —
        /// never typed into it directly.
        #[arg(long)]
        fresh: bool,
    },
    /// Record a decision, tell the lead, and post it to the ticket.
    Resolve {
        /// The escalation id, e.g. `e7`.
        id: relay_core::EscalationId,
        /// The decision.
        decision: String,
    },
    /// Read or act on one escalation's full detail.
    Escalation {
        #[command(subcommand)]
        cmd: EscalationCmd,
    },
}

/// `agent-msg escalation <cmd>`.
#[derive(Subcommand)]
enum EscalationCmd {
    /// Print one escalation's full detail: ticket, lead, machine, cwd, kind,
    /// status, context, options and recommendation. This is what
    /// `investigate --fresh`'s prompt points a fresh session at, rather than
    /// embedding that text directly (see `escalate.rs`'s module doc comment).
    Show {
        /// The escalation id, e.g. `e7`.
        id: relay_core::EscalationId,
    },
    /// Retry posting a resolved escalation's decision to its Linear ticket —
    /// for when `resolve`'s own attempt failed.
    PostLinear {
        /// The escalation id, e.g. `e7`.
        id: relay_core::EscalationId,
    },
}

/// `--status` for `escalations`: [`EscalationStatus`] plus `all` (the default).
#[derive(Clone, Copy, ValueEnum)]
enum StatusArg {
    /// Filed; nobody has started looking at it yet.
    Waiting,
    /// Someone has a session open on it.
    Investigating,
    /// A decision was recorded.
    Resolved,
}

impl From<StatusArg> for EscalationStatus {
    fn from(s: StatusArg) -> Self {
        match s {
            StatusArg::Waiting => Self::Waiting,
            StatusArg::Investigating => Self::Investigating,
            StatusArg::Resolved => Self::Resolved,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum HookKind {
    Claude,
    Codex,
}

fn main() -> ExitCode {
    // A hook must never fail an agent's turn with a nonzero exit, not even on a
    // bad `--as`/`AGENT_MSG_NAME` or an argv clap can't parse at all: Claude Code
    // treats exit 2 as a blocking error. `argv[1] == "hook"` is checked before
    // `Cli::parse` (which errors, and by default exits 2) runs.
    let is_hook = std::env::args().nth(1).as_deref() == Some("hook");
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            if is_hook {
                hook_run::record_early(
                    Spool::in_config_dir().as_ref(),
                    &SystemClock::start(),
                    Stage::Args,
                    ErrorKind::Args,
                );
                report::line(format_args!("agent-msg hook: {e}"));
                return ExitCode::SUCCESS;
            }
            e.exit();
        }
    };

    if let Cmd::Hook { kind: _ } = cli.cmd {
        // Hooks outside Herdr are not lead hooks. Identity is established from
        // the genuine session association after the machine config is loaded.
        if std::env::var("HERDR_ENV").as_deref() != Ok("1") {
            return ExitCode::SUCCESS;
        }
        let clock = SystemClock::start();
        let spool = Spool::in_config_dir();
        // ~/.ix/relay/config.toml (AGE-2017). A hook must never break the
        // agent: a malformed file is reported on stderr and in the spool, and
        // exit stays 0, same as any other failure in `hook_run::run`.
        let settings = match relay_core::config::load_core(core_flags(&cli)) {
            Ok(settings) => settings,
            Err(e) => {
                hook_run::record_early(spool.as_ref(), &clock, Stage::Config, ErrorKind::Config);
                report::line(format_args!("agent-msg hook: {e}"));
                return ExitCode::SUCCESS;
            }
        };
        let cx = HookCtx {
            machine: &settings.machine.value,
        };
        // A panic anywhere in the hook is caught inside `run` and recorded;
        // this second net keeps even a panic in that recording from becoming a
        // nonzero exit, which Claude Code treats as a blocking error.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            hook_run::run(
                &cx,
                &clock,
                spool.as_ref(),
                (
                    &mut stdin::ThreadedStdin::new(io::stdin()),
                    &mut io::stdout().lock(),
                    &mut io::stderr().lock(),
                ),
                &mut |timeout| {
                    RelayBus::connect(
                        &settings.url.value,
                        &settings.prefix.value,
                        &settings.machine.value,
                        timeout,
                    )
                },
            );
        }));
        return ExitCode::SUCCESS;
    }

    // `doctor` reports a bad or unreachable setup, so it loads its own settings
    // and connects for itself.
    if let Cmd::Doctor { json, strict } = cli.cmd {
        return doctor::run(core_flags(&cli), json, strict);
    }

    // ~/.ix/relay/config.toml (AGE-2017), resolved flag > env > file >
    // default. A malformed file is an error every non-hook command exits 1
    // over.
    let settings = match relay_core::config::load_core(core_flags(&cli)) {
        Ok(settings) => settings,
        Err(e) => {
            eprintln!("agent-msg: {e}");
            return ExitCode::FAILURE;
        }
    };
    match run(cli, &settings) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("agent-msg: {e}");
            ExitCode::FAILURE
        }
    }
}

/// `agent-msg` reads no `--machine` flag (see [`relay_core::config::CoreFlags`]).
fn core_flags(cli: &Cli) -> relay_core::config::CoreFlags {
    relay_core::config::CoreFlags {
        url: cli.url.clone(),
        prefix: cli.prefix.clone(),
        machine: None,
        human: cli.human.clone(),
    }
}

pub(crate) type BoxError = Box<dyn std::error::Error>;

fn run(cli: Cli, settings: &relay_core::config::CoreSettings) -> Result<(), BoxError> {
    let redial = retry::Redial::new(&settings.url.value, &settings.prefix.value);
    let mut relay = redial.dial(CLI_TIMEOUT)?;
    let mut out = io::stdout().lock();
    match cli.cmd {
        Cmd::Work(args) => work::run(args, &mut relay, settings, &mut out)?,
        Cmd::Send { to, text } => {
            let mut resolved = resolve_send_target(&mut relay, &settings.machine.value, &to)?;
            let broadcast = matches!(resolved.target, relay_core::Target::All);
            let unreadable = resolved.unreadable;
            let mut msg = Message {
                from: {
                    let id = me(None, &mut relay, &settings.machine.value)?;
                    let address = relay
                        .agent(&id)?
                        .and_then(|r| r.full_address())
                        .ok_or("sender has no verified tab address")?;
                    address.to_string().parse()?
                },
                to: resolved.target.clone(),
                text: text.join(" "),
                sent_at_ms: now_ms(),
                forwarded: false,
            };
            let recipients = enqueue_resolved(&mut relay, &mut resolved, &mut msg)?;
            let recipient = recipient_registration(
                resolved.recipient,
                resolved.registry.as_deref(),
                &recipients,
            );
            // Reuse verified recipient metadata for public confirmation.
            let book = match resolved.registry {
                Some(registrations) => AddressBook::from_registrations(&registrations),
                None if broadcast => address_book(&mut relay),
                None => AddressBook::default(),
            };
            match recipients.as_slice() {
                [] => writeln!(out, "no other leads registered; logged only")?,
                [one] if unreadable => writeln!(
                    out,
                    "queued for {} (its registration is unreadable: nothing delivers until it re-registers)",
                    book.describe(one)
                )?,
                [one] => {
                    write!(out, "queued for {}", book.describe(one))?;
                    // One more pipelined request, bounded: whatever it cannot
                    // read in time leaves the plain line (FR-041).
                    if let Some(seen) =
                        recipient.and_then(|reg| send_state::observe(&mut relay, one, &reg))
                    {
                        write!(out, " ({})", send_state::describe(&seen, now_ms()))?;
                    }
                    writeln!(out)?;
                }
                many => {
                    let names = many.iter().map(|n| book.label(n)).collect::<Vec<_>>();
                    writeln!(out, "queued for {}: {}", many.len(), names.join(", "))?;
                }
            }
        }
        Cmd::Inbox {
            in_flight: true, ..
        } => {
            let backlog = relay.backlog()?;
            if backlog.leads.is_empty() {
                writeln!(out, "nothing in flight and no dead letters")?;
            }
            for lead in &backlog.leads {
                let note = if lead.unreadable {
                    " (a key of this lead holds the wrong type; counts are not reliable)"
                } else {
                    ""
                };
                writeln!(
                    out,
                    "{} in-flight={} dead-letters={}{note}",
                    relay_core::text::sanitise_line(&lead.lead),
                    lead.in_flight,
                    lead.dead
                )?;
            }
            if backlog.truncated {
                writeln!(
                    out,
                    "(stopped after {} keys; more leads may have entries)",
                    relay_core::BACKLOG_SCAN_BOUND
                )?;
            }
        }
        Cmd::Inbox {
            dead: true, max, ..
        } => {
            let name = me(None, &mut relay, &settings.machine.value)?;
            let found = relay.dead_letters(&name, max)?;
            if found.messages.is_empty() && found.rejected.is_empty() {
                writeln!(out, "no dead letters")?;
            }
            let book = display_book(&mut relay, &redial);
            for m in &found.messages {
                writeln!(out, "{} {}", clock(m.sent_at_ms), book.render(m))?;
            }
            for e in &found.rejected {
                writeln!(
                    out,
                    "undecodable dead letter: {}",
                    relay_core::text::sanitise_line(&e.to_string())
                )?;
            }
        }
        Cmd::Inbox { count: true, .. } => {
            let name = me(None, &mut relay, &settings.machine.value)?;
            let n = relay.inbox_len(&name)?;
            writeln!(out, "{n}")?;
        }
        Cmd::Inbox {
            max,
            json,
            count: false,
            dead: false,
            in_flight: false,
        } => {
            let name = me(None, &mut relay, &settings.machine.value)?;
            let popped = relay.pop_inbox(&name, max)?;
            if json {
                let report = InboxReport::new(&popped, &display_book(&mut relay, &redial));
                serde_json::to_writer(&mut out, &report)?;
                writeln!(out)?;
            } else {
                if popped.messages.is_empty() {
                    writeln!(out, "inbox empty")?;
                }
                let book = display_book(&mut relay, &redial);
                for m in &popped.messages {
                    writeln!(out, "{} {}", clock(m.sent_at_ms), book.render(m))?;
                }
            }
            for e in &popped.rejected {
                eprintln!("agent-msg: dropped one inbox entry: {e}");
            }
        }
        Cmd::Who => {
            let (list, replaced, holding) = retry::read(&mut relay, &redial, |r| {
                let list = r.agents()?;
                let names: Vec<&AgentName> = list.registrations.iter().map(|x| &x.name).collect();
                let replaced = r.panes_replaced(&names);
                // Mail left for a replaced session is forwarded or dead-lettered by the
                // watcher (FR-053): only a lead that still holds some is undeliverable. A
                // failed read of an inbox leaves the lead marked.
                let lens = r.inbox_lens(&names)?;
                let holding: std::collections::BTreeSet<AgentName> = names
                    .iter()
                    .zip(lens)
                    .filter(|(_, len)| !matches!(len, Ok(0)))
                    .map(|(name, _)| (*name).clone())
                    .collect();
                Ok((list, replaced, holding))
            })?;
            // Whether a pane now hosts another session is extra: a failed read of
            // it is said, and the listing is still printed.
            let replaced = replaced.map(|p| p.found).unwrap_or_else(|e| {
                eprintln!("agent-msg who: could not read which panes were replaced: {e}");
                std::collections::BTreeMap::new()
            });
            let registrations =
                list.into_registrations_reporting(|skipped| eprintln!("agent-msg who: {skipped}"));
            // The address column follows the one shown-address rule (a lead's
            // tab is dropped only when it is alone in its workspace), so it needs
            // the whole registry, which is in hand.
            let book = AddressBook::from_registrations(&registrations);
            for r in registrations {
                let address = book
                    .place_address(&r.name)
                    .map_or_else(|| "-".to_owned(), ToString::to_string);
                // Registration fields come from whoever registered: one line, no
                // escape codes, before they reach this terminal.
                let line = relay_core::text::sanitise_line;
                let cwd = line(r.cwd.as_deref().unwrap_or("-"));
                let pane = r.herdr_pane.as_ref().map_or("-", PaneId::as_str);
                let (machine, kind) = (r.machine.as_str(), line(&r.kind));
                let seen = clock(r.last_seen_ms);
                let place_cache = match r
                    .verified_place()
                    .and_then(|place| place.herdr_ids.as_ref())
                {
                    Some(ids) if ids.is_stale(now_ms()) => {
                        format!("cache stale: {}:{}", ids.workspace, ids.tab)
                    }
                    Some(ids) => format!("Herdr {}:{}", ids.workspace, ids.tab),
                    None => "location unverified".to_owned(),
                };
                write!(
                    out,
                    "{address}  {machine}  {kind}  seen {seen}  pane {pane}  {cwd}  {place_cache}"
                )?;
                // FR-032: the watcher found this lead's pane hosting another
                // session, so what is listed for it is stale, not live.
                if let Some(found) = replaced
                    .get(&r.name)
                    .filter(|f| f.describes(&r) && holding.contains(&r.name))
                {
                    write!(
                        out,
                        "  UNDELIVERABLE: pane {pane} now hosts another session (since {})",
                        clock(found.since_ms)
                    )?;
                }
                writeln!(out)?;
            }
        }
        Cmd::Find { query, json } => {
            let query = query.unwrap_or_default();
            let report = find::run(&mut relay, &redial, &settings.machine.value, &query)?;
            for skipped in &report.skipped {
                eprintln!(
                    "agent-msg find: skipped {}: {}",
                    skipped.machine, skipped.reason
                );
            }
            if json {
                for row in &report.rows {
                    serde_json::to_writer(&mut out, &find::FoundJson::from(row))?;
                    writeln!(out)?;
                }
            } else {
                if report.rows.is_empty() {
                    writeln!(out, "no matching agents")?;
                }
                for row in &report.rows {
                    writeln!(out, "{}", find::render_line(row))?;
                }
                let targets = find::sendable_targets(&report.rows);
                if !targets.is_empty() {
                    writeln!(out, "send to: {}", targets.join(", "))?;
                }
            }
        }
        Cmd::Log { since, max } => {
            let window = i64::try_from(since.0.as_millis()).unwrap_or(i64::MAX);
            let book = display_book(&mut relay, &redial);
            let since_ms = now_ms().saturating_sub(window);
            let messages = retry::read(&mut relay, &redial, |r| Ok(r.log(since_ms, max)?))?;
            for m in messages {
                writeln!(
                    out,
                    "{} {} -> {}: {}",
                    clock(m.sent_at_ms),
                    book.label(&m.from),
                    book.label_target(&m.to),
                    relay_core::text::neutralise(&m.text)
                )?;
            }
        }
        Cmd::Register { kind } => {
            let _ = kind;
            let name = me(None, &mut relay, &settings.machine.value)?;
            let address = display_book(&mut relay, &redial).label(&name);
            writeln!(out, "registered {address}")?;
        }
        Cmd::Escalate {
            ticket,
            context,
            option,
            recommend,
        } => {
            let name = me(None, &mut relay, &settings.machine.value)?;
            let outcome = escalate::run_escalate(
                &mut relay,
                &name,
                EscalateArgs {
                    ticket,
                    context,
                    options: option,
                    recommendation: recommend,
                    human: settings.human.value.clone(),
                },
                now_ms(),
            )?;
            writeln!(out, "{}", render::escalate_id_line(&outcome))?;
            if let Some(warning) = render::escalate_warning_line(&outcome) {
                eprintln!("{warning}");
            }
        }
        Cmd::Escalations { status } => {
            let outcome =
                escalate::run_escalations(&mut relay, status.map(EscalationStatus::from))?;
            for warning in render::escalations_rejected_lines(&outcome) {
                eprintln!("{warning}");
            }
            for line in render::escalations_lines(&outcome) {
                writeln!(out, "{line}")?;
            }
        }
        Cmd::Investigate { id, fresh } => {
            let outcome = escalate::run_investigate(
                &mut relay,
                &RealHerdr::new(),
                &settings.machine.value,
                &id,
                fresh,
            )?;
            writeln!(out, "{}", render::investigate_line(&outcome))?;
        }
        Cmd::Resolve { id, decision } => {
            let outcome = escalate::run_resolve(
                &mut relay,
                &RealLinear::new(),
                &id,
                &decision,
                &settings.human.value,
                now_ms(),
            )?;
            for line in render::resolve_lines(&outcome) {
                writeln!(out, "{line}")?;
            }
            for warning in render::resolve_warning_lines(&outcome) {
                eprintln!("{warning}");
            }
        }
        Cmd::Escalation {
            cmd: EscalationCmd::Show { id },
        } => {
            let esc = escalate::run_escalation_show(&mut relay, &id)?;
            write!(out, "{}", escalate::render_escalation_detail(&esc))?;
        }
        Cmd::Escalation {
            cmd: EscalationCmd::PostLinear { id },
        } => {
            let esc = escalate::run_escalation_post_linear(&mut relay, &RealLinear::new(), &id)?;
            writeln!(out, "{}", render::post_linear_line(&esc.filing.ticket))?;
        }
        Cmd::Whoami { json } => {
            // AGE-2018 PR review, finding 11: a Redis failure inside
            // `derive` propagates as a real error here (via `?`), not a
            // silently-swallowed "not a lead".
            let identity = retry::read(&mut relay, &redial, |r| {
                derive_or_hint(r, &settings.machine.value)
            })?;
            let address = identity.as_ref().and_then(|id| {
                display_book(&mut relay, &redial)
                    .place_address(&id.name)
                    .map(ToString::to_string)
            });
            match &identity {
                Some(id) if json => {
                    serde_json::to_writer(
                        &mut out,
                        &identity::WhoamiJson::lead(id, address.as_deref()),
                    )?;
                    writeln!(out)?;
                }
                Some(id) => {
                    if let Some(address) = &address {
                        write!(out, "{address}  ")?;
                    }
                    write!(out, "({})", id.source.as_str())?;
                    writeln!(out)?;
                }
                None if json => {
                    serde_json::to_writer(&mut out, &identity::WhoamiJson::not_a_lead())?;
                    writeln!(out)?;
                }
                None => writeln!(out, "not a lead")?,
            }
        }
        Cmd::Hook { .. } | Cmd::Doctor { .. } => unreachable!("handled in main"),
    }
    Ok(())
}

/// [`identity::derive`], with a way out added to the collision error: the
/// derived name is held by another live lead, so this one must be told a name.
fn derive_or_hint(
    relay: &mut Relay,
    machine: &MachineName,
) -> Result<Option<identity::Identity>, BoxError> {
    identity::derive(relay, machine).map_err(|e| match e {
        identity::IdentityError::Relay(e) => e.into(),
    })
}

/// Resolves this invocation's identity: `--as`/`AGENT_MSG_NAME` verbatim
/// when set, else a name derived from this pane's herdr location
/// ([`identity::derive`], AGE-2018).
fn me(
    explicit_name: Option<&str>,
    relay: &mut Relay,
    machine: &MachineName,
) -> Result<AgentName, BoxError> {
    let _ = explicit_name; // Preserved environment metadata, never a routing identity.
    // AGE-2018 PR review, finding 11: a genuine Redis failure inside
    // `derive` must surface as itself, not get relabeled "set
    // AGENT_MSG_NAME" — that message is only for the real "not a lead" case
    // (`Ok(None)`).
    match derive_or_hint(relay, machine)? {
        Some(id) => Ok(id.name),
        None => Err(
            "no verified lead location; run the genuine terminal session in Herdr (Codex requires --no-daemon)"
                .into(),
        ),
    }
}

#[cfg(test)]
fn registration(
    name: AgentName,
    machine: MachineName,
    kind: &str,
    session_id: Option<relay_core::SessionId>,
    cwd: Option<String>,
    _place: Option<relay_core::Place>,
) -> Registration {
    // Set whenever this process runs inside a herdr-managed pane (every lead
    // session does); `relay-watcher` needs it to type messages in later.
    let herdr_pane = _place
        .as_ref()
        .and_then(|p| p.herdr_ids.as_ref())
        .map(|ids| ids.pane.clone());
    let place = _place;
    Registration {
        name,
        machine,
        kind: kind.to_owned(),
        session_id,
        cwd,
        herdr_pane,
        place,
        last_seen_ms: now_ms(),
    }
}

/// Who `send` is addressed to, and the registry when resolving it had to read it.
struct SendTarget {
    directory: Option<relay_core::DirectoryRecipient>,
    target: relay_core::Target,
    /// Verified recipient metadata gathered during address resolution.
    registry: Option<Vec<Registration>>,
    /// The named lead's record exists but this binary cannot read it.
    unreadable: bool,
    /// The unique live recipient enrolled by address resolution.
    recipient: Option<Registration>,
}

/// Retry only an atomic directory rejection, which proves nothing was queued.
/// Transport timeouts remain uncertain and are never automatically retried.
fn enqueue_resolved(
    relay: &mut Relay,
    resolved: &mut SendTarget,
    msg: &mut Message,
) -> relay_core::Result<Vec<AgentName>> {
    if matches!(msg.to, relay_core::Target::All) {
        return relay.send_directory_broadcast(msg);
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        let Some(proof) = &resolved.directory else {
            return relay.send(msg);
        };
        match relay.send_directory(msg, proof) {
            Err(relay_core::Error::DirectoryChanged) if std::time::Instant::now() < deadline => {
                let address = proof
                    .registration
                    .full_address()
                    .ok_or(relay_core::Error::DirectoryChanged)?;
                let relay_core::DirectoryRead::Unique(next) =
                    relay.directory_recipient(&address)?
                else {
                    return Err(relay_core::Error::DirectoryChanged);
                };
                msg.to = relay_core::Target::Agent(next.registration.name.clone());
                resolved.recipient = Some(next.registration.clone());
                resolved.registry = Some(vec![next.registration.clone()]);
                resolved.directory = Some(*next);
            }
            result => return result,
        }
    }
}

/// The registration of the single lead `queued_for`, from what resolving the
/// target already read: what `send` describes the recipient from, without another
/// read. `None` for a broadcast or when neither source has it.
fn recipient_registration(
    recipient: Option<Registration>,
    registry: Option<&[Registration]>,
    queued_for: &[AgentName],
) -> Option<Registration> {
    let [name] = queued_for else {
        return None;
    };
    recipient
        .filter(|r| r.name == *name)
        .or_else(|| registry?.iter().find(|r| r.name == *name).cloned())
}

/// Resolves full and relative addresses through the relay directory.
/// The unique genuine recipient is enrolled before its durable queue is used.
fn resolve_send_target(
    relay: &mut Relay,
    machine: &MachineName,
    input: &str,
) -> Result<SendTarget, BoxError> {
    if input == "all" {
        if !relay.directory_available(machine)? {
            relay_ops::enrollment::refresh_local_directory(relay, machine)?;
        }
        return Ok(SendTarget {
            directory: None,
            target: relay_core::Target::All,
            registry: None,
            unreadable: false,
            recipient: None,
        });
    }
    let address = if let Some(relative) = input.strip_prefix(':') {
        let identity = identity::derive(relay, machine)?
            .ok_or("relative addresses require this session's verified Herdr location")?;
        let sender = relay
            .agent(&identity.name)?
            .ok_or("sender enrollment missing")?;
        let place = sender
            .verified_place()
            .ok_or("sender location unverified")?;
        let expanded = match relative.split(':').collect::<Vec<_>>().as_slice() {
            [tab] if !tab.is_empty() => format!("{machine}:{}:{tab}", place.workspace),
            [workspace, tab] if !workspace.is_empty() && !tab.is_empty() => {
                format!("{machine}:{workspace}:{tab}")
            }
            _ => return Err("use :tab or :workspace:tab".into()),
        };
        relay_ops::enrollment::validate_address(&expanded)?
    } else {
        relay_ops::enrollment::validate_address(input)?
    };
    let directory = relay_ops::enrollment::recipient(relay, machine, &address)?;
    let recipient = directory.registration.clone();
    Ok(SendTarget {
        directory: Some(directory),
        target: relay_core::Target::Agent(recipient.name.clone()),
        registry: Some(vec![recipient.clone()]),
        unreadable: false,
        recipient: Some(recipient),
    })
}

/// Addresses for printing, or why the registry could not be read.
fn try_address_book(relay: &mut Relay) -> relay_core::Result<AddressBook> {
    let list = relay.agents()?;
    Ok(AddressBook::from_registrations(
        &list.into_registrations_reporting(|skipped| {
            report::line(format_args!("agent-msg: {skipped}"));
        }),
    ))
}

/// Addresses for printing. A registry read failure only costs the labels:
/// unresolved identities are marked unverified, and the command carries on.
fn address_book(relay: &mut Relay) -> AddressBook {
    try_address_book(relay).unwrap_or_else(|e| {
        report::line(format_args!(
            "agent-msg: could not read addresses; unresolved labels are unverified: {e}"
        ));
        AddressBook::default()
    })
}

/// [`address_book`] for a command's display, which only reads: a timed-out read
/// is retried on a fresh connection (see [`retry`]) before the labels are given up.
fn display_book(relay: &mut Relay, redial: &retry::Redial) -> AddressBook {
    retry::read(relay, redial, |r| Ok(try_address_book(r)?)).unwrap_or_else(|e| {
        report::line(format_args!(
            "agent-msg: could not read addresses; unresolved labels are unverified: {e}"
        ));
        AddressBook::default()
    })
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}
