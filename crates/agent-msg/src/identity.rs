//! Resolving "who am I on the bus" for one process (AGE-2018): the explicit
//! `AGENT_MSG_NAME`/`--as` override when given, else a name-precedence chain
//! (the operator's 2026-09-25 correction), highest priority first:
//!
//! 1. `AGENT_MSG_NAME`/`--as` (handled by the caller, not this module);
//! 2. the last Claude `/rename` title recorded in the session's transcript
//!    (Claude only — Codex and opencode have no transcript to read);
//! 3. a herdr agent name (`herdr agent rename`/`agent start <name>`), read
//!    from `herdr agent get`'s `name` field, present only while a rename is
//!    in effect (see [`relay_ops::naming::LocationHerdr::agent`]'s own
//!    doc comment);
//! 4. a name derived from this pane's herdr location.
//!
//! These 4 are the only real precedence sources (AGE-2018 PR review, finding
//! 10) — every hook call resolves through exactly one of them. A 5th value,
//! [`Source::Registry`], is not a precedence tier at all: it is what
//! *interactive* commands (`send`, `inbox`, `whoami`, ...) report when they
//! resolve by reading the bus registry instead of re-running this chain —
//! see [`derive()`]'s own doc comment (finding 4).
//!
//! Whichever tier wins, the herdr *location* name is kept reachable as an
//! alias of it when they differ, so a message sent to the location-derived
//! name (what a lead was reachable as before a `/rename`, say) still
//! arrives. When a session's canonical name changes a *second* time (e.g.
//! a `/rename` to "b", then again to "c"), the now-demoted "b" becomes an
//! alias of "c" too, and anything that already pointed at "b" — including
//! the original location name — is repointed straight at "c", so
//! [`relay_core::Relay::resolve_alias`] never needs more than one hop
//! (finding 5). Turning a name into an alias never silently deletes a
//! registration that belongs to a different session or pane — see
//! [`relay_core::Relay::commit_identity`] (finding 2).
//!
//! An escalation investigation fork (an `esc-*` tab or herdr agent name) or
//! a Claude `--fork-session` fork never joins the bus at all: no derived
//! name, no registration, no delivery, under *any* tier — see
//! [`derive_for_hook`] (finding 1).
//!
//! On the hook path, the resolved name is cached by harness session id so
//! `PostToolUse` and `Stop` skip straight to a Redis lookup instead of
//! re-deriving through herdr. `SessionStart` always re-derives the full
//! chain (workspace/tab/pane lookups, herdr agent name, transcript scan).
//! `UserPromptSubmit` re-derives too, so a `/rename` takes effect on the
//! very next prompt — but, per the operator's 2026-09-25 correction (finding 6),
//! makes *no herdr calls at all*: only the transcript (a local file read) is
//! re-scanned; the herdr-derived tiers are assumed unchanged since the last
//! full derivation (nothing about a pane's herdr location or its herdr
//! agent name changes mid-session without a new `SessionStart` anyway). A
//! `/rename` title still goes through collision resolution, from registry
//! reads alone: every registered same-machine session is assumed live (see
//! [`relay_ops::naming::AssumeHosted`]), which can cost a needless `-2`
//! suffix but never lets a rename take over a live lead's name.

use std::{cell::Cell, time::Duration};

use relay_core::{
    AgentName, Demotion, IdentityCommit, MachineName, PaneId, PaneRef, Place, Relay, SessionId,
    SessionProbe, WorkspaceId,
};
pub use relay_ops::naming::CollisionError;
use relay_ops::naming::{
    AssumeHosted, HerdrEnv, HerdrLocation, LocationHerdr, PaneEntry, ProcessError,
    RealLocationHerdr, TabEntry, WorkspaceEntry, derive_from_herdr, resolve_collision,
};

use crate::{budget::Budget, hook::Event};

/// Where a resolved name came from. `agent-msg whoami` prints this. See the
/// module doc comment for which of these are real precedence tiers (all but
/// [`Source::Registry`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// `AGENT_MSG_NAME`/`--as` was set.
    Explicit,
    /// The last Claude `/rename` title recorded in the session's transcript.
    RenameTitle,
    /// A herdr agent name (`herdr agent rename`/`agent start <name>`).
    HerdrAgentName,
    /// Derived from this pane's herdr location, per AGE-2018's naming rule.
    Location,
    /// Resolved by looking up this pane's current registration in the bus
    /// registry, rather than re-running the precedence chain — what an
    /// interactive command uses, since it has no transcript to check for a
    /// `/rename` the way a hook does (AGE-2018 PR review, finding 4). Not a
    /// precedence tier: it's *how* the already-resolved name was found, not
    /// which tier originally produced it.
    Registry,
}

impl Source {
    /// The word `whoami` prints for this source.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
            Self::RenameTitle => "rename-title",
            Self::HerdrAgentName => "herdr-agent-name",
            Self::Location => "location",
            Self::Registry => "registry",
        }
    }
}

/// A resolved identity: the name, where it came from, and — when it
/// differs from the plain herdr-location name — the location name kept
/// reachable as an alias.
#[derive(Debug, Clone)]
pub struct Identity {
    /// The bus name.
    pub name: AgentName,
    /// Where it came from.
    pub source: Source,
    /// The herdr-location name, when it differs from `name`. Only ever set
    /// on the hook path, which is the only one that writes anything —
    /// [`derive()`] (the interactive path) never sets this, since it never
    /// registers or aliases anything as a side effect (finding 4).
    pub location_alias: Option<AgentName>,
}

/// One JSON shape for `agent-msg whoami --json` (AGE-2018 PR review, finding
/// 10): a typed enum instead of an ad hoc `serde_json::json!` literal, so
/// the two possible shapes are declared once, checked by the compiler, and
/// visible in one place instead of scattered across `match` arms.
#[derive(Debug, serde::Serialize)]
#[serde(untagged)]
pub enum WhoamiJson<'a> {
    /// A resolved identity.
    Lead {
        /// The bus name.
        name: &'a str,
        /// Where it came from — see [`Source::as_str`].
        source: &'a str,
        /// The location name, if kept reachable as an alias.
        location_alias: Option<&'a str>,
        /// The lead's `server:workspace[:tab[:pane]]` address, when it registered
        /// one — what other leads send to.
        address: Option<&'a str>,
    },
    /// Nothing resolved: not a lead.
    NotALead {
        /// Always `false`.
        lead: bool,
    },
}

