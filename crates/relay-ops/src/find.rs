//! Live lead discovery joins session queues against Herdr locations.
//! Public identities are server:workspace:tab; display names are metadata.
//!
//! [`join`] and [`matches_query`] are pure — no herdr or Redis calls —
//! so the join logic is unit-testable over fake data with no process or
//! network involved. `agent-msg`'s `find` command gathers the inputs
//! (shelling out to herdr per machine, reading the registry) and calls into
//! this module.

use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
    time::{Duration, Instant},
};

pub use herdr_cli::ProcessError;
use herdr_cli::ProcessRunner;
use relay_core::{
    Address, AgentName, HerdrPlaceIds, MachineName, PaneId, PaneRef, Place, Registration,
    SessionId, TabId, WorkspaceId,
};
use serde::Deserialize;

pub use crate::lenient::Lenient;

/// One live agent herdr reports, on one machine, before it's joined against
/// the bus registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HerdrAgent {
    /// Harness kind reported by local Herdr, or unknown if unavailable.
    pub kind: String,
    /// The pane hosting it, e.g. `wE:p1`.
    pub pane_id: PaneId,
    /// The tab it's in.
    pub tab_id: TabId,
    /// The workspace it's in.
    pub workspace_id: WorkspaceId,
    /// herdr's own lifecycle status, e.g. `idle`, `working`, `blocked`,
    /// `done`, `unknown` — printed verbatim, not reinterpreted.
    pub status: String,
    /// A best-effort session/agent name herdr shows for this pane
    /// (`terminal_title_stripped` — see
    /// the live agent snapshot's own doc comment on
    /// why this is an approximation).
    pub session_name: Option<String>,
    /// The harness session id herdr reports for the pane (`agent_session.value`),
    /// when the harness has reported one and it is a session id. The durable key
    /// a registration follows across a pane move (FR-043).
    pub session_id: Option<SessionId>,
}

/// One workspace's id -> label, as `herdr workspace list` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct HerdrWorkspace {
    /// The workspace's opaque id.
    pub workspace_id: WorkspaceId,
    /// Its label, e.g. `QSL`.
    pub label: String,
    /// How many tabs the workspace has; `0` when herdr did not say. The tab
    /// segment of an address is left out only when this is exactly `1`.
    #[serde(default)]
    pub tab_count: u32,
}

/// One tab's id -> label, as `herdr tab list` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct HerdrTab {
    /// The tab's opaque id.
    pub tab_id: TabId,
    /// Its label (or herdr's own display number, rendered as a string, for
    /// an unlabelled tab).
    pub label: String,
}

/// Everything gathered from one machine (the local one, or a saved herdr
/// machine profile): its live agents, and the workspace/tab labels needed
/// to describe where each one sits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineView {
    /// This machine's name, normalized the same way the bus registry's
    /// `machine` field is (see [`relay_core::MachineName`]).
    pub machine: MachineName,
    /// Every live agent herdr reports on this machine.
    pub agents: Vec<HerdrAgent>,
    /// Workspace labels for every workspace referenced by `agents`.
    pub workspaces: Vec<HerdrWorkspace>,
    /// Tab labels for every tab referenced by `agents`.
    pub tabs: Vec<HerdrTab>,
    /// List entries herdr returned that this binary could not decode (an id
    /// outside its type's rule, a changed field), across the three lists. A
    /// dropped agent is a missing row, a dropped tab or workspace a missing
    /// place: never a wrong one. The caller reports the count.
    pub dropped: usize,
}

/// One joined row of `agent-msg find`'s output: a live herdr agent, plus
/// whatever the bus registry knows about it.
#[derive(Debug, Clone)]
pub struct FoundAgent {
    /// The lead's canonical bus name, if this pane is registered.
    pub bus_name: Option<AgentName>,
    /// The machine this agent runs on.
    pub machine: MachineName,
    /// Its `server:workspace[:tab[:pane]]` address, when the workspace label is known.
    pub address: Option<Address>,
    /// The workspace's label, if known.
    pub workspace_label: Option<String>,
    /// The tab's label, if known.
    pub tab_label: Option<String>,
    /// The pane hosting it.
    pub pane_id: PaneId,
    /// The tab it is in.
    pub tab_id: TabId,
    /// The workspace it is in.
    pub workspace_id: WorkspaceId,
    /// The harness session id herdr reports for the pane, when it reports one
    /// (see [`HerdrAgent::session_id`]).
    pub session_id: Option<SessionId>,
    /// The best-effort session/agent name herdr reports (see
    /// [`HerdrAgent::session_name`]).
    pub session_name: Option<String>,
    /// herdr's own lifecycle status.
    pub herdr_status: String,
    /// Whether this pane is registered on the bus at all.
    pub on_bus: bool,
}

/// Joins every machine's live herdr agents against the bus registry
/// using session-bound registrations.
///
/// Iterates live herdr agents, not bus registrations: a registration whose
/// pane isn't live on its machine right now doesn't produce a row (its
/// message still queues and is delivered at that lead's next
/// `SessionStart` — this command is for finding someone to message *now*).
/// A live herdr agent with no matching registration still produces a row,
/// `on_bus: false` and `bus_name: None`, so a sender can see why a message
/// to it wouldn't be picked up.
#[must_use]
pub fn join(machines: &[MachineView], registrations: &[Registration]) -> Vec<FoundAgent> {
    let mut rows = Vec::new();
    for view in machines {
        let places = place_map(view);

        for agent in &view.agents {
            let pane = PaneRef::new(view.machine.clone(), agent.pane_id.clone());
            let on_pane: Vec<&Registration> =
                registrations.iter().filter(|r| r.is_on(&pane)).collect();
            let reg = match agent.session_id.as_ref() {
                Some(session) => {
                    let matching: Vec<_> = on_pane
                        .iter()
                        .copied()
                        .filter(|r| r.session_id.as_ref() == Some(session))
                        .collect();
                    (matching.len() == 1).then(|| matching[0])
                }
                None => None,
            };
            let bus_name = reg.map(|r| r.name.clone());
            let place = places.get(&agent.pane_id);
            let address = agent
                .session_id
                .as_ref()
                .and(place)
                .map(|p| p.tab_address(&view.machine));
            let workspace_label = workspace_label(view, agent);
            let tab_label = tab_label(view, agent);

            rows.push(FoundAgent {
                bus_name,
                address,
                machine: view.machine.clone(),
                workspace_label: workspace_label.map(str::to_owned),
                tab_label: tab_label.map(str::to_owned),
                pane_id: agent.pane_id.clone(),
                tab_id: agent.tab_id.clone(),
                workspace_id: agent.workspace_id.clone(),
                session_id: agent.session_id.clone(),
                session_name: agent.session_name.clone(),
                herdr_status: agent.status.clone(),
                on_bus: reg.is_some(),
            });
        }
    }
    rows
}

