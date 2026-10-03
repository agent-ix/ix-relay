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
mod status;
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
    AddressBook, AgentName, EscalationStatus, MachineName, Message, PaneId, PaneRef, Place,
    Registration, Relay, SessionId,
};
use relay_ops::{
    clock,
    escalate::{self, EscalateArgs},
    herdr::RealHerdr,
    linear::RealLinear,
    naming::HerdrEnv,
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
const MAX_HISTORY_REPORT: usize = 500;

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
    /// Your lead name. Signs what you send and picks your inbox.
    ///
    /// Taken as a raw string here, not `AgentName`: a hook must never fail to
    /// parse its way into a nonzero exit (see `hook_run::run`), so validation happens
    /// per command instead of at the clap layer.
    #[arg(long = "as", env = "AGENT_MSG_NAME", global = true)]
    name: Option<String>,
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
        /// Who to send to: `server:workspace[:tab[:pane]]` (e.g. `hub:EA:1` or
        /// `hub:EA:code:wA-p1`; without a tab this selects the workspace and
        /// without a pane it selects the whole tab),
        /// or `all`. A bus name, alias or herdr pane id also works.
        /// Resolved here, so no `find` is needed first.
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
        #[arg(long, conflicts_with_all = ["count", "peek", "dead", "in_flight"])]
        json: bool,
        /// Print the number of pending messages without removing anything,
        /// so a caller can decide whether it's worth starting a turn to read
        /// them (the opencode plugin's idle path does this).
        #[arg(long, conflicts_with_all = ["json", "peek", "dead", "in_flight"])]
        count: bool,
        /// Show a bounded JSON preview without removing messages.
        #[arg(long, conflicts_with_all = ["json", "count", "dead", "in_flight"])]
        peek: bool,
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
    /// Look up an agent by name, alias, workspace/tab label, session name,
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
        /// Print one stable JSON document including machine reachability and watcher freshness.
        #[arg(long, conflicts_with = "json")]
        report_json: bool,
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
    /// Report this machine's relay, watcher and verified lead status.
    Status {
        /// Print a stable JSON object instead of the compact Herdr status text.
        #[arg(long)]
        json: bool,
    },
    /// Print a compact, read-only status and pending-count badge for Herdr.
    Badge,
    /// Show recent message history.
    Log {
        /// How far back, e.g. 30m, 1h, 2d.
        #[arg(long, default_value = "1h")]
        since: Since,
        /// Most messages to show.
        #[arg(long, default_value_t = 100)]
        max: usize,
        /// Emit a stable JSON history report.
        #[arg(long)]
        json: bool,
    },
    /// Register yourself by hand. Hooks do this on session start.
    Register {
        /// Harness kind.
        #[arg(long, default_value = "manual")]
        kind: String,
    },
    /// Print this session's bus name and where it came from: an explicit
    /// `AGENT_MSG_NAME`/`--as`, or one resolved via `agent_msg::identity`'s
    /// name-precedence chain (AGE-2018) — see that module's doc comment for
    /// the 4 real precedence sources (`rename-title`, `herdr-agent-name`,
    /// `location`, plus `explicit`) and the 5th, non-precedence source,
    /// `registry`, an interactive command like this one falls back to.
    /// Prints `not a lead` (or, with `--json`, `{"lead":false}`) when
    /// nothing resolves — always exits 0, since this is a query, not an
    /// action.
    Whoami {
        /// Print `{"name": "...", "source": "..."}` (`source` is one of
        /// `explicit`, `rename-title`, `herdr-agent-name`, `location`,
        /// `registry` — see `identity::Source`), or `{"lead": false}` when
        /// this session isn't a lead — for a caller that parses the output
        /// (e.g. the opencode plugin, once at startup).
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

impl HookKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }
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

    if let Cmd::Hook { kind } = cli.cmd {
        // No explicit name and not running inside a herdr-managed pane
        // either: stay silent, exactly as before AGE-2018 — checked *before*
        // the config file is ever read, so a malformed
        // `~/.ix/relay/config.toml` never prints anything for a session
        // that's clearly not a lead (no name to derive at all). A session
        // with no explicit name but *inside* herdr still needs the config
        // loaded next, to reach the registry for the derived-name collision
        // check.
        let has_explicit_name = matches!(cli.name.as_deref(), Some(n) if !n.is_empty());
        if !has_explicit_name && relay_ops::naming::HerdrEnv::from_process_env().is_none() {
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
            cli_name: cli.name.as_deref(),
            kind,
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
    if let Cmd::Status { json } = cli.cmd {
        return status::run(core_flags(&cli), json);
    }
    if let Cmd::Badge = cli.cmd {
        return status::badge(core_flags(&cli));
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
            let resolved = resolve_send_target(&mut relay, &settings.machine.value, &to)?;
            let broadcast = matches!(resolved.target, relay_core::Target::All);
            let unreadable = resolved.unreadable;
            let msg = Message {
                from: me(cli.name.as_deref(), &mut relay, &settings.machine.value)?,
                to: resolved.target,
                text: text.join(" "),
                sent_at_ms: now_ms(),
            };
            let recipients = relay.send(&msg)?;
            let recipient = recipient_registration(
                resolved.recipient,
                resolved.registry.as_deref(),
                &recipients,
            );
            // The confirmation names the recipients by address where it can.
            // A send by bus name or alias read only that one lead's record, so
            // it confirms by name and reads nothing more (AGE-2028); an address
            // or pane id already read the registry, and a broadcast names every
            // lead, so it reads it once.
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
            let name = me(cli.name.as_deref(), &mut relay, &settings.machine.value)?;
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
            let name = me(cli.name.as_deref(), &mut relay, &settings.machine.value)?;
            let n = relay.inbox_len(&name)?;
            writeln!(out, "{n}")?;
        }
        Cmd::Inbox {
            peek: true, max, ..
        } => {
            let name = me(cli.name.as_deref(), &mut relay, &settings.machine.value)?;
            let found = relay.peek_inbox(&name, max)?;
            let report = inbox_report::PeekReport::new(&found);
            serde_json::to_writer(&mut out, &report)?;
            writeln!(out)?;
        }
        Cmd::Inbox {
            max,
            json,
            count: false,
            dead: false,
            in_flight: false,
            peek: false,
        } => {
            let name = me(cli.name.as_deref(), &mut relay, &settings.machine.value)?;
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
            // AGE-2018 PR review, finding 11: one `all_aliases` round trip
            // for every row, instead of one `aliases_of` call per registered
            // lead (which made `who`'s Redis cost O(leads), not O(1)).
            let (all_aliases, list, replaced) = retry::read(&mut relay, &redial, |r| {
                let aliases = r.all_aliases()?;
                let list = r.agents()?;
                let names: Vec<&AgentName> = list.registrations.iter().map(|x| &x.name).collect();
                let replaced = r.panes_replaced(&names);
                Ok((aliases, list, replaced))
            })?;
            // Whether a pane now hosts another session is extra: a failed read of
            // it is said, and the listing is still printed.
            let replaced = replaced.map(|p| p.found).unwrap_or_else(|e| {
                eprintln!("agent-msg who: could not read which panes were replaced: {e}");
                std::collections::BTreeMap::new()
            });
            let mut aliases_by_canonical: std::collections::BTreeMap<AgentName, Vec<AgentName>> =
                std::collections::BTreeMap::new();
            for (alias, canonical) in all_aliases {
                aliases_by_canonical
                    .entry(canonical)
                    .or_default()
                    .push(alias);
            }
            for aliases in aliases_by_canonical.values_mut() {
                aliases.sort();
            }
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
                // AGE-2018: any name(s) this lead is also reachable as
                // (e.g. its herdr-location name, kept as an alias once a
                // `/rename` or a herdr agent name takes over as canonical).
                let empty = Vec::new();
                let aliases = aliases_by_canonical.get(&r.name).unwrap_or(&empty);
                write!(
                    out,
                    "{address}  {}  {machine}  {kind}  seen {seen}  pane {pane}  {cwd}  {place_cache}",
                    r.name
                )?;
                if !aliases.is_empty() {
                    let names = aliases.iter().map(AgentName::as_str).collect::<Vec<_>>();
                    write!(out, "  aka {}", names.join(", "))?;
                }
                // FR-032: the watcher found this lead's pane hosting another
                // session, so what is listed for it is stale, not live.
                if let Some(found) = replaced.get(&r.name).filter(|f| f.describes(&r)) {
                    write!(
                        out,
                        "  UNDELIVERABLE: pane {pane} now hosts another session (since {})",
                        clock(found.since_ms)
                    )?;
                }
                writeln!(out)?;
            }
        }
        Cmd::Find {
            query,
            json,
            report_json,
        } => {
            let query = query.unwrap_or_default();
            let report = find::run(&mut relay, &redial, &settings.machine.value, &query)?;
            for skipped in &report.skipped {
                eprintln!(
                    "agent-msg find: skipped {}: {}",
                    skipped.machine, skipped.reason
                );
            }
            if report_json {
                serde_json::to_writer(&mut out, &find::report_json(&report))?;
                writeln!(out)?;
            } else if json {
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
        Cmd::Log { since, max, json } => {
            let window = i64::try_from(since.0.as_millis()).unwrap_or(i64::MAX);
            let book = display_book(&mut relay, &redial);
            let since_ms = now_ms().saturating_sub(window);
            let effective_max = max.min(MAX_HISTORY_REPORT);
            let fetch_max = effective_max.saturating_add(1);
            let mut messages =
                retry::read(&mut relay, &redial, |r| Ok(r.log(since_ms, fetch_max)?))?;
            let truncated = messages.len() > effective_max;
            messages.truncate(effective_max);
            if json {
                let report = inbox_report::HistoryReport::new(&messages, truncated);
                serde_json::to_writer(&mut out, &report)?;
                writeln!(out)?;
            } else {
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
        }
        Cmd::Register { kind } => {
            let cwd = std::env::current_dir()
                .ok()
                .map(|p| p.display().to_string());
            let machine = settings.machine.value.clone();
            let name = me(cli.name.as_deref(), &mut relay, &settings.machine.value)?;
            let place = identity::current_place().or(registered_place(&mut relay, &name)?);
            relay.register(&registration(name, machine, &kind, None, cwd, place))?;
            writeln!(out, "registered")?;
        }
        Cmd::Escalate {
            ticket,
            context,
            option,
            recommend,
        } => {
            let name = me(cli.name.as_deref(), &mut relay, &settings.machine.value)?;
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
            let identity = match cli.name.as_deref() {
                None | Some("") => retry::read(&mut relay, &redial, |r| {
                    derive_or_hint(r, &settings.machine.value)
                })?,
                Some(raw) => Some(identity::Identity {
                    name: raw.parse()?,
                    source: identity::Source::Explicit,
                    location_alias: None,
                }),
            };
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
                    write!(out, "{} ({})", id.name, id.source.as_str())?;
                    if let Some(alias) = &id.location_alias {
                        write!(out, ", reachable as {alias}")?;
                    }
                    writeln!(out)?;
                }
                None if json => {
                    serde_json::to_writer(&mut out, &identity::WhoamiJson::not_a_lead())?;
                    writeln!(out)?;
                }
                None => writeln!(out, "not a lead")?,
            }
        }
        Cmd::Hook { .. } | Cmd::Doctor { .. } | Cmd::Status { .. } | Cmd::Badge => {
            unreachable!("handled in main")
        }
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
        identity::CollisionError::Relay(e) => e.into(),
        identity::CollisionError::NoFreeName { .. } => {
            format!("{e}; pass --as <name> or set AGENT_MSG_NAME").into()
        }
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
    if let Some(raw) = explicit_name {
        return Ok(raw.parse::<AgentName>()?);
    }
    // AGE-2018 PR review, finding 11: a genuine Redis failure inside
    // `derive` must surface as itself, not get relabeled "set
    // AGENT_MSG_NAME" — that message is only for the real "not a lead" case
    // (`Ok(None)`).
    match derive_or_hint(relay, machine)? {
        Some(id) => Ok(id.name),
        None => Err(
            "set AGENT_MSG_NAME (or pass --as), or run this inside a herdr-managed pane, \
         to establish your lead name"
                .into(),
        ),
    }
}

fn registration(
    name: AgentName,
    machine: MachineName,
    kind: &str,
    session_id: Option<SessionId>,
    cwd: Option<String>,
    _place: Option<Place>,
) -> Registration {
    // Set whenever this process runs inside a herdr-managed pane (every lead
    // session does); `relay-watcher` needs it to type messages in later.
    let herdr_pane = HerdrEnv::pane_id_from_process_env();
    // Hook and interactive processes may inherit Herdr's pane variables from a
    // shared harness daemon. Keep the human labels as a claim, but only the
    // watcher may attach the verified Herdr IDs after matching the registered
    // harness session to a live Herdr agent.
    // Human labels from this process are not a verification source. The
    // watcher will attach a complete Place only after Herdr confirms the
    // registration's session at the claimed pane.
    let place = None;
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
    target: relay_core::Target,
    /// Every decodable registration, present only for a pane id or a
    /// `server:workspace[:tab[:pane]]` address.
    registry: Option<Vec<Registration>>,
    /// The named lead's record exists but this binary cannot read it.
    unreadable: bool,
    /// The one recipient's registration when resolving it read it: a bus name or
    /// alias reads exactly that record.
    recipient: Option<Registration>,
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

/// Turns what `send` was given into who it goes to. A bus name, alias or `all`
/// is checked against just that one registration (two point reads: the alias
/// table and the agent), so one undecodable record elsewhere in the registry
/// cannot block a plain send and its cost does not grow with the number of
/// leads; only a pane id or a `server:workspace[:tab[:pane]]` address reads the whole
/// registry.
fn resolve_send_target(
    relay: &mut Relay,
    machine: &MachineName,
    input: &str,
) -> Result<SendTarget, BoxError> {
    if let Ok(target) = input.parse::<relay_core::Target>() {
        match target {
            relay_core::Target::All => {
                return Ok(SendTarget {
                    target,
                    registry: None,
                    unreadable: false,
                    recipient: None,
                });
            }
            relay_core::Target::Agent(name) => {
                let canonical = relay.resolve_alias(&name)?;
                // A record this binary cannot decode still names a registered
                // lead (an older writer's odd id): send to it, as before.
                let slot = relay.agent_slot(&canonical)?;
                if slot != relay_core::AgentSlot::Absent {
                    let unreadable = slot == relay_core::AgentSlot::Unreadable;
                    let recipient = match slot {
                        relay_core::AgentSlot::Registered(reg) => Some(*reg),
                        _ => None,
                    };
                    return Ok(SendTarget {
                        target: relay_core::Target::Agent(canonical),
                        registry: None,
                        unreadable,
                        recipient,
                    });
                }
            }
        }
    }
    let registrations = relay
        .agents()?
        .into_registrations_reporting(|skipped| eprintln!("agent-msg send: {skipped}"));
    let sender_place = pane_place(relay, machine)?.map(|place| place.address(machine));
    let target = relay_core::resolve_target_scoped(
        input,
        machine,
        sender_place.as_ref(),
        &registrations,
        &relay.all_aliases()?,
    )?;
    Ok(SendTarget {
        target,
        registry: Some(registrations),
        unreadable: false,
        recipient: None,
    })
}

/// The workspace/tab last registered for `name`, so a re-registration that
/// made no herdr call does not erase it.
fn registered_place(relay: &mut Relay, name: &AgentName) -> relay_core::Result<Option<Place>> {
    Ok(match relay.agent_slot(name)? {
        relay_core::AgentSlot::Registered(reg) => reg.place,
        relay_core::AgentSlot::Absent | relay_core::AgentSlot::Unreadable => None,
    })
}

/// The workspace/tab this pane last registered under *any* name. Read before
/// a hook derives its name: a `/rename` demotes the old name to an alias,
/// which deletes its registration — and the place with it — before the new
/// name registers.
fn pane_place(relay: &mut Relay, machine: &MachineName) -> relay_core::Result<Option<Place>> {
    let Some(pane) = HerdrEnv::pane_id_from_process_env() else {
        return Ok(None);
    };
    // Pane ids repeat across machines: only this machine's registrations count.
    Ok(relay
        .registrations_for_pane(&PaneRef::new(machine.clone(), pane))?
        .into_iter()
        .find_map(|r| r.verified_place().cloned()))
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
/// bare bus names are printed instead, and the command carries on.
fn address_book(relay: &mut Relay) -> AddressBook {
    try_address_book(relay).unwrap_or_else(|e| {
        report::line(format_args!(
            "agent-msg: could not read addresses, showing bus names: {e}"
        ));
        AddressBook::default()
    })
}

/// [`address_book`] for a command's display, which only reads: a timed-out read
/// is retried on a fresh connection (see [`retry`]) before the labels are given up.
fn display_book(relay: &mut Relay, redial: &retry::Redial) -> AddressBook {
    retry::read(relay, redial, |r| Ok(try_address_book(r)?)).unwrap_or_else(|e| {
        report::line(format_args!(
            "agent-msg: could not read addresses, showing bus names: {e}"
        ));
        AddressBook::default()
    })
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}