impl<'a> WhoamiJson<'a> {
    /// Builds the shape for a resolved `identity`.
    #[must_use]
    pub fn lead(identity: &'a Identity, address: Option<&'a str>) -> Self {
        Self::Lead {
            address,
            name: identity.name.as_str(),
            source: identity.source.as_str(),
            location_alias: identity.location_alias.as_ref().map(AgentName::as_str),
        }
    }

    /// The shape for "not a lead".
    #[must_use]
    pub fn not_a_lead() -> Self {
        Self::NotALead { lead: false }
    }
}

/// Total herdr-location budget for an interactive CLI command (`send`,
/// `inbox`, `escalate`, `register`, `whoami`, ...) that needs to derive an
/// identity because `AGENT_MSG_NAME`/`--as` wasn't given and no
/// registration for this pane exists yet (see [`derive()`]). Generous: an
/// interactive command has no per-tool-call latency requirement the way a
/// hook does.
const INTERACTIVE_HERDR_BUDGET: Duration = Duration::from_millis(3000);

/// Total herdr-location budget at a hook's `SessionStart`: the same order of
/// magnitude as [`INTERACTIVE_HERDR_BUDGET`], since this is a one-time cost
/// per session and `SessionStart`'s own hook timeout (`hooks/hooks.json`) is
/// set wide enough to afford it.
pub(crate) const SESSION_START_HERDR_BUDGET: Duration = Duration::from_millis(3000);

/// Total herdr-location budget for a cache-miss lazy derive at `PostToolUse`
/// or `Stop` (e.g. the plugin was installed mid-session, so no
/// `SessionStart` ever ran to populate the session->name cache). Small: it is
/// part of the `PostToolUse`/`Stop` deadline — see `hook_run`'s
/// `hook_deadline_budget`.
pub(crate) const LAZY_HERDR_BUDGET: Duration = Duration::from_millis(600);

/// Timeout for the one extra herdr call [`resolve_collision`] sometimes
/// makes (checking whether another same-machine registration's pane is
/// still live — finding 8). Independent of the budgets above: this always
/// runs after the main precedence chain has already resolved, so it gets
/// its own small, fixed allowance rather than sharing whatever was left of
/// a bigger budget.
pub(crate) const COLLISION_HERDR_TIMEOUT: Duration = Duration::from_millis(800);

/// What the precedence chain produced: the winning name and its source, the
/// plain herdr-location name to keep reachable as an alias (when it
/// differs), and this pane's id (needed to check who owns a name before
/// aliasing it away — finding 2).
struct Chain {
    canonical: AgentName,
    source: Source,
    location_alias: Option<AgentName>,
    pane_id: PaneId,
    place: Option<Place>,
}

/// What a hook derived: the bus name, and — only when this call ran through
/// herdr — where the pane sits, for the registration's address. A call that
/// made no herdr call has no place; the caller keeps the one already
/// registered.
///
/// Deriving only *reads*. Every write it implies (alias demotions, the
/// session-to-name cache) is in `commit`, for the caller to apply together with
/// the registration in one step once it knows it has time left
/// ([`relay_core::Relay::commit_identity`]) — so a hook that runs out of time
/// midway leaves nothing half-written.
#[derive(Debug, Clone)]
pub struct Derived {
    /// The bus name.
    pub name: AgentName,
    /// This pane's workspace and tab, when freshly read from herdr.
    pub place: Option<Place>,
    /// The writes deriving `name` implies, not yet applied.
    pub commit: IdentityCommit,
    /// Set when a holder's liveness could not be checked and it was treated as
    /// live, so `name` is a numbered variant of what herdr named this pane.
    pub note: Option<LivenessNote>,
}

impl Derived {
    /// A name that needs no writes: it came from the cache, or `--as`.
    pub fn cached(name: AgentName) -> Self {
        Self {
            name,
            place: None,
            commit: IdentityCommit::default(),
            note: None,
        }
    }
}

/// Why a holder of the derived name was treated as live without being checked
/// (FR-022). Not a failure of the hook: the lead registers under the next free
/// name. The hook records it.
#[derive(Debug, Clone)]
pub enum LivenessNote {
    /// herdr could not be asked, or answered with an error.
    HerdrDown {
        /// The name the holder has.
        candidate: AgentName,
        /// What herdr said.
        source: ProcessError,
    },
    /// The hook's time budget was spent before herdr could be asked.
    BudgetSpent {
        /// The name the holder has.
        candidate: AgentName,
    },
}

impl LivenessNote {
    /// The name the holder has.
    pub fn candidate(&self) -> &AgentName {
        match self {
            Self::HerdrDown { candidate, .. } | Self::BudgetSpent { candidate } => candidate,
        }
    }
}

impl std::fmt::Display for LivenessNote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HerdrDown { candidate, source } => write!(
                f,
                "could not tell whether the holder of {candidate} is live ({source}); \
                 treating it as live"
            ),
            Self::BudgetSpent { candidate } => write!(
                f,
                "out of time to tell whether the holder of {candidate} is live; \
                 treating it as live"
            ),
        }
    }
}

/// A name chosen for a lead, and the note that came with it.
struct Named {
    name: AgentName,
    note: Option<LivenessNote>,
}

/// What [`derive_for_hook`] decided about a hook call.
#[derive(Debug, Clone)]
pub enum HookIdentity {
    /// Not a lead for this call: the hook stays silent, exactly like an unset
    /// `AGENT_MSG_NAME` does.
    NotALead,
    /// This session must never join the bus (finding 1): the caller records
    /// that, once, with its other identity writes.
    Suppress {
        /// The session to mark. `None` when the harness gave no id, so there
        /// is nothing to remember.
        session_id: Option<SessionId>,
    },
    /// A lead, under this name.
    Lead(Box<Derived>),
    /// The hook runs in a herdr pane but herdr could not say who this session is
    /// (a call failed, timed out or was never made for want of time) and nothing
    /// else named it. Not "not a lead": the caller records the failure, so a hook
    /// that stays silent is silent because there is nothing to report.
    HerdrFailed {
        /// Whether the hook's time budget was spent before herdr could be asked
        /// (or while it was), as opposed to herdr itself failing.
        budget_spent: bool,
    },
}