/// The workspace label herdr shows for `agent`, when known.
fn workspace_label<'a>(view: &'a MachineView, agent: &HerdrAgent) -> Option<&'a str> {
    view.workspaces
        .iter()
        .find(|w| w.workspace_id == agent.workspace_id)
        .map(|w| w.label.as_str())
}

/// The tab label herdr shows for `agent`, when known.
fn tab_label<'a>(view: &'a MachineView, agent: &HerdrAgent) -> Option<&'a str> {
    view.tabs
        .iter()
        .find(|t| t.tab_id == agent.tab_id)
        .map(|t| t.label.as_str())
}

fn place_map(view: &MachineView) -> HashMap<&PaneId, Place> {
    view.agents
        .iter()
        .filter_map(|agent| {
            let session_id = agent.session_id.clone()?;
            let workspace = view
                .workspaces
                .iter()
                .find(|w| w.workspace_id == agent.workspace_id)?;
            let tab = tab_label(view, agent)?;
            Some((
                &agent.pane_id,
                Place {
                    workspace: workspace.label.clone(),
                    tab: tab.to_owned(),
                    herdr_ids: Some(HerdrPlaceIds {
                        workspace: workspace.workspace_id.clone(),
                        tab: agent.tab_id.clone(),
                        pane: agent.pane_id.clone(),
                        session_id,
                        verified_at_ms: herdr_verified_at_ms(),
                    }),
                },
            ))
        })
        .collect()
}

fn herdr_verified_at_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Where every live agent on `view`'s machine sits, keyed by pane id — the
/// same [`Place`] a hook records at `SessionStart`, read from herdr's state
/// now. `relay-watcher` compares this with what each lead registered, so a
/// tab closing (herdr renumbers unlabelled tabs) or a workspace or tab rename
/// does not leave a stale `server:workspace[:tab[:pane]]` address resolving to
/// the wrong lead. A pane without an exact session ID or stable location labels
/// is left out: better no update than a wrong one. Tab counts do not establish
/// identity and are not part of this decision.
#[must_use]
pub fn places(view: &MachineView) -> HashMap<PaneId, Place> {
    place_map(view)
        .into_iter()
        .map(|(pane, place)| (pane.clone(), place))
        .collect()
}

/// Every pane herdr lists an agent in, with the harness session it reports:
/// `None` for an agent whose harness has not reported one.
#[must_use]
pub fn sessions(view: &MachineView) -> HashMap<PaneId, Option<SessionId>> {
    view.agents
        .iter()
        .map(|a| (a.pane_id.clone(), a.session_id.clone()))
        .collect()
}

/// Whether `row` matches `query`, case-insensitively, against any of: its
/// address, workspace or tab label, session name, or pane id.
/// An empty `query` matches everything.
#[must_use]
pub fn matches_query(row: &FoundAgent, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let query = query.to_lowercase();
    let address = row.address.as_ref().map(ToString::to_string);
    let fields: [Option<&str>; 6] = [
        row.bus_name.as_ref().map(AgentName::as_str),
        row.workspace_label.as_deref(),
        row.tab_label.as_deref(),
        row.session_name.as_deref(),
        Some(row.pane_id.as_str()),
        address.as_deref(),
    ];
    fields
        .into_iter()
        .flatten()
        .any(|f| f.to_lowercase().contains(&query))
}

// ============================================================================
// Live gathering: shells out to herdr, per machine. Everything above this
// point is pure and unit-tested with no process involved (see this
// module's own tests); `gather_machine_view` is the seam between the two.
// ============================================================================

/// One saved herdr machine profile, as `herdr machine list --json` reports
/// it. Unlike every other herdr listing this crate decodes, this one is a
/// bare JSON array with no `{"result": ...}` envelope — decoded directly
/// with `serde_json`, not [`herdr_cli::decode`].
#[derive(Debug, Clone, Deserialize)]
pub struct MachineProfile {
    /// The profile's label, e.g. `hub` — what `--machine <label>` takes.
    pub label: String,
    /// Whether the profile is enabled. A disabled profile's remote session
    /// keeps running (per `herdr machine`'s own help text) but `find` skips
    /// querying it, same as herdr's own commands would refuse `--machine`
    /// against a disabled profile.
    pub enabled: bool,
    /// Exact SSH target saved by Herdr; never inferred from a lead name.
    #[serde(default)]
    pub target: Option<String>,
    /// Exact remote Herdr session saved by Herdr.
    #[serde(default)]
    pub session: Option<String>,
}