/// What [`resolve_chain`] found.
enum ChainOutcome {
    /// A precedence tier won.
    Winner(Box<Chain>),
    /// This session must never join the bus at all — an escalation
    /// investigation fork, by tab label or herdr agent name (finding 1).
    /// Distinct from [`ChainOutcome::None`]: a caller must actively
    /// remember this (see [`HookIdentity::Suppress`]), not
    /// just decline to register once.
    Suppressed,
    /// Nothing resolved and this isn't a fork either — not running inside
    /// herdr at all, or the location couldn't be resolved and neither of
    /// the other two tiers had anything.
    None,
    /// Nothing resolved *because* a herdr call failed: the pane may well be a
    /// lead, so this is a failure to report, not silence.
    HerdrFailed {
        /// Whether a call was refused for want of budget.
        budget_spent: bool,
    },
}

/// A herdr client whose every call is bounded by what is left of a hook's
/// deadline, not just by a fixed share of it: `cap` limits any one call, and
/// `budget` (when there is one) limits it further to the time remaining, so
/// however many calls a derivation makes they cannot run past the deadline.
/// With no `budget` (an interactive command, which has none) only `cap` applies.
/// Once the budget is spent a call fails at once with [`ProcessError::Timeout`]
/// instead of starting.
struct BoundedHerdr<'a, F> {
    cap: Duration,
    budget: Option<&'a Budget<'a>>,
    make: F,
    /// Set once a call was refused because the budget was spent, so a caller
    /// can tell "out of time" from "herdr failed".
    spent: Cell<bool>,
}

impl<'a, F, L> BoundedHerdr<'a, F>
where
    F: Fn(Duration) -> L,
    L: LocationHerdr,
{
    fn new(cap: Duration, budget: Option<&'a Budget<'a>>, make: F) -> Self {
        Self {
            cap,
            budget,
            make,
            spent: Cell::new(false),
        }
    }

    /// Whether any call was refused for want of budget.
    fn budget_was_spent(&self) -> bool {
        self.spent.get()
    }

    /// The herdr client for the next call, with its timeout.
    fn next(&self) -> Result<L, ProcessError> {
        let timeout = match self.budget {
            None => self.cap,
            Some(budget) => budget
                .remaining()
                .map(|left| left.min(self.cap))
                .ok_or_else(|| {
                    self.spent.set(true);
                    ProcessError::Timeout {
                        program: "herdr".to_owned(),
                        secs: 0,
                    }
                })?,
        };
        Ok((self.make)(timeout))
    }
}

impl<F, L> LocationHerdr for BoundedHerdr<'_, F>
where
    F: Fn(Duration) -> L,
    L: LocationHerdr,
{
    fn workspace_list(&self) -> Result<Vec<WorkspaceEntry>, ProcessError> {
        self.next()?.workspace_list()
    }

    fn tab_list(&self, workspace_id: &WorkspaceId) -> Result<Vec<TabEntry>, ProcessError> {
        self.next()?.tab_list(workspace_id)
    }

    fn pane_list(&self, workspace_id: &WorkspaceId) -> Result<Vec<PaneEntry>, ProcessError> {
        self.next()?.pane_list(workspace_id)
    }

    fn agent(
        &self,
        pane_id: &PaneId,
    ) -> Result<Option<relay_ops::naming::HerdrAgent>, ProcessError> {
        self.next()?.agent(pane_id)
    }
}

/// The real Herdr client: every command can use the remaining shared deadline,
/// so shorter earlier calls leave more room for later subprocesses.
fn real_herdr<'a>(
    total: Duration,
    budget: Option<&'a Budget<'a>>,
) -> BoundedHerdr<'a, fn(Duration) -> RealLocationHerdr> {
    BoundedHerdr::new(total, budget, RealLocationHerdr::new)
}

/// Runs the name-precedence chain (see this module's doc comment). Each Herdr
/// call gets the remaining overall deadline, never more than what is
/// left of `budget`. `transcript_path` is `None` for anything but a Claude
/// hook call.
fn resolve_chain(
    transcript_path: Option<&str>,
    herdr_budget: Duration,
    budget: Option<&Budget<'_>>,
) -> ChainOutcome {
    let Some(env) = HerdrEnv::from_process_env() else {
        return ChainOutcome::None;
    };
    let herdr = real_herdr(herdr_budget, budget);
    chain_from(&herdr, &env, transcript_path)
}

/// [`resolve_chain`] over any bounded herdr and an environment already read.
fn chain_from<F, L>(
    herdr: &BoundedHerdr<'_, F>,
    env: &HerdrEnv,
    transcript_path: Option<&str>,
) -> ChainOutcome
where
    F: Fn(Duration) -> L,
    L: LocationHerdr,
{
    // Set when a herdr call failed: with no name found, that is a failure to
    // report, not a pane that is not a lead.
    let mut herdr_failed = false;
    let location = match derive_from_herdr(herdr, env) {
        Ok(loc) => loc,
        Err(e) => {
            herdr_failed = true;
            crate::report::line(format_args!("agent-msg: herdr location lookup failed: {e}"));
            HerdrLocation::default()
        }
    };
    if location.is_escalation_fork {
        return ChainOutcome::Suppressed;
    }
    let mut winner = location.name.clone().map(|name| (name, Source::Location));

    if let Some(raw) = location
        .agent
        .as_ref()
        .and_then(|agent| agent.name.as_deref())
    {
        if raw.starts_with(relay_core::naming::ESCALATION_TAB_PREFIX) {
            return ChainOutcome::Suppressed;
        }
        winner = Some((relay_core::naming::slugify(raw), Source::HerdrAgentName));
    }

    if let Some(path) = transcript_path
        && let Some(title) = custom_title_from_transcript(path)
    {
        winner = Some((relay_core::naming::slugify(&title), Source::RenameTitle));
    }

    match winner {
        Some((canonical, source)) => {
            let location_alias = location.name.filter(|loc| *loc != canonical);
            ChainOutcome::Winner(Box::new(Chain {
                canonical,
                source,
                location_alias,
                pane_id: env.pane_id.clone(),
                place: location.place,
            }))
        }
        None if herdr_failed => ChainOutcome::HerdrFailed {
            budget_spent: herdr.budget_was_spent(),
        },
        None => ChainOutcome::None,
    }
}

/// Most bytes [`custom_title_from_transcript`] reads, counted back from the
/// end of the file. This runs on every `UserPromptSubmit`, inside the hook's
/// tight overall timeout, and a long session's transcript runs to hundreds of
/// MB: an unbounded scan for a title written long ago would stall every
/// prompt. 8 MiB is a few milliseconds of reads. A `/rename` further back
/// than that is simply not seen; the session keeps the name it already has.
const MAX_TRANSCRIPT_SCAN_BYTES: u64 = 8 * 1024 * 1024;

/// Scans the last [`MAX_TRANSCRIPT_SCAN_BYTES`] of a Claude Code transcript
/// (JSONL) for the last line Claude writes when the user runs `/rename` —
/// `{"type":"custom-title","customTitle":"...","sessionId":...}` — and returns
/// its `customTitle`. `None` if the file can't be read, no such line is found
/// in that window, or the only candidate is the window's first line, cut off
/// mid-line by the bound.
///
/// Transcripts can run to many MB, so this reads from the end in bounded
/// chunks rather than parsing the whole file forward, and is careful to stay
/// O(file size) even in the pathological case of one huge line with no
/// newline in it anywhere (AGE-2018 PR review, finding 9): an earlier
/// version re-built and re-split a `String` combining the whole
/// not-yet-terminated tail on every chunk, which is O(file size²) when that
/// tail never completes. This version only ever appends each byte to a
/// carry buffer once and reads it back out once, when the line it belongs to
/// finally completes.
fn custom_title_from_transcript(path: &str) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    const CHUNK: u64 = 64 * 1024;

    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let mut pos = len;
    // Never read below this offset (0 for a small file).
    let floor = len.saturating_sub(MAX_TRANSCRIPT_SCAN_BYTES);
    // The tail of a line that hasn't been newline-terminated yet, held in
    // *reverse* byte order (nearest-EOF byte first) so that carrying a new
    // chunk's leading fragment into it is an O(chunk size) append, never an
    // O(existing carry size) copy — see the module doc comment above.
    let mut pending_rev: Vec<u8> = Vec::new();

    while pos > floor {
        let read_len = CHUNK.min(pos - floor);
        pos -= read_len;
        file.seek(SeekFrom::Start(pos)).ok()?;
        let mut buf = vec![0u8; usize::try_from(read_len).unwrap_or(usize::MAX)];
        file.read_exact(&mut buf).ok()?;

        let mut chunk_end = buf.len();
        loop {
            match buf[..chunk_end].iter().rposition(|&b| b == b'\n') {
                Some(newline) => {
                    let segment = &buf[newline + 1..chunk_end];
                    if pending_rev.is_empty() {
                        if let Some(title) = parse_custom_title_bytes(segment) {
                            return Some(title);
                        }
                    } else {
                        let mut line = segment.to_vec();
                        line.extend(pending_rev.iter().rev().copied());
                        pending_rev.clear();
                        if let Some(title) = parse_custom_title_bytes(&line) {
                            return Some(title);
                        }
                    }
                    chunk_end = newline;
                }
                None => {
                    // No newline left in what's left of this chunk: all of
                    // it is still part of the line currently being
                    // assembled. Append in reverse order so this stays an
                    // O(chunk size) operation regardless of how large
                    // `pending_rev` has already grown.
                    pending_rev.extend(buf[..chunk_end].iter().rev());
                    break;
                }
            }
        }
    }
    // Bytes still pending at the floor: the first line of the window. Only the
    // file's true first line (floor 0) is whole; otherwise the bound cut it.
    if pending_rev.is_empty() || floor > 0 {
        None
    } else {
        pending_rev.reverse();
        parse_custom_title_bytes(&pending_rev)
    }
}