/// One entry from `herdr agent list`'s `agents` array — the fields `find`
/// needs; herdr's own payload carries more (see e.g. `relay_watcher::herdr`
/// for a different slice of the same command).
#[derive(Debug, Clone, Deserialize)]
pub struct AgentListEntry {
    /// Harness executable kind reported by Herdr.
    #[serde(default)]
    pub agent: Option<String>,
    /// The pane hosting it.
    pub pane_id: PaneId,
    /// The tab it's in.
    pub tab_id: TabId,
    /// The workspace it's in.
    pub workspace_id: WorkspaceId,
    /// herdr's own lifecycle status.
    pub agent_status: String,
    /// A best-effort session/agent name (see [`HerdrAgent::session_name`]).
    #[serde(default)]
    pub terminal_title_stripped: Option<String>,
    /// The harness session herdr reports for the pane, once the harness has said.
    #[serde(default)]
    pub agent_session: Option<ListedSession>,
}

/// herdr's `agent_session` member (in `agent list` and `agent get`): what the
/// harness told herdr about its session. herdr 0.9.1's schema gives `kind` as
/// `id` or `path`; only an `id` is a harness session id.
#[derive(Debug, Clone, Deserialize)]
pub struct ListedSession {
    /// `id` or `path` (herdr's `AgentSessionRefKind`). Absent, or any other value
    /// a newer herdr adds, is not an id.
    #[serde(default)]
    pub kind: Option<String>,
    /// The reference, as text.
    pub value: String,
}

impl ListedSession {
    /// The session id this reports: only `kind == "id"` with a value that is a
    /// [`SessionId`]. A path (a transcript file) or an unknown kind is "no session
    /// known", never a different session: it can be neither the registered one nor
    /// proof that the pane was replaced.
    #[must_use]
    pub fn session_id(&self) -> Option<SessionId> {
        (self.kind.as_deref() == Some("id"))
            .then(|| self.value.parse().ok())
            .flatten()
    }
}

/// The herdr calls `find`'s live gathering needs, each optionally scoped to
/// a saved machine (`None` = local, no `--machine` flag).
pub trait FindHerdr {
    /// `herdr [--machine <m>] agent list`.
    fn agent_list(
        &self,
        machine: Option<&str>,
        timeout: Duration,
    ) -> Result<Lenient<AgentListEntry>, ProcessError>;
    /// `herdr [--machine <m>] workspace list`.
    fn workspace_list(
        &self,
        machine: Option<&str>,
        timeout: Duration,
    ) -> Result<Lenient<HerdrWorkspace>, ProcessError>;
    /// `herdr [--machine <m>] tab list` — every tab on the machine, across
    /// every workspace, in one call. herdr accepts `tab list` with no
    /// `--workspace` filter and returns every tab regardless (confirmed
    /// live on node-a); AGE-2018 PR review, finding 3: an earlier version
    /// called this once *per distinct workspace* a machine's live agents
    /// were in, which multiplied the SSH round trips `find` needed against
    /// a remote machine like hub and was the reason it kept timing out
    /// there.
    fn tab_list(
        &self,
        machine: Option<&str>,
        timeout: Duration,
    ) -> Result<Lenient<HerdrTab>, ProcessError>;
    /// `herdr machine list --json`: every saved machine profile, enabled or
    /// not (see [`MachineProfile::enabled`]).
    fn machine_list(&self, timeout: Duration) -> Result<Vec<MachineProfile>, ProcessError>;
}

/// Shells out to the real `herdr` binary on `PATH`.
pub struct RealFindHerdr<R: ProcessRunner = herdr_cli::RealProcessRunner> {
    runner: R,
    profiles: Mutex<Option<Vec<MachineProfile>>>,
}

impl RealFindHerdr<herdr_cli::RealProcessRunner> {
    /// The production `RealFindHerdr`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            runner: herdr_cli::RealProcessRunner,
            profiles: Mutex::new(None),
        }
    }
}

impl<R: ProcessRunner> RealFindHerdr<R> {
    /// A `RealFindHerdr` that runs its commands through `runner`: the seam a test
    /// uses to replay herdr's real output without a herdr.
    #[must_use]
    pub fn with_runner(runner: R) -> Self {
        Self {
            runner,
            profiles: Mutex::new(None),
        }
    }
}

impl<R: ProcessRunner> RealFindHerdr<R> {
    fn listing<T: serde::de::DeserializeOwned>(
        &self,
        machine: Option<&str>,
        args: &[&str],
        timeout: Duration,
    ) -> Result<T, ProcessError> {
        let Some(machine) = machine else {
            return herdr_cli::call(&self.runner, "herdr", args, timeout);
        };
        let cached = self
            .profiles
            .lock()
            .map_err(|_| ProcessError::BadOutput {
                program: "herdr".into(),
                reason: "profile cache lock poisoned".into(),
            })?
            .clone();
        let profiles = match cached {
            Some(profiles) => profiles,
            None => self.machine_list(timeout)?,
        };
        let matches: Vec<_> = profiles
            .iter()
            .filter(|p| p.enabled && p.label.eq_ignore_ascii_case(machine))
            .collect();
        let [profile] = matches.as_slice() else {
            return Err(ProcessError::BadOutput {
                program: "herdr".into(),
                reason: format!("expected one enabled machine profile for {machine}"),
            });
        };
        let target = profile
            .target
            .as_deref()
            .filter(|s| !s.is_empty() && !s.starts_with('-') && !s.chars().any(char::is_control));
        let session = profile
            .session
            .as_deref()
            .filter(|s| !s.is_empty() && !s.chars().any(char::is_control));
        let (Some(target), Some(session)) = (target, session) else {
            return Err(ProcessError::BadOutput {
                program: "herdr".into(),
                reason: format!("profile {machine} must specify its SSH target and Herdr session"),
            });
        };
        // Invoke the installed CLI in that exact saved session. Herdr's machine
        // bootstrap currently delays completed reads; no pane or caller env is
        // forwarded across machines, and no transport is guessed or retried.
        let quote = |s: &str| format!("'{}'", s.replace('\'', "'\"'\"'"));
        let command = format!(
            "HERDR_ENV=1 PATH=\"$HOME/.local/bin:$HOME/.cargo/bin:$PATH\" herdr --session {} {}",
            quote(session),
            args.iter()
                .map(|arg| quote(arg))
                .collect::<Vec<_>>()
                .join(" ")
        );
        herdr_cli::call(
            &self.runner,
            "ssh",
            &[
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=10",
                "--",
                target,
                &command,
            ],
            timeout,
        )
    }
}