/// Parses one transcript line (raw bytes, lossily decoded) as a
/// `custom-title` record and returns its `customTitle`, or `None` if it
/// isn't one.
fn parse_custom_title_bytes(line: &[u8]) -> Option<String> {
    let line = String::from_utf8_lossy(line);
    let line = line.trim();
    if line.is_empty() || !line.contains("custom-title") {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    if value.get("type")?.as_str()? != "custom-title" {
        return None;
    }
    value.get("customTitle")?.as_str().map(str::to_owned)
}

/// Runs [`resolve_collision`] with a live herdr check.
///
/// A herdr failure while asking whether the holder is still live treats it as
/// live (FR-022), so the lead gets the next free number and the failure comes
/// back as a [`LivenessNote`]. No free variant is an error, never "keep the
/// candidate": that would put two live leads on one inbox (FR-022-AC-6). The
/// caller does not register; see [`CollisionError`].
fn resolve_name(
    relay: &mut Relay,
    candidate: AgentName,
    machine: &MachineName,
    session_id: Option<&SessionId>,
    budget: Option<&Budget<'_>>,
) -> Result<Named, CollisionError> {
    let herdr = real_herdr(COLLISION_HERDR_TIMEOUT, budget);
    let resolved = resolve_collision(relay, &herdr, candidate.clone(), machine, session_id)?;
    let note = resolved.liveness_error.map(|source| {
        if herdr.budget_was_spent() {
            LivenessNote::BudgetSpent { candidate }
        } else {
            LivenessNote::HerdrDown { candidate, source }
        }
    });
    Ok(Named {
        name: resolved.name,
        note,
    })
}

/// [`resolve_name`] for the one hook event that may make no herdr call
/// (`UserPromptSubmit`): registry reads only, with every registered
/// same-machine session assumed live (see [`AssumeHosted`]).
fn resolve_name_registry_only(
    relay: &mut Relay,
    candidate: AgentName,
    machine: &MachineName,
    session_id: Option<&SessionId>,
) -> Result<AgentName, CollisionError> {
    resolve_collision(relay, &AssumeHosted, candidate, machine, session_id).map(|r| r.name)
}

/// The demotion that turns `old_name` into an alias of `new_name` (AGE-2018 PR
/// review, finding 2: [`relay_core::Relay::commit_identity`] never deletes a
/// registration owned by a different session/pane — it reports it instead —
/// and finding 5: it repoints every alias that already pointed at `old_name`
/// straight at `new_name`, so [`relay_core::Relay::resolve_alias`] never needs
/// more than one hop). `None` when they are the same name: nothing to do.
fn demotion(
    old_name: &AgentName,
    new_name: &AgentName,
    session_id: Option<&SessionId>,
    pane_id: &PaneId,
    machine: &MachineName,
) -> Option<Demotion> {
    (old_name != new_name).then(|| Demotion {
        old: old_name.clone(),
        new: new_name.clone(),
        session_id: session_id.cloned(),
        pane_id: Some(pane_id.clone()),
        machine: machine.clone(),
    })
}

/// Derives an identity for an interactive command (`send`, `inbox`,
/// `escalate`, `register`, `whoami`, ...) — called only once the caller has
/// already established there's no explicit `AGENT_MSG_NAME`/`--as`.
///
/// AGE-2018 PR review, finding 4: a command run directly in the agent's own
/// shell has no session transcript to check for a `/rename` the way a hook
/// does, so re-running the precedence chain here would silently ignore one.
/// Instead, this reads the pane's registration to preserve the hook's rename,
/// but accepts it only after a live Herdr snapshot confirms its session. A
/// Codex caller's `CODEX_THREAD_ID`, when present, must identify that same
/// session: inherited pane variables alone do not prove who the caller is.
/// Missing, replaced, or unreachable pane identities fail closed rather than
/// refresh or send as an old lead. The registry is not a verification source.
///
/// With no registration yet, tiers 3 and 4 can still derive a name. A Codex
/// caller must first match the pane's live session before that fallback too.
/// Both paths make no Redis writes and return `None` when no lead resolves.
///
/// # Errors
///
/// Propagates a Redis failure ([`relay_core::Error`]) from the registry
/// lookup (finding 11) — this used to be swallowed into a plain "not a
/// lead," which mislabeled a broken Redis connection as an unset
/// `AGENT_MSG_NAME`. A [`CollisionError::NoFreeName`] is
/// returned as such: the command fails rather than act under a name another
/// live lead holds.
pub fn derive(
    relay: &mut Relay,
    machine: &MachineName,
) -> Result<Option<Identity>, CollisionError> {
    if let Some(env) = HerdrEnv::from_process_env() {
        let regs =
            relay.registrations_for_pane(&PaneRef::new(machine.clone(), env.pane_id.clone()))?;
        let caller_session = std::env::var("CODEX_THREAD_ID").ok();
        if !regs.is_empty() || caller_session.is_some() {
            let herdr = real_herdr(INTERACTIVE_HERDR_BUDGET, None);
            let live_session = herdr
                .agent(&env.pane_id)
                .ok()
                .flatten()
                .and_then(|agent| agent.session_id);
            if let Some(reg) = regs.iter().find(|reg| {
                interactive_session_matches(
                    reg.session_id.as_ref(),
                    live_session.as_ref(),
                    caller_session.as_deref(),
                )
            }) {
                return Ok(Some(Identity {
                    name: reg.name.clone(),
                    source: Source::Registry,
                    location_alias: None,
                }));
            }
            if !regs.is_empty()
                || !interactive_session_matches(
                    live_session.as_ref(),
                    live_session.as_ref(),
                    caller_session.as_deref(),
                )
            {
                crate::report::line(format_args!(
                    "agent-msg: inherited Herdr pane {} has no verified identity for this \
                     session; refresh the genuine Herdr/Codex session hooks and caller context \
                     before registering (manual registration cannot repair it)",
                    env.pane_id
                ));
                return Ok(None);
            }
        }
    }
    let ChainOutcome::Winner(chain) = resolve_chain(None, INTERACTIVE_HERDR_BUDGET, None) else {
        return Ok(None);
    };
    let Named { name, note } = resolve_name(relay, chain.canonical, machine, None, None)?;
    if let Some(note) = note {
        crate::report::line(format_args!("agent-msg: {note}"));
    }
    Ok(Some(Identity {
        name,
        source: chain.source,
        location_alias: chain.location_alias,
    }))
}

/// Two absent session IDs are never proof of the same agent.
fn interactive_session_matches(
    registered: Option<&SessionId>,
    live: Option<&SessionId>,
    caller: Option<&str>,
) -> bool {
    let Some(live) = live else { return false };
    registered == Some(live) && caller.is_none_or(|caller| caller == live.as_str())
}

/// Where this pane sits in herdr, read now: what an interactive
/// `agent-msg register` records, since it has no hook's cached registration
/// to carry a place over from. `None` outside herdr, in an escalation fork, or
/// when herdr can't say (a herdr failure is not an error here: the
/// registration just keeps whatever place it already had).
#[must_use]
pub fn current_place() -> Option<Place> {
    let env = HerdrEnv::from_process_env()?;
    let herdr = real_herdr(INTERACTIVE_HERDR_BUDGET, None);
    derive_from_herdr(&herdr, &env).ok()?.place
}

/// Resolves the identity a hook call should use when there's no explicit
/// `AGENT_MSG_NAME`/`--as` — called only once the caller has already
/// established that, and only after ruling out a suppressed session (see
/// below). Per AGE-2018 (as corrected 2026-09-25):
///
/// - at `SessionStart`, always re-derives the full chain through herdr: the
///   authoritative moment a herdr rename or this pane's location can
///   change, a cross-machine collision is checked, and the session->name
///   cache is (re)written;
/// - at `UserPromptSubmit`, makes **no herdr calls at all** (finding 6):
///   only the transcript is re-scanned for a `/rename`; a hit wins outright,
///   a miss reuses the cached name. Either way the registry is refreshed
///   (`hook_run`'s caller still registers on this event) and, if the
///   canonical name just changed, the old one is demoted to an alias
///   (finding 5);
/// - at any other event (`PostToolUse`, `Stop`), first tries the
///   session->name cache (no herdr call — the fast path these need); on a
///   miss, derives once, tightly bounded by [`LAZY_HERDR_BUDGET`], and
///   caches the result for next time.
///
/// Before any of that: an escalation investigation fork or a Claude
/// `--fork-session` fork (`hook_source == Some("fork")`) never joins the
/// bus at all (finding 1) — [`HookIdentity::Suppress`], which the caller
/// remembers so every later call in the same session short-circuits from one
/// cheap Redis lookup instead of re-discovering it.
///
/// This only reads. Every herdr call it makes is bounded by what is left of
/// `budget`, and every write it implies is returned in the [`Derived`]'s
/// `commit` for the caller to apply at the end, atomically, once it knows it
/// still has time (AGE-2033).
///
/// # Errors
///
/// [`CollisionError::Relay`] for a Redis failure from the registry or
/// session-cache reads; [`CollisionError::NoFreeName`] when the derived name
/// is held by another session and no free one could be found (a holder whose
/// liveness could not be checked counts as live and comes back as a
/// [`LivenessNote`], not an error). The caller must not register: with no name the lead stays
/// unregistered (it receives no mail, since inboxes are keyed by name) until a
/// later hook resolves. It records the failure and the hook still exits 0 —
/// a lead's turn is never blocked by this.
pub fn derive_for_hook(
    relay: &mut Relay,
    machine: &MachineName,
    request: &HookRequest<'_>,
    probe: &SessionProbe,
    budget: &Budget<'_>,
) -> Result<HookIdentity, CollisionError> {
    if probe.suppressed {
        return Ok(HookIdentity::NotALead);
    }
    if request.source == Some("fork") {
        return Ok(suppress(request.session_id));
    }

    match request.event {
        Event::SessionStart => derive_at_session_start(relay, machine, request, probe, budget),
        Event::UserPromptSubmit => derive_at_user_prompt_submit(relay, machine, request, probe),
        Event::PostToolUse | Event::Stop => {
            derive_from_cache_or_lazily(relay, machine, request, probe, budget)
        }
    }
}

/// What a hook call carries that name derivation reads.
#[derive(Debug, Clone, Copy)]
pub struct HookRequest<'a> {
    /// The harness session id.
    pub session_id: Option<&'a SessionId>,
    /// The session's transcript, for a `/rename` title (Claude only).
    pub transcript_path: Option<&'a str>,
    /// `SessionStart`'s `source`, e.g. `"fork"`.
    pub source: Option<&'a str>,
    /// Which hook fired.
    pub event: Event,
}

fn suppress(session_id: Option<&SessionId>) -> HookIdentity {
    HookIdentity::Suppress {
        session_id: session_id.cloned(),
    }
}

/// `SessionStart`'s share of [`derive_for_hook`]: full re-derivation through
/// herdr.
fn derive_at_session_start(
    relay: &mut Relay,
    machine: &MachineName,
    request: &HookRequest<'_>,
    probe: &SessionProbe,
    budget: &Budget<'_>,
) -> Result<HookIdentity, CollisionError> {
    let session_id = request.session_id;
    let previous_cached = probe.name.clone();
    match resolve_chain(
        request.transcript_path,
        SESSION_START_HERDR_BUDGET,
        Some(budget),
    ) {
        ChainOutcome::Suppressed => Ok(suppress(session_id)),
        ChainOutcome::None => Ok(HookIdentity::NotALead),
        ChainOutcome::HerdrFailed { budget_spent } => {
            Ok(HookIdentity::HerdrFailed { budget_spent })
        }
        ChainOutcome::Winner(chain) => {
            let Named { name, note } =
                resolve_name(relay, chain.canonical, machine, session_id, Some(budget))?;
            let mut commit = IdentityCommit::default();
            if let Some(alias) = &chain.location_alias {
                commit.demotions.extend(demotion(
                    alias,
                    &name,
                    session_id,
                    &chain.pane_id,
                    machine,
                ));
            }
            if let Some(prev) = &previous_cached {
                commit
                    .demotions
                    .extend(demotion(prev, &name, session_id, &chain.pane_id, machine));
            }
            commit.session_name = session_id.map(|sid| (sid.clone(), name.clone()));
            Ok(HookIdentity::Lead(Box::new(Derived {
                name,
                place: chain.place,
                commit,
                note,
            })))
        }
    }
}

/// `UserPromptSubmit`'s share of [`derive_for_hook`] (finding 6): no herdr
/// calls — only the transcript (a local file read) is re-scanned, and the
/// herdr-derived tiers are trusted to be unchanged since the last full
/// derivation.
fn derive_at_user_prompt_submit(
    relay: &mut Relay,
    machine: &MachineName,
    request: &HookRequest<'_>,
    probe: &SessionProbe,
) -> Result<HookIdentity, CollisionError> {
    let session_id = request.session_id;
    let cached = probe.name.clone();
    let title_name = match request
        .transcript_path
        .and_then(custom_title_from_transcript)
        .map(|t| relay_core::naming::slugify(&t))
    {
        // The title names this session already: nothing to resolve.
        Some(title) if cached.as_ref() == Some(&title) => Some(title),
        // A `/rename` may land on a name another lead holds — resolve it like
        // SessionStart does, from registry reads alone (no herdr call here).
        Some(title) => match resolve_name_registry_only(relay, title, machine, session_id) {
            Ok(name) => Some(name),
            // No free variant: keep the name this session already has rather
            // than fail the prompt (nothing is registered or overwritten).
            Err(CollisionError::NoFreeName { .. }) if cached.is_some() => None,
            Err(e) => return Err(e),
        },
        None => None,
    };
    let Some(name) = title_name.or_else(|| cached.clone()) else {
        // Nothing cached (this session's SessionStart was itself suppressed
        // or never a lead) and no title either: stay silent.
        return Ok(HookIdentity::NotALead);
    };
    let mut commit = IdentityCommit::default();
    if let Some(prev) = &cached
        && let Some(sid) = session_id
        && let Some(pane) = HerdrEnv::pane_id_from_process_env()
    {
        commit
            .demotions
            .extend(demotion(prev, &name, Some(sid), &pane, machine));
    }
    commit.session_name = session_id.map(|sid| (sid.clone(), name.clone()));
    Ok(HookIdentity::Lead(Box::new(Derived {
        name,
        place: None,
        commit,
        note: None,
    })))
}