impl Default for RealFindHerdr<herdr_cli::RealProcessRunner> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R: ProcessRunner> FindHerdr for RealFindHerdr<R> {
    fn agent_list(
        &self,
        machine: Option<&str>,
        timeout: Duration,
    ) -> Result<Lenient<AgentListEntry>, ProcessError> {
        #[derive(Deserialize)]
        struct Listing {
            agents: Lenient<AgentListEntry>,
        }
        let listing: Listing = self.listing(machine, &["agent", "list"], timeout)?;
        Ok(listing.agents)
    }

    fn workspace_list(
        &self,
        machine: Option<&str>,
        timeout: Duration,
    ) -> Result<Lenient<HerdrWorkspace>, ProcessError> {
        #[derive(Deserialize)]
        struct Listing {
            workspaces: Lenient<HerdrWorkspace>,
        }
        let listing: Listing = self.listing(machine, &["workspace", "list"], timeout)?;
        Ok(listing.workspaces)
    }

    fn tab_list(
        &self,
        machine: Option<&str>,
        timeout: Duration,
    ) -> Result<Lenient<HerdrTab>, ProcessError> {
        #[derive(Deserialize)]
        struct Listing {
            tabs: Lenient<HerdrTab>,
        }
        let listing: Listing = self.listing(machine, &["tab", "list"], timeout)?;
        Ok(listing.tabs)
    }

    fn machine_list(&self, timeout: Duration) -> Result<Vec<MachineProfile>, ProcessError> {
        let out = self
            .runner
            .run("herdr", &["machine", "list", "--json"], timeout)?;
        if !out.success {
            return Err(ProcessError::UnexpectedExit {
                program: "herdr".to_owned(),
                status: out.status_code,
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            });
        }
        let profiles: Vec<MachineProfile> =
            serde_json::from_slice(&out.stdout).map_err(|e| ProcessError::BadOutput {
                program: "herdr".to_owned(),
                reason: e.to_string(),
            })?;
        *self.profiles.lock().map_err(|_| ProcessError::BadOutput {
            program: "herdr".into(),
            reason: "profile cache lock poisoned".into(),
        })? = Some(profiles.clone());
        Ok(profiles)
    }
}