/// `PostToolUse`/`Stop`'s share of [`derive_for_hook`]: cache first, and a
/// tightly-bounded lazy full derivation on a cache miss.
fn derive_from_cache_or_lazily(
    relay: &mut Relay,
    machine: &MachineName,
    request: &HookRequest<'_>,
    probe: &SessionProbe,
    budget: &Budget<'_>,
) -> Result<HookIdentity, CollisionError> {
    let session_id = request.session_id;
    if let Some(cached) = &probe.name {
        return Ok(HookIdentity::Lead(Box::new(Derived::cached(
            cached.clone(),
        ))));
    }
    if probe.unresolved {
        // An earlier derive found no free name and recorded it: stay quiet
        // until the mark expires (or `SessionStart` derives again).
        return Ok(HookIdentity::NotALead);
    }
    match resolve_chain(request.transcript_path, LAZY_HERDR_BUDGET, Some(budget)) {
        ChainOutcome::Suppressed => Ok(suppress(session_id)),
        ChainOutcome::None => Ok(HookIdentity::NotALead),
        ChainOutcome::HerdrFailed { budget_spent } => {
            Ok(HookIdentity::HerdrFailed { budget_spent })
        }
        ChainOutcome::Winner(chain) => {
            let Named { name, note } =
                resolve_name(relay, chain.canonical, machine, session_id, Some(budget))?;
            let mut commit = IdentityCommit::default();
            if let Some(alias) = &chain.location_alias {
                commit.demotions.extend(demotion(
                    alias,
                    &name,
                    session_id,
                    &chain.pane_id,
                    machine,
                ));
            }
            commit.session_name = session_id.map(|sid| (sid.clone(), name.clone()));
            Ok(HookIdentity::Lead(Box::new(Derived {
                name,
                place: chain.place,
                commit,
                note,
            })))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, io::Write, rc::Rc};

    use super::*;

    use crate::budget::{Clock, testing::FakeClock};

    /// A herdr that answers nothing, for the timeouts it was built with.
    struct NoHerdr;

    impl LocationHerdr for NoHerdr {
        fn workspace_list(&self) -> Result<Vec<WorkspaceEntry>, ProcessError> {
            Ok(Vec::new())
        }
        fn tab_list(&self, _: &WorkspaceId) -> Result<Vec<TabEntry>, ProcessError> {
            Ok(Vec::new())
        }
        fn pane_list(&self, _: &WorkspaceId) -> Result<Vec<PaneEntry>, ProcessError> {
            Ok(Vec::new())
        }
        fn agent(&self, _: &PaneId) -> Result<Option<relay_ops::naming::HerdrAgent>, ProcessError> {
            Ok(None)
        }
    }

    /// Each Herdr call can use the shared remaining deadline, and no call
    /// starts after that deadline is spent.
    #[test]
    fn herdr_calls_are_bounded_by_the_remaining_deadline() {
        let clock = FakeClock::default();
        // SessionStart's real deadline: the herdr budget plus 300ms.
        let budget = Budget::new(
            &clock,
            SESSION_START_HERDR_BUDGET + Duration::from_millis(300),
        );
        let timeouts = RefCell::new(Vec::new());
        let herdr = BoundedHerdr::new(SESSION_START_HERDR_BUDGET, Some(&budget), |t| {
            timeouts.borrow_mut().push(t);
            clock.advance(Duration::from_millis(100));
            NoHerdr
        });
        // Each call can use the full remaining budget, but quick calls leave
        // time for the rest of the identity snapshot.
        herdr.workspace_list().unwrap();
        herdr.tab_list(&"w".parse().unwrap()).unwrap();
        herdr.pane_list(&"w".parse().unwrap()).unwrap();
        herdr.agent(&"p".parse().unwrap()).unwrap();
        assert_eq!(
            *timeouts.borrow(),
            [
                SESSION_START_HERDR_BUDGET,
                SESSION_START_HERDR_BUDGET,
                SESSION_START_HERDR_BUDGET,
                SESSION_START_HERDR_BUDGET,
            ]
        );
        assert!(clock.elapsed() <= SESSION_START_HERDR_BUDGET + Duration::from_millis(300));
        assert_eq!(herdr.agent(&"p".parse().unwrap()).unwrap(), None);
        assert_eq!(
            timeouts.borrow().last(),
            Some(&(SESSION_START_HERDR_BUDGET - Duration::from_millis(100)))
        );
        // After the full overall deadline, another call cannot start.
        clock.advance(SESSION_START_HERDR_BUDGET);
        let err = herdr.agent(&"p".parse().unwrap()).unwrap_err();
        assert!(matches!(err, ProcessError::Timeout { .. }), "{err:?}");
        assert_eq!(timeouts.borrow().len(), 5);
    }

    /// A herdr that fails every call.
    struct DeadHerdr;

    fn gone() -> ProcessError {
        ProcessError::Spawn {
            program: "herdr".into(),
            reason: "gone".into(),
        }
    }

    impl LocationHerdr for DeadHerdr {
        fn workspace_list(&self) -> Result<Vec<WorkspaceEntry>, ProcessError> {
            Err(gone())
        }
        fn tab_list(&self, _: &WorkspaceId) -> Result<Vec<TabEntry>, ProcessError> {
            Err(gone())
        }
        fn pane_list(&self, _: &WorkspaceId) -> Result<Vec<PaneEntry>, ProcessError> {
            Err(gone())
        }
        fn agent(&self, _: &PaneId) -> Result<Option<relay_ops::naming::HerdrAgent>, ProcessError> {
            Err(gone())
        }
    }

    fn herdr_env() -> HerdrEnv {
        HerdrEnv {
            pane_id: "w:p1".parse().unwrap(),
            tab_id: "w:t1".parse().unwrap(),
            workspace_id: "w".parse().unwrap(),
        }
    }

    /// Tracing: TC-030-10
    /// ACs: FR-030-AC-10
    ///
    /// AGE-2059: a pane whose herdr fails and names nothing else is a failure
    /// the hook records, not a non-lead it passes over in silence. With the
    /// budget spent before herdr is asked, the outcome says so.
    #[test]
    fn a_failing_herdr_that_names_nothing_is_a_failure_not_a_non_lead() {
        let herdr = BoundedHerdr::new(Duration::from_millis(1000), None, |_| DeadHerdr);
        assert!(matches!(
            chain_from(&herdr, &herdr_env(), None),
            ChainOutcome::HerdrFailed {
                budget_spent: false
            }
        ));

        let clock = FakeClock::default();
        let budget = Budget::new(&clock, Duration::from_millis(50));
        clock.advance(Duration::from_millis(50));
        let herdr = BoundedHerdr::new(Duration::from_millis(1000), Some(&budget), |_| NoHerdr);
        assert!(matches!(
            chain_from(&herdr, &herdr_env(), None),
            ChainOutcome::HerdrFailed { budget_spent: true }
        ));
    }

    /// A herdr that answers, and knows nothing of the pane, is still a non-lead.
    #[test]
    fn a_herdr_that_answers_and_names_nothing_is_a_non_lead() {
        let herdr = BoundedHerdr::new(Duration::from_millis(1000), None, |_| NoHerdr);
        assert!(matches!(
            chain_from(&herdr, &herdr_env(), None),
            ChainOutcome::None
        ));
    }

    /// With no budget (an interactive command) only the per-call cap applies.
    #[test]
    fn without_a_budget_only_the_cap_bounds_a_herdr_call() {
        let timeouts = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&timeouts);
        let herdr = BoundedHerdr::new(Duration::from_millis(1000), None, move |t| {
            seen.borrow_mut().push(t);
            NoHerdr
        });
        herdr.workspace_list().unwrap();
        assert_eq!(*timeouts.borrow(), [Duration::from_millis(1000)]);
    }

    fn write_transcript(lines: &[String]) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        for line in lines {
            writeln!(file, "{line}").unwrap();
        }
        file.flush().unwrap();
        file
    }

    fn custom_title_line(title: &str) -> String {
        format!(r#"{{"type":"custom-title","customTitle":"{title}","sessionId":"s1"}}"#)
    }

    #[test]
    fn finds_the_only_custom_title_line() {
        let file = write_transcript(&[
            r#"{"type":"user","message":{"role":"user","content":"hi"}}"#.to_owned(),
            custom_title_line("agent-d"),
            r#"{"type":"assistant","message":{"role":"assistant","content":"ok"}}"#.to_owned(),
        ]);
        let title = custom_title_from_transcript(file.path().to_str().unwrap());
        assert_eq!(title.as_deref(), Some("agent-d"));
    }

    #[test]
    fn the_last_custom_title_line_wins() {
        let file = write_transcript(&[
            custom_title_line("first-name"),
            r#"{"type":"user","message":{"role":"user","content":"more"}}"#.to_owned(),
            custom_title_line("second-name"),
        ]);
        let title = custom_title_from_transcript(file.path().to_str().unwrap());
        assert_eq!(title.as_deref(), Some("second-name"));
    }

    #[test]
    fn no_custom_title_line_at_all_is_none() {
        let file = write_transcript(&[
            r#"{"type":"user","message":{"role":"user","content":"hi"}}"#.to_owned(),
        ]);
        assert_eq!(
            custom_title_from_transcript(file.path().to_str().unwrap()),
            None
        );
    }

    #[test]
    fn missing_file_is_none_not_a_panic() {
        assert_eq!(
            custom_title_from_transcript("/definitely/not/a/real/transcript.jsonl"),
            None
        );
    }

    /// A transcript spanning several read chunks (the scan reads 64KB at a
    /// time from the end), with the winning `custom-title` line near the
    /// very start of the file: proves the backward scan walks all the way
    /// back instead of giving up after the first chunk.
    #[test]
    fn finds_the_last_custom_title_line_in_a_multi_chunk_file() {
        let mut lines = vec![custom_title_line("early-rename")];
        // Padding well past one 64KB read chunk.
        for i in 0..3000 {
            lines.push(format!(
                r#"{{"type":"user","message":{{"role":"user","content":"padding line {i} {}"}}}}"#,
                "x".repeat(40)
            ));
        }
        let file = write_transcript(&lines);
        let title = custom_title_from_transcript(file.path().to_str().unwrap());
        assert_eq!(title.as_deref(), Some("early-rename"));
    }

    /// AGE-2018 PR review, finding 9: a transcript ending in one single huge
    /// line with no newline anywhere must still scan in work proportional to
    /// the file size, not the O(n²) blowup an earlier version had — it
    /// re-built and re-split a string holding the *entire* not-yet-
    /// terminated tail on every 64KB chunk. The real `custom-title` line sits
    /// *before* this line, so finding it requires scanning across the huge
    /// line first. (Kept inside [`MAX_TRANSCRIPT_SCAN_BYTES`] so the bound
    /// does not hide it.)
    #[test]
    fn finds_a_custom_title_line_that_precedes_one_huge_single_line_with_no_newlines() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(file, "{}", custom_title_line("found-before-one-big-line")).unwrap();
        // No trailing newline: this is the file's last, unterminated line.
        write!(file, "{}", "x".repeat(4 * 1024 * 1024)).unwrap();
        file.flush().unwrap();
        let title = custom_title_from_transcript(file.path().to_str().unwrap());
        assert_eq!(title.as_deref(), Some("found-before-one-big-line"));
    }

    /// AGE-2026: the scan is bounded. A title older than the window is not
    /// found, however large the file, and one inside it still is.
    #[test]
    fn the_scan_stops_at_the_byte_bound() {
        let padding = format!(
            r#"{{"type":"user","message":{{"role":"user","content":"{}"}}}}"#,
            "p".repeat(1000)
        );
        let lines_past_bound =
            usize::try_from(MAX_TRANSCRIPT_SCAN_BYTES).unwrap() / padding.len() + 10;

        // Title first, then more than the bound of padding: out of the window.
        let mut too_old = tempfile::NamedTempFile::new().unwrap();
        writeln!(too_old, "{}", custom_title_line("too-old")).unwrap();
        for _ in 0..lines_past_bound {
            writeln!(too_old, "{padding}").unwrap();
        }
        too_old.flush().unwrap();
        assert_eq!(
            custom_title_from_transcript(too_old.path().to_str().unwrap()),
            None
        );

        // The same padding with the title after it: found.
        let mut recent = tempfile::NamedTempFile::new().unwrap();
        for _ in 0..lines_past_bound {
            writeln!(recent, "{padding}").unwrap();
        }
        writeln!(recent, "{}", custom_title_line("recent")).unwrap();
        recent.flush().unwrap();
        assert_eq!(
            custom_title_from_transcript(recent.path().to_str().unwrap()).as_deref(),
            Some("recent")
        );
    }

    #[test]
    fn a_line_merely_mentioning_custom_title_in_prose_is_not_mistaken_for_one() {
        let file = write_transcript(&[
            r#"{"type":"user","message":{"role":"user","content":"what does custom-title do?"}}"#
                .to_owned(),
        ]);
        assert_eq!(
            custom_title_from_transcript(file.path().to_str().unwrap()),
            None
        );
    }

    /// `whoami --json` pins both shapes' field lists.
    #[test]
    fn whoami_json_pins_both_shapes() {
        let id = Identity {
            name: "qsl-lead".parse().unwrap(),
            source: Source::Explicit,
            location_alias: Some("qsl".parse().unwrap()),
        };
        assert_eq!(
            serde_json::to_value(WhoamiJson::lead(&id, Some("node-a:qsl"))).unwrap(),
            serde_json::json!({
                "name": "qsl-lead",
                "source": "explicit",
                "location_alias": "qsl",
                "address": "node-a:qsl",
            })
        );
        assert_eq!(
            serde_json::to_value(WhoamiJson::not_a_lead()).unwrap(),
            serde_json::json!({"lead": false})
        );
    }
}