/// Gathers one machine's [`MachineView`]: its live agents, and the
/// workspace/tab labels for whichever workspaces and tabs those agents are
/// actually in (never every workspace/tab on the machine, even though both
/// listings themselves come back unfiltered — filtering happens locally) —
/// bounded to at most one `agent list` call, one `workspace list` call, and
/// one `tab list` call, full stop, regardless of how many workspaces or
/// tabs the machine has (AGE-2018 PR review, finding 3: a previous version
/// made one extra `tab list` call per distinct workspace referenced, which
/// multiplied the SSH round trips against a remote machine and was why
/// `find` kept timing out against hub).
///
/// # Errors
///
/// Propagates [`ProcessError`] from any call. The caller (`agent-msg find`)
/// treats a failure here as "skip this machine," not as failing the whole
/// command.
pub fn gather_machine_view(
    herdr: &impl FindHerdr,
    machine_flag: Option<&str>,
    machine_name: &MachineName,
    timeout: Duration,
) -> Result<MachineView, ProcessError> {
    let started = Instant::now();
    let remaining = || {
        let left = timeout.saturating_sub(started.elapsed());
        if left.is_zero() {
            Err(ProcessError::Timeout {
                program: "herdr".into(),
                secs: timeout.as_secs(),
            })
        } else {
            Ok(left)
        }
    };
    let listed = herdr.agent_list(machine_flag, remaining()?)?;
    let mut dropped = listed.dropped;
    let entries = listed.entries;
    let tab_ids: HashSet<&TabId> = entries.iter().map(|a| &a.tab_id).collect();
    let workspace_ids: HashSet<&WorkspaceId> = entries.iter().map(|a| &a.workspace_id).collect();

    let listed = herdr.workspace_list(machine_flag, remaining()?)?;
    dropped += listed.dropped;
    let workspaces: Vec<HerdrWorkspace> = listed
        .entries
        .into_iter()
        .filter(|w| workspace_ids.contains(&w.workspace_id))
        .collect();

    let listed = herdr.tab_list(machine_flag, remaining()?)?;
    dropped += listed.dropped;
    let tabs: Vec<HerdrTab> = listed
        .entries
        .into_iter()
        .filter(|t| tab_ids.contains(&t.tab_id))
        .collect();

    let agents = entries
        .into_iter()
        .map(|a| HerdrAgent {
            kind: a.agent.unwrap_or_else(|| "unknown".into()),
            pane_id: a.pane_id,
            tab_id: a.tab_id,
            workspace_id: a.workspace_id,
            status: a.agent_status,
            session_name: a.terminal_title_stripped,
            session_id: a.agent_session.and_then(|s| s.session_id()),
        })
        .collect();

    Ok(MachineView {
        machine: machine_name.clone(),
        agents,
        workspaces,
        tabs,
        dropped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct OverrunHerdr;
    impl FindHerdr for OverrunHerdr {
        fn agent_list(
            &self,
            _machine: Option<&str>,
            _timeout: Duration,
        ) -> Result<Lenient<AgentListEntry>, ProcessError> {
            std::thread::sleep(Duration::from_millis(10));
            Ok(Lenient {
                entries: vec![],
                dropped: 0,
            })
        }
        fn workspace_list(
            &self,
            _machine: Option<&str>,
            _timeout: Duration,
        ) -> Result<Lenient<HerdrWorkspace>, ProcessError> {
            panic!("must not start a workspace call after the shared deadline");
        }
        fn tab_list(
            &self,
            _machine: Option<&str>,
            _timeout: Duration,
        ) -> Result<Lenient<HerdrTab>, ProcessError> {
            panic!("must not start a tab call after the shared deadline");
        }
        fn machine_list(&self, _timeout: Duration) -> Result<Vec<MachineProfile>, ProcessError> {
            panic!("gather does not load machine profiles");
        }
    }

    #[test]
    fn topology_gather_does_not_start_more_calls_after_the_shared_deadline() {
        let result = gather_machine_view(
            &OverrunHerdr,
            None,
            &"draco".parse().unwrap(),
            Duration::from_millis(5),
        );
        assert!(matches!(result, Err(ProcessError::Timeout { .. })));
    }

    struct ProfileRunner {
        profiles: serde_json::Value,
        calls: Mutex<Vec<(String, Vec<String>)>>,
    }
    impl ProcessRunner for ProfileRunner {
        fn run(
            &self,
            program: &str,
            args: &[&str],
            _timeout: Duration,
        ) -> Result<herdr_cli::Output, ProcessError> {
            self.calls
                .lock()
                .unwrap()
                .push((program.into(), args.iter().map(|s| (*s).into()).collect()));
            let value = if args == ["machine", "list", "--json"] {
                self.profiles.clone()
            } else {
                serde_json::json!({"result":{"agents":[],"workspaces":[],"tabs":[]}})
            };
            Ok(herdr_cli::Output {
                status_code: 0,
                success: true,
                stdout: serde_json::to_vec(&value).unwrap(),
                stderr: vec![],
            })
        }
    }

    #[test]
    fn remote_reads_use_the_exact_saved_target_session_and_cached_profiles() {
        let herdr = RealFindHerdr::with_runner(ProfileRunner {
            profiles: serde_json::json!([{"label":"terra","enabled":true,"target":"cave","session":"default' ; printf bad"}]),
            calls: Mutex::new(vec![]),
        });
        herdr
            .agent_list(Some("TERRA"), Duration::from_secs(1))
            .unwrap();
        herdr
            .workspace_list(Some("terra"), Duration::from_secs(1))
            .unwrap();
        herdr
            .tab_list(Some("terra"), Duration::from_secs(1))
            .unwrap();
        let calls = herdr.runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 4, "one profile read plus three remote reads");
        assert_eq!(calls[0].0, "herdr");
        for (program, args) in &calls[1..] {
            assert_eq!(program, "ssh");
            assert_eq!(args[args.len() - 2], "cave");
            assert!(
                args.last()
                    .unwrap()
                    .contains("--session 'default'\"'\"' ; printf bad'")
            );
            assert!(!args.last().unwrap().contains("--machine"));
        }
    }

    #[test]
    fn missing_disabled_ambiguous_or_option_like_profiles_never_start_ssh() {
        for profiles in [
            serde_json::json!([{ "label":"terra","enabled":true }]),
            serde_json::json!([{ "label":"terra","enabled":false,"target":"cave","session":"default" }]),
            serde_json::json!([{ "label":"terra","enabled":true,"target":"-oProxyCommand=bad","session":"default" }]),
            serde_json::json!([{ "label":"terra","enabled":true,"target":"cave","session":"default" },{ "label":"TERRA","enabled":true,"target":"other","session":"default" }]),
        ] {
            let herdr = RealFindHerdr::with_runner(ProfileRunner {
                profiles,
                calls: Mutex::new(vec![]),
            });
            assert!(
                herdr
                    .agent_list(Some("terra"), Duration::from_secs(1))
                    .is_err()
            );
            assert!(
                herdr
                    .runner
                    .calls
                    .lock()
                    .unwrap()
                    .iter()
                    .all(|(program, _)| program != "ssh")
            );
        }
    }

    #[test]
    fn local_reads_do_not_load_profiles_or_invoke_ssh() {
        let herdr = RealFindHerdr::with_runner(ProfileRunner {
            profiles: serde_json::json!([]),
            calls: Mutex::new(vec![]),
        });
        herdr.agent_list(None, Duration::from_secs(1)).unwrap();
        let calls = herdr.runner.calls.lock().unwrap();
        assert_eq!(
            *calls,
            vec![("herdr".into(), vec!["agent".into(), "list".into()])]
        );
    }

    fn name(s: &str) -> AgentName {
        s.parse().unwrap()
    }

    fn reg(n: &str, machine: &str, pane: &str) -> Registration {
        Registration {
            name: name(n),
            machine: machine.parse().unwrap(),
            kind: "claude".into(),
            session_id: Some(format!("sid-{n}").parse().unwrap()),
            cwd: Some("/work".into()),
            herdr_pane: Some(pane.parse().unwrap()),
            place: None,
            last_seen_ms: 1,
        }
    }

    fn agent(pane: &str, tab: &str, workspace: &str, status: &str) -> HerdrAgent {
        HerdrAgent {
            kind: "unknown".into(),
            pane_id: pane.parse().unwrap(),
            tab_id: tab.parse().unwrap(),
            workspace_id: workspace.parse().unwrap(),
            status: status.into(),
            session_name: None,
            session_id: None,
        }
    }

    fn machine_view(
        machine: &str,
        agents: Vec<HerdrAgent>,
        ws_label: &str,
        tab_label: &str,
    ) -> MachineView {
        MachineView {
            dropped: 0,
            machine: machine.parse().unwrap(),
            workspaces: vec![HerdrWorkspace {
                workspace_id: "wQ".parse().unwrap(),
                label: ws_label.into(),
                tab_count: 1,
            }],
            tabs: vec![HerdrTab {
                tab_id: "wQ:t1".parse().unwrap(),
                label: tab_label.into(),
            }],
            agents,
        }
    }

    #[test]
    fn a_live_agent_registered_on_the_bus_joins_its_name_and_labels() {
        let mut live = agent("wQ:p1", "wQ:t1", "wQ", "idle");
        live.session_id = Some("sid-qsl-review".parse().unwrap());
        let machines = vec![machine_view("node-a", vec![live], "QSL", "review")];
        let regs = vec![reg("qsl-review", "node-a", "wQ:p1")];
        let rows = join(&machines, &regs);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].bus_name.as_ref().unwrap(), &name("qsl-review"));
        assert_eq!(rows[0].workspace_label.as_deref(), Some("QSL"));
        assert_eq!(rows[0].tab_label.as_deref(), Some("review"));
        assert!(rows[0].on_bus);
        assert_eq!(rows[0].herdr_status, "idle");
    }

    #[test]
    fn a_live_session_does_not_join_the_first_stale_registration_on_its_pane() {
        let mut live = agent("wQ:p1", "wQ:t1", "wQ", "working");
        live.session_id = Some("current-session".parse().unwrap());
        let mut stale = reg("stale-lead", "node-a", "wQ:p1");
        stale.session_id = Some("old-session".parse().unwrap());
        let machines = vec![machine_view("node-a", vec![live], "QSL", "review")];
        let rows = join(&machines, &[stale]);
        assert_eq!(rows[0].bus_name, None);
        assert!(!rows[0].on_bus);
    }

    #[test]
    fn a_sessionless_herdr_agent_does_not_verify_a_pane_claim() {
        let machines = vec![machine_view(
            "node-a",
            vec![agent("wQ:p1", "wQ:t1", "wQ", "working")],
            "QSL",
            "review",
        )];
        let regs = vec![reg("claimed-lead", "node-a", "wQ:p1")];
        let rows = join(&machines, &regs);
        assert_eq!(rows[0].bus_name, None);
        assert!(!rows[0].on_bus);
        assert!(
            rows[0].address.is_none(),
            "an unverified pane is not a send address"
        );
    }

    /// A row carries the typed ids of its pane, tab, workspace and session, so a
    /// consumer keyed on ids (the watcher's metrics, FR-045) needs no second join.
    ///
    /// Tracing: TC-045-03
    /// ACs: FR-045-AC-3
    #[test]
    fn tc_045_03_a_row_carries_the_ids_of_its_pane_tab_workspace_and_session() {
        let mut live = agent("wQ:p1", "wQ:t1", "wQ", "idle");
        live.session_id = Some("sess-1".parse().unwrap());
        let machines = vec![machine_view("node-a", vec![live], "QSL", "review")];
        let rows = join(&machines, &[]);
        assert_eq!(rows[0].pane_id.as_str(), "wQ:p1");
        assert_eq!(rows[0].tab_id.as_str(), "wQ:t1");
        assert_eq!(rows[0].workspace_id.as_str(), "wQ");
        assert_eq!(
            rows[0].session_id.as_ref().map(SessionId::as_str),
            Some("sess-1")
        );
    }

    #[test]
    fn a_live_agent_with_no_registration_is_still_listed_as_not_on_bus() {
        let machines = vec![machine_view(
            "node-a",
            vec![agent("wQ:p1", "wQ:t1", "wQ", "working")],
            "QSL",
            "1",
        )];
        let rows = join(&machines, &[]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].bus_name, None);
        assert!(!rows[0].on_bus);
    }

    #[test]
    fn a_row_carries_its_full_address_independent_of_tab_count() {
        let mut view = machine_view(
            "hub",
            vec![agent("wQ:p1", "wQ:t1", "wQ", "idle")],
            "EA",
            "code",
        );
        view.agents[0].session_id = Some("sid-qsl-review".parse().unwrap());
        view.workspaces[0].tab_count = 3;
        let mut registered = reg("qsl-review", "hub", "wQ:p1");
        registered.place = Some(Place {
            workspace: "EA".into(),
            tab: "code".into(),
            herdr_ids: Some(HerdrPlaceIds {
                workspace: "wQ".parse().unwrap(),
                tab: "wQ:t1".parse().unwrap(),
                pane: "wQ:p1".parse().unwrap(),
                session_id: "sid-qsl-review".parse().unwrap(),
                verified_at_ms: 1,
            }),
        });
        let rows = join(&[view.clone()], &[registered.clone()]);
        assert_eq!(rows[0].address.as_ref().unwrap().to_string(), "hub:EA:code");

        view.workspaces[0].tab_count = 1;
        let rows = join(&[view.clone()], &[registered.clone()]);
        assert_eq!(rows[0].address.as_ref().unwrap().to_string(), "hub:EA:code");

        // Unknown count (0): conservative, the tab stays.
        view.workspaces[0].tab_count = 0;
        let rows = join(&[view], &[registered]);
        assert_eq!(rows[0].address.as_ref().unwrap().to_string(), "hub:EA:code");
    }

    /// Tracing: TC-005-01
    /// ACs: FR-005-AC-1
    #[test]
    fn find_adds_the_pane_when_registered_agents_share_a_tab() {
        let mut first = agent("wQ:p1", "wQ:t1", "wQ", "idle");
        first.session_id = Some("sid-first".parse().unwrap());
        let mut second = agent("wQ:p2", "wQ:t1", "wQ", "working");
        second.session_id = Some("sid-second".parse().unwrap());
        let view = machine_view("hub", vec![first, second], "EA", "code");
        let mut first_reg = reg("first", "hub", "wQ:p1");
        first_reg.session_id = Some("sid-first".parse().unwrap());
        let mut second_reg = reg("second", "hub", "wQ:p2");
        second_reg.session_id = Some("sid-second".parse().unwrap());
        let proof = |session: &str| {
            Some(Place {
                workspace: "EA".into(),
                tab: "code".into(),
                herdr_ids: Some(HerdrPlaceIds {
                    workspace: "wQ".parse().unwrap(),
                    tab: "wQ:t1".parse().unwrap(),
                    pane: if session == "sid-first" {
                        "wQ:p1"
                    } else {
                        "wQ:p2"
                    }
                    .parse()
                    .unwrap(),
                    session_id: session.parse().unwrap(),
                    verified_at_ms: 1,
                }),
            })
        };
        first_reg.place = proof("sid-first");
        second_reg.place = proof("sid-second");
        let rows = join(&[view], &[first_reg, second_reg]);
        assert_eq!(rows[0].address.as_ref().unwrap().to_string(), "hub:EA:code");
        assert_eq!(rows[1].address.as_ref().unwrap().to_string(), "hub:EA:code");
    }

    #[test]
    fn places_maps_each_live_pane_to_its_workspace_tab_and_tab_count() {
        let mut view = machine_view(
            "hub",
            vec![agent("wQ:p1", "wQ:t1", "wQ", "idle")],
            "EA",
            "code",
        );
        view.agents[0].session_id = Some("sid-qsl-review".parse().unwrap());
        view.workspaces[0].tab_count = 3;
        let places = places(&view);
        let place = places.get(&"wQ:p1".parse().unwrap()).unwrap();
        assert_eq!(place.workspace, "EA");
        assert_eq!(place.tab, "code");
        let ids = place.herdr_ids.as_ref().expect("verified Herdr ids");
        assert_eq!(ids.workspace.as_str(), "wQ");
        assert_eq!(ids.tab.as_str(), "wQ:t1");
        assert_eq!(ids.session_id.as_str(), "sid-qsl-review");
    }

    #[test]
    fn places_ignores_tab_count_but_leaves_out_unknown_labels() {
        let mut known = agent("wQ:p1", "wQ:t1", "wQ", "idle");
        known.session_id = Some("sid-known".parse().unwrap());
        let mut view = machine_view(
            "hub",
            vec![known, agent("wX:p1", "wX:t1", "wX", "idle")],
            "EA",
            "code",
        );
        // wX has no workspace/tab entry at all; wQ's tab count is unknown (0).
        view.workspaces[0].tab_count = 0;
        assert_eq!(places(&view).len(), 1);
        assert!(places(&view).contains_key(&"wQ:p1".parse().unwrap()));
    }

    #[test]
    fn a_registration_with_no_live_herdr_agent_produces_no_row() {
        let machines = vec![machine_view("node-a", vec![], "QSL", "1")];
        let regs = vec![reg("qsl", "node-a", "wQ:p1")];
        assert!(join(&machines, &regs).is_empty());
    }

    #[test]
    fn a_registration_on_a_different_machine_does_not_match_the_same_pane_id() {
        // Pane ids are workspace-scoped, not globally unique — the join
        // must also match on machine, not pane id alone.
        let machines = vec![machine_view(
            "node-a",
            vec![agent("wQ:p1", "wQ:t1", "wQ", "idle")],
            "QSL",
            "1",
        )];
        let regs = vec![reg("hub-lead", "hub", "wQ:p1")];
        let rows = join(&machines, &regs);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].bus_name, None, "must not match hub's registration");
    }

    struct FakeFindHerdr {
        agents: Vec<AgentListEntry>,
        workspaces: Vec<HerdrWorkspace>,
        tabs: Vec<HerdrTab>,
        agent_list_calls: std::cell::RefCell<u32>,
        workspace_list_calls: std::cell::RefCell<u32>,
        tab_list_calls: std::cell::RefCell<u32>,
    }

    impl FindHerdr for FakeFindHerdr {
        fn agent_list(
            &self,
            _machine: Option<&str>,
            _timeout: Duration,
        ) -> Result<Lenient<AgentListEntry>, ProcessError> {
            *self.agent_list_calls.borrow_mut() += 1;
            Ok(Lenient {
                entries: self.agents.clone(),
                dropped: 0,
            })
        }
        fn workspace_list(
            &self,
            _machine: Option<&str>,
            _timeout: Duration,
        ) -> Result<Lenient<HerdrWorkspace>, ProcessError> {
            *self.workspace_list_calls.borrow_mut() += 1;
            Ok(Lenient {
                entries: self.workspaces.clone(),
                dropped: 0,
            })
        }
        fn tab_list(
            &self,
            _machine: Option<&str>,
            _timeout: Duration,
        ) -> Result<Lenient<HerdrTab>, ProcessError> {
            *self.tab_list_calls.borrow_mut() += 1;
            Ok(Lenient {
                entries: self.tabs.clone(),
                dropped: 0,
            })
        }
        fn machine_list(&self, _timeout: Duration) -> Result<Vec<MachineProfile>, ProcessError> {
            Ok(vec![])
        }
    }

    #[test]
    fn gather_machine_view_makes_exactly_one_call_each_regardless_of_workspace_count() {
        // AGE-2018 PR review, finding 3: no matter how many workspaces or
        // tabs the machine has, this must be exactly one `agent list`, one
        // `workspace list`, one `tab list` — never one `tab list` per
        // workspace, which is what made `find` keep timing out against
        // hub.
        let herdr = FakeFindHerdr {
            agents: vec![
                AgentListEntry {
                    agent: None,
                    pane_id: "wQ:p1".parse().unwrap(),
                    tab_id: "wQ:t1".parse().unwrap(),
                    workspace_id: "wQ".parse().unwrap(),
                    agent_status: "idle".into(),
                    terminal_title_stripped: Some("agent-a".into()),
                    agent_session: None,
                },
                AgentListEntry {
                    agent: None,
                    pane_id: "wR:p1".parse().unwrap(),
                    tab_id: "wR:t1".parse().unwrap(),
                    workspace_id: "wR".parse().unwrap(),
                    agent_status: "working".into(),
                    terminal_title_stripped: None,
                    agent_session: None,
                },
            ],
            workspaces: vec![
                HerdrWorkspace {
                    workspace_id: "wQ".parse().unwrap(),
                    label: "QSL".into(),
                    tab_count: 1,
                },
                HerdrWorkspace {
                    workspace_id: "wR".parse().unwrap(),
                    label: "Research".into(),
                    tab_count: 1,
                },
                HerdrWorkspace {
                    workspace_id: "wUnused".parse().unwrap(),
                    label: "NoAgentsHere".into(),
                    tab_count: 1,
                },
            ],
            tabs: vec![
                HerdrTab {
                    tab_id: "wQ:t1".parse().unwrap(),
                    label: "1".into(),
                },
                HerdrTab {
                    tab_id: "wR:t1".parse().unwrap(),
                    label: "1".into(),
                },
                HerdrTab {
                    tab_id: "wUnused:t1".parse().unwrap(),
                    label: "1".into(),
                },
            ],
            agent_list_calls: std::cell::RefCell::default(),
            workspace_list_calls: std::cell::RefCell::default(),
            tab_list_calls: std::cell::RefCell::default(),
        };

        let view = gather_machine_view(
            &herdr,
            None,
            &"node-a".parse().unwrap(),
            Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(view.agents.len(), 2);
        assert_eq!(view.agents[0].session_name.as_deref(), Some("agent-a"));
        // The unused workspace/tab are filtered out of the view entirely.
        assert_eq!(view.workspaces.len(), 2);
        assert!(
            view.workspaces
                .iter()
                .all(|w| w.workspace_id.as_str() != "wUnused")
        );
        assert_eq!(view.tabs.len(), 2);
        assert!(view.tabs.iter().all(|t| t.tab_id.as_str() != "wUnused:t1"));
        assert_eq!(*herdr.agent_list_calls.borrow(), 1);
        assert_eq!(*herdr.workspace_list_calls.borrow(), 1);
        assert_eq!(*herdr.tab_list_calls.borrow(), 1);
    }

    #[test]
    fn matches_query_checks_location_labels_session_name_and_pane_id_case_insensitively() {
        let row = FoundAgent {
            bus_name: Some(name("qsl-review")),
            address: Some(Address::new("node-a", "QSL", Some("Review"))),

            machine: "node-a".parse().unwrap(),
            workspace_label: Some("QSL".into()),
            tab_label: Some("Review".into()),
            pane_id: "wQ:p1".parse().unwrap(),
            tab_id: "wQ:t1".parse().unwrap(),
            workspace_id: "wQ".parse().unwrap(),
            session_id: None,
            session_name: Some("agent-d".into()),
            herdr_status: "idle".into(),
            on_bus: true,
        };
        for query in [
            "QSL-REVIEW",
            "review",
            "agent-d",
            "wQ:p1",
            "node-a:qsl:review",
            "",
        ] {
            assert!(matches_query(&row, query), "query {query:?} should match");
        }
        assert!(!matches_query(&row, "nonexistent"));
    }

    struct BodyRunner(&'static str);

    impl ProcessRunner for BodyRunner {
        fn run(
            &self,
            _program: &str,
            _args: &[&str],
            _timeout: Duration,
        ) -> Result<herdr_cli::Output, ProcessError> {
            Ok(herdr_cli::Output {
                status_code: 0,
                success: true,
                stdout: self.0.as_bytes().to_vec(),
                stderr: Vec::new(),
            })
        }
    }

    /// `find`'s lists tolerate one bad entry and count it; a list in which every
    /// entry fails is a decode failure, not an empty answer.
    #[test]
    fn a_find_list_drops_and_counts_a_bad_entry_but_fails_when_every_entry_is_bad() {
        let herdr = RealFindHerdr {
            profiles: Mutex::new(None),
            runner: BodyRunner(
                r#"{"result":{"agents":[
                    {"pane_id":"w1:p1","tab_id":"w1:t1","workspace_id":"w1","agent_status":"idle"},
                    {"pane_id":"","tab_id":"w1:t1","workspace_id":"w1","agent_status":"idle"}]}}"#,
            ),
        };
        let listed = herdr.agent_list(None, Duration::from_secs(1)).unwrap();
        assert_eq!((listed.entries.len(), listed.dropped), (1, 1));
        let all_bad = RealFindHerdr {
            profiles: Mutex::new(None),
            runner: BodyRunner(r#"{"result":{"agents":[{"id":"w1"},{"id":"w2"}]}}"#),
        };
        assert!(all_bad.agent_list(None, Duration::from_secs(1)).is_err());
    }

    /// herdr 0.9.1's `agent_session.kind` is `id` or `path`: only an id is a
    /// session id (AGE-2033 review, finding 3).
    ///
    /// Tracing: TC-043-02
    /// ACs: FR-043-AC-2
    #[test]
    fn tc_043_02_only_an_id_kind_is_a_session_id() {
        let read = |raw: &str| -> Option<SessionId> {
            serde_json::from_str::<ListedSession>(raw)
                .unwrap()
                .session_id()
        };
        assert_eq!(
            read(r#"{"agent":"claude","kind":"id","source":"herdr:claude","value":"sess-1"}"#),
            Some("sess-1".parse().unwrap())
        );
        assert_eq!(
            read(r#"{"kind":"path","value":"/w/rollout-1.jsonl"}"#),
            None
        );
        assert_eq!(
            read(r#"{"kind":"a-kind-from-the-future","value":"sess-1"}"#),
            None
        );
        assert_eq!(read(r#"{"value":"sess-1"}"#), None, "no kind is not an id");
        assert_eq!(
            read(r#"{"kind":"id","value":""}"#),
            None,
            "an empty id is none"
        );
    }
}
