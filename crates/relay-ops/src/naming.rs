//! Deriving a lead's bus name from its herdr location (AGE-2018): gathers the
//! herdr inputs [`relay_core::naming::derive_agent_name`] needs, and the
//! orchestration around it — resolving a cross-machine name collision
//! against the bus registry, and the timeout budget the caller splits across
//! four herdr calls, including a combined identity snapshot.
//!
//! Kept in relay-ops (not relay-core) because it shells out to herdr and
//! reads the bus registry — the same pattern this crate already uses for
//! `investigate` (see `herdr.rs`) and `escalate`/`resolve` (see
//! `escalate.rs`). `relay-core::naming` stays pure data-in/[`AgentName`]-out,
//! independent of herdr and Redis, so its rule can be proptested with no
//! process or network involved (see that module's own tests).

use std::time::Duration;

pub use herdr_cli::ProcessError;
use herdr_cli::ProcessRunner;
use relay_core::{
    AgentName, AgentSlot, HerdrPlaceIds, MachineName, PaneId, Place, Relay, SessionId, TabId,
    WorkspaceId,
    naming::{PaneLocation, TabLocation, WorkspaceLocation, derive_agent_name},
};
use serde::Deserialize;

/// The herdr calls [`derive_from_herdr`] needs to work out where a pane
/// sits: which workspace, which tab (and whether it's the workspace's
/// first), and which pane (and its position among the tab's other panes).
pub trait LocationHerdr {
    /// `herdr workspace list`.
    fn workspace_list(&self) -> Result<Vec<WorkspaceEntry>, ProcessError>;
    /// `herdr tab list --workspace <id>`.
    fn tab_list(&self, workspace_id: &WorkspaceId) -> Result<Vec<TabEntry>, ProcessError>;
    /// `herdr pane list --workspace <id>`.
    fn pane_list(&self, workspace_id: &WorkspaceId) -> Result<Vec<PaneEntry>, ProcessError>;
    /// `herdr agent get <pane_id>`: one snapshot of the pane's optional
    /// deliberately assigned name and current harness session id. The same
    /// response verifies placement and participates in name derivation, so a
    /// pane move cannot split those facts across separate reads.
    fn agent(&self, pane_id: &PaneId) -> Result<Option<HerdrAgent>, ProcessError>;
}

/// Herdr's identity snapshot for one pane.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HerdrAgent {
    /// A deliberately assigned Herdr name, if any.
    pub name: Option<String>,
    /// The harness session Herdr currently reports in the pane, if any.
    pub session_id: Option<SessionId>,
}

/// One entry from `herdr workspace list`'s `workspaces` array. herdr's own
/// payload also carries a workspace `number`, but AGE-2018's naming rule
/// never needs it (only a tab's and a pane's position matter) — not decoded
/// here; `serde` ignores the extra field without `deny_unknown_fields`.
#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceEntry {
    /// The workspace's opaque id, e.g. `wE`.
    pub workspace_id: WorkspaceId,
    /// The workspace's own label, e.g. `QSL`.
    pub label: String,
}

/// One entry from `herdr tab list`'s `tabs` array.
#[derive(Debug, Clone, Deserialize)]
pub struct TabEntry {
    /// The tab's opaque id, e.g. `wE:t1`.
    pub tab_id: TabId,
    /// The tab's label. An unlabelled tab's label equals its own
    /// `display_number` rendered as a string.
    pub label: String,
    /// herdr's own tab *display* number: renumbers when a sibling tab
    /// closes. Used only to recognize herdr's "label equals number"
    /// unlabelled-tab convention — never to build a name or decide which
    /// tab is first; both of those use the tab id's own stable number
    /// instead (parsed from `tab_id` — see `id_number`, private).
    #[serde(rename = "number")]
    pub display_number: u64,
}

/// One entry from `herdr pane list`'s `panes` array. herdr's pane JSON
/// carries no `number` field of its own; `id_number` (private) parses one
/// out of `pane_id`.
#[derive(Debug, Clone, Deserialize)]
pub struct PaneEntry {
    /// The pane's opaque, workspace-scoped id, e.g. `wE:p3`.
    pub pane_id: PaneId,
    /// The tab this pane belongs to.
    pub tab_id: TabId,
}

/// This process's herdr location, from the env vars herdr injects into every
/// managed pane.
#[derive(Debug, Clone)]
pub struct HerdrEnv {
    /// `HERDR_PANE_ID`, an inherited location claim that must be checked against
    /// Herdr before it is used as a verified registration location.
    pub pane_id: PaneId,
    /// `HERDR_TAB_ID`.
    pub tab_id: TabId,
    /// `HERDR_WORKSPACE_ID`.
    pub workspace_id: WorkspaceId,
}

impl HerdrEnv {
    /// `HERDR_PANE_ID` alone: the pane this process claims. `None` when it is
    /// unset, empty, or not a pane id (see [`PaneId`]). The one reader of that
    /// variable, so a pane id reaches the registry only as a [`PaneId`].
    #[must_use]
    pub fn pane_id_from_process_env() -> Option<PaneId> {
        std::env::var("HERDR_PANE_ID").ok()?.parse().ok()
    }

    /// The tiled pane a Herdr plugin popup was opened over, when available.
    /// Popup panes have no own pane id; this reads the host-provided plugin
    /// context and never treats the popup itself as a lead.
    #[must_use]
    pub fn focused_pane_id_from_process_env() -> Option<PaneId> {
        Self::pane_id_from_process_env().or_else(|| {
            let context = std::env::var("HERDR_PLUGIN_CONTEXT_JSON").ok()?;
            focused_pane_from_plugin_context(&context)
        })
    }

    /// Reads the three env vars herdr sets. `None` if any is unset or empty:
    /// not running inside a herdr-managed pane (the common case for a
    /// one-off or non-herdr session), or — shouldn't happen — only
    /// partially propagated.
    #[must_use]
    pub fn from_process_env() -> Option<Self> {
        fn get<T: std::str::FromStr>(name: &str) -> Option<T> {
            std::env::var(name).ok()?.parse().ok()
        }
        Some(Self {
            pane_id: get("HERDR_PANE_ID")?,
            tab_id: get("HERDR_TAB_ID")?,
            workspace_id: get("HERDR_WORKSPACE_ID")?,
        })
    }
}

#[derive(serde::Deserialize)]
struct PluginContext {
    focused_pane_id: Option<PaneId>,
}

fn focused_pane_from_plugin_context(context: &str) -> Option<PaneId> {
    serde_json::from_str::<PluginContext>(context)
        .ok()?
        .focused_pane_id
}

/// Shells out to the real `herdr` binary on `PATH`, through `R`
/// ([`herdr_cli::RealProcessRunner`] in production).
pub struct RealLocationHerdr<R: ProcessRunner = herdr_cli::RealProcessRunner> {
    runner: R,
    timeout: Duration,
}

impl RealLocationHerdr<herdr_cli::RealProcessRunner> {
    /// The production `RealLocationHerdr`. `timeout` is capped by the
    /// remaining shared deadline before each Herdr call starts.
    #[must_use]
    pub fn new(timeout: Duration) -> Self {
        Self {
            runner: herdr_cli::RealProcessRunner,
            timeout,
        }
    }
}

impl<R: ProcessRunner> LocationHerdr for RealLocationHerdr<R> {
    fn workspace_list(&self) -> Result<Vec<WorkspaceEntry>, ProcessError> {
        #[derive(Deserialize)]
        struct Listing {
            workspaces: Vec<WorkspaceEntry>,
        }
        let listing: Listing =
            herdr_cli::call(&self.runner, "herdr", &["workspace", "list"], self.timeout)?;
        Ok(listing.workspaces)
    }

    fn tab_list(&self, workspace_id: &WorkspaceId) -> Result<Vec<TabEntry>, ProcessError> {
        #[derive(Deserialize)]
        struct Listing {
            tabs: Vec<TabEntry>,
        }
        let listing: Listing = herdr_cli::call(
            &self.runner,
            "herdr",
            &["tab", "list", "--workspace", workspace_id.as_str()],
            self.timeout,
        )?;
        Ok(listing.tabs)
    }

    fn pane_list(&self, workspace_id: &WorkspaceId) -> Result<Vec<PaneEntry>, ProcessError> {
        #[derive(Deserialize)]
        struct Listing {
            panes: Vec<PaneEntry>,
        }
        let listing: Listing = herdr_cli::call(
            &self.runner,
            "herdr",
            &["pane", "list", "--workspace", workspace_id.as_str()],
            self.timeout,
        )?;
        Ok(listing.panes)
    }

    fn agent(&self, pane_id: &PaneId) -> Result<Option<HerdrAgent>, ProcessError> {
        #[derive(Deserialize)]
        struct GetResult {
            agent: AgentInfo,
        }
        #[derive(Deserialize)]
        struct AgentInfo {
            // `name` is present in `herdr agent get`'s JSON only after a
            // deliberate `herdr agent rename` (or `agent start <name>`) —
            // confirmed live on node-a: absent entirely before a rename,
            // present with the given name after, gone again after
            // `--clear`. `terminal_title_stripped` was tried first and
            // rejected: it reflects whatever text is currently in the
            // terminal title bar — the shell's own pre-launch title, then
            // Claude's generic "Claude Code" idle title, then a
            // conversation-derived title — none of which is a deliberate
            // rename, and it raced with the pane's own startup, so it
            // fired on nearly every pane regardless of any real rename.
            #[serde(default)]
            name: Option<String>,
            #[serde(default)]
            agent_session: Option<crate::find::ListedSession>,
        }
        let result = herdr_cli::call::<GetResult>(
            &self.runner,
            "herdr",
            &["agent", "get", pane_id.as_str()],
            self.timeout,
        );
        match result {
            // A session value that is not a session id cannot be the registered one.
            Ok(r) => Ok(Some(HerdrAgent {
                name: r.agent.name,
                session_id: r.agent.agent_session.and_then(|s| s.session_id()),
            })),
            Err(ProcessError::Reported { code, .. }) if code == "agent_not_found" => Ok(None),
            Err(e) => Err(e),
        }
    }
}

/// Parses the numeric suffix off a herdr tab or pane id (`<workspace>:t<N>`
/// or `<workspace>:p<N>`): skips the workspace prefix and the one-letter
/// type tag (`t`/`p`), then parses the rest as a number.
///
/// The operator's 2026-09-25 correction to AGE-2018: a derived name (and "is this
/// the first tab" / "is this pane alone in its tab") must use herdr's own
/// *id* numbers, which are never reused or renumbered, not the *display*
/// numbers herdr shows in the UI and reuses/renumbers when a sibling
/// tab/pane closes (`tab list`'s `number` field is a display number; a
/// pane's JSON carries no number field of its own at all). Documented here
/// as an explicit assumption about herdr's current id scheme (`herdr
/// --skill` calls IDs "opaque stable handles"): if herdr ever changes that
/// scheme, [`derive_from_herdr`] fails closed (`None`) rather than silently
/// misnaming something.
fn id_number(id: &impl AsRef<str>) -> Option<u64> {
    let suffix = id.as_ref().rsplit(':').next()?;
    let mut chars = suffix.chars();
    chars.next()?; // the one-letter type tag ('t' or 'p')
    chars.as_str().parse().ok()
}

/// What [`derive_from_herdr`] found: the derived name (if any), and whether
/// this pane's tab is an escalation investigation fork.
///
/// Split out as its own field (AGE-2018 PR review, finding 1) rather than
/// folded into `name.is_none()`: an unresolvable location (workspace/tab/pane
/// not found, or an id that doesn't parse) is *also* `name: None`, but is not
/// an escalation fork — callers that need to tell "skip, this is an
/// investigation fork; suppress the whole session" apart from "skip, we
/// just couldn't work out a name; try another precedence tier" need this
/// distinction, which `Option<AgentName>` alone can't carry.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HerdrLocation {
    /// The derived name, or `None` if the location can't be resolved
    /// cleanly, or the tab is an escalation fork (see `is_escalation_fork`).
    pub name: Option<AgentName>,
    /// Whether this pane's tab is an escalation investigation fork (an
    /// `esc-*` label — see
    /// [`relay_core::naming::TabLocation::is_escalation_fork`]). A caller
    /// must treat this as "suppress the whole session," not just "no
    /// location name" — an escalation fork must never join the bus under
    /// *any* precedence tier, not only the location one.
    pub is_escalation_fork: bool,
    /// Where this pane sits (Herdr's stable workspace/tab/pane ids and labels), for the
    /// registration's `server:workspace[:tab[:pane]]` address. `None` whenever `name`
    /// is unresolvable for a location reason, and for an escalation fork.
    pub place: Option<Place>,
    /// The agent identity snapshot used to verify the placement.
    pub agent: Option<HerdrAgent>,
}

/// Turns one pane's herdr location into a derived bus name, per
/// [`derive_agent_name`]'s naming rule.
///
/// `name` is `None` both for an `esc-*` tab (see
/// [`HerdrLocation::is_escalation_fork`]) and when the location can't be
/// resolved cleanly — `env`'s workspace, tab or pane isn't in the listing
/// herdr just returned, or a tab/pane id doesn't parse (see `id_number`,
/// private). Both cases leave `name: None`; `is_escalation_fork` tells them
/// apart.
///
/// # Errors
///
/// Propagates [`ProcessError`] from any Herdr call.
pub fn derive_from_herdr(
    herdr: &impl LocationHerdr,
    env: &HerdrEnv,
) -> Result<HerdrLocation, ProcessError> {
    let workspaces = herdr.workspace_list()?;
    let Some(workspace) = workspaces
        .iter()
        .find(|w| w.workspace_id == env.workspace_id)
    else {
        return Ok(HerdrLocation::default());
    };

    let tabs = herdr.tab_list(&env.workspace_id)?;
    let Some(tab) = tabs.iter().find(|t| t.tab_id == env.tab_id) else {
        return Ok(HerdrLocation::default());
    };
    let Some(tab_id_number) = id_number(&tab.tab_id) else {
        return Ok(HerdrLocation::default());
    };
    let mut min_tab_id_number: Option<u64> = None;
    for other in &tabs {
        let Some(n) = id_number(&other.tab_id) else {
            // Can't reliably tell which tab is first without every
            // sibling's id number — fail closed rather than guess.
            return Ok(HerdrLocation::default());
        };
        min_tab_id_number = Some(min_tab_id_number.map_or(n, |m| m.min(n)));
    }
    let is_first = min_tab_id_number == Some(tab_id_number);

    let tab_loc = TabLocation {
        label: &tab.label,
        display_number: tab.display_number,
        id_number: tab_id_number,
        is_first,
    };
    if tab_loc.is_escalation_fork() {
        return Ok(HerdrLocation {
            name: None,
            is_escalation_fork: true,
            place: None,
            agent: None,
        });
    }

    let panes = herdr.pane_list(&env.workspace_id)?;
    let same_tab: Vec<&PaneEntry> = panes.iter().filter(|p| p.tab_id == env.tab_id).collect();
    if !same_tab.iter().any(|p| p.pane_id == env.pane_id) {
        return Ok(HerdrLocation::default());
    }
    let is_lone = same_tab.len() <= 1;
    let pane_id_number = if is_lone {
        0 // unused by derive_agent_name when is_lone is true.
    } else {
        let Some(n) = id_number(&env.pane_id) else {
            return Ok(HerdrLocation::default());
        };
        n
    };

    let ws_loc = WorkspaceLocation {
        label: &workspace.label,
    };
    let pane_loc = PaneLocation {
        id_number: pane_id_number,
        is_lone,
    };
    let agent = herdr.agent(&env.pane_id)?;
    let session_id = agent.as_ref().and_then(|agent| agent.session_id.clone());
    Ok(HerdrLocation {
        name: derive_agent_name(&ws_loc, &tab_loc, &pane_loc),
        is_escalation_fork: false,
        place: Some(Place {
            workspace: workspace.label.clone(),
            tab: tab.label.clone(),
            herdr_ids: session_id.map(|session_id| HerdrPlaceIds {
                workspace: env.workspace_id.clone(),
                tab: env.tab_id.clone(),
                pane: env.pane_id.clone(),
                session_id,
                verified_at_ms: herdr_verified_at_ms(),
            }),
        }),
        agent,
    })
}

fn herdr_verified_at_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Whether a registered session is still running in its pane — the one
/// question [`resolve_collision`] asks about a same-machine registration that
/// is not ours.
///
/// Every [`LocationHerdr`] answers it from `herdr agent get`. [`AssumeHosted`]
/// answers "yes" without a herdr call, for the hook event that must make none.
pub trait PaneLiveness {
    /// Whether `pane` currently hosts session `session_id`.
    ///
    /// # Errors
    ///
    /// [`ProcessError`] when herdr cannot be asked.
    fn hosts(&self, pane: &PaneId, session_id: &SessionId) -> Result<bool, ProcessError>;
}

impl<T: LocationHerdr> PaneLiveness for T {
    fn hosts(&self, pane: &PaneId, session_id: &SessionId) -> Result<bool, ProcessError> {
        Ok(self
            .agent(pane)?
            .and_then(|agent| agent.session_id)
            .as_ref()
            == Some(session_id))
    }
}

/// A [`PaneLiveness`] that makes no herdr call and assumes every registered
/// session is still live. Used where no herdr call is allowed (the
/// `UserPromptSubmit` hook): the safe direction, since it can cost a
/// needless `-2` suffix but never lets two live leads share one inbox.
#[derive(Debug, Clone, Copy)]
pub struct AssumeHosted;

impl PaneLiveness for AssumeHosted {
    fn hosts(&self, _pane: &PaneId, _session_id: &SessionId) -> Result<bool, ProcessError> {
        Ok(true)
    }
}

/// Why [`resolve_collision`] could not name a free name. Every variant means
/// "do not register the candidate": it is held by another session, so falling
/// back to it would put two live leads on one inbox (FR-022-AC-6).
#[derive(Debug, thiserror::Error)]
pub enum CollisionError {
    /// A registry read failed.
    #[error(transparent)]
    Relay(#[from] relay_core::Error),
    /// `candidate` is taken and every widened or numbered variant of it is
    /// taken too.
    #[error("no free bus name near {candidate}: every variant is already registered")]
    NoFreeName {
        /// The name that collided.
        candidate: AgentName,
    },
}

/// The name [`resolve_collision`] chose, and what went wrong on the way that
/// did not stop it.
#[derive(Debug)]
pub struct Resolved {
    /// The name to register under.
    pub name: AgentName,
    /// Set when the candidate's same-machine holder could not be shown gone
    /// because herdr could not be asked (FR-022: the holder was treated as
    /// live). The caller records it; it does not change `name`.
    pub liveness_error: Option<ProcessError>,
}

/// Resolves a name collision against the bus registry (AGE-2018):
///
/// - no registration under `candidate`, or one that is *ours* (the same
///   session id — both sides must actually know it: an absent id on either
///   side is "unknown", never "the same caller"): keeps the candidate;
/// - a *different* machine and a different (or unknown) session: widens to
///   `<candidate>-<machine>`, so the two don't fight over one name;
/// - the *same* machine, a different session, and that other session's own
///   pane still hosts it (`liveness`; genuinely two independent,
///   still-running leads landing on the same derived name — the operator's
///   2026-09-25 correction, finding 8): widens to `<candidate>-2`, `-3`, ...
///   instead of silently sharing one inbox between two leads;
/// - the same machine, but that other session is gone (a stale registration
///   left behind by a restart in the same spot): keeps the candidate, which
///   [`Relay::register`]'s upsert-by-name takes over;
/// - the same machine, and herdr cannot say whether that session is still
///   there (an error, or a spent hook budget): treats it as live, so numbers
///   the name as above, and reports the herdr error in
///   [`Resolved::liveness_error`].
///
/// Every widened or numbered name is built with
/// [`relay_core::naming::with_suffix`] (a base already at the 32-character
/// limit is truncated so the suffix survives) and is checked itself: it is
/// used only if unregistered, or registered to this same session, so a
/// re-derivation keeps landing on `-2` rather than drifting to `-3`.
///
/// # Errors
///
/// [`CollisionError::Relay`] from the registry lookups;
/// [`CollisionError::NoFreeName`] when every variant up to
/// [`relay_core::naming::MAX_VARIANT_NUMBER`] is taken. Neither yields a name:
/// a caller that cannot resolve the collision must not register the candidate,
/// which another live session holds (FR-022-AC-6).
pub fn resolve_collision(
    relay: &mut Relay,
    liveness: &impl PaneLiveness,
    candidate: AgentName,
    this_machine: &MachineName,
    this_session_id: Option<&SessionId>,
) -> Result<Resolved, CollisionError> {
    let plain = |name| Resolved {
        name,
        liveness_error: None,
    };
    let existing = match relay.agent_slot(&candidate)? {
        AgentSlot::Absent => return Ok(plain(candidate)),
        AgentSlot::Registered(existing) => existing,
        // Registered, but not readable here (an older writer's id this binary
        // refuses): the name is held, so it is never registered over. Whose it
        // is cannot be told, so it is numbered, not widened by machine.
        AgentSlot::Unreadable => return numbered_variant(relay, candidate, this_session_id),
    };
    if same_session(existing.session_id.as_ref(), this_session_id) {
        return Ok(plain(candidate));
    }
    let mut liveness_error = None;
    let widen = if existing.machine == *this_machine {
        // Unknown is not "gone": a registration with no session id or no
        // pane (e.g. one made by `agent-msg register` outside a hook) cannot
        // be shown stale, so the name is not taken from it. Neither can one
        // whose check failed.
        let live = match (&existing.herdr_pane, &existing.session_id) {
            (Some(pane), Some(session_id)) => match liveness.hosts(pane, session_id) {
                Ok(hosted) => hosted,
                Err(e) => {
                    liveness_error = Some(e);
                    true
                }
            },
            _ => true,
        };
        if !live {
            return Ok(plain(candidate));
        }
        None
    } else {
        Some(this_machine.as_str())
    };
    let name = relay_core::naming::free_variant(&candidate, widen, |variant| {
        Ok::<_, relay_core::Error>(match relay.agent_slot(variant)? {
            AgentSlot::Absent => true,
            AgentSlot::Registered(other) => {
                same_session(other.session_id.as_ref(), this_session_id)
            }
            AgentSlot::Unreadable => false,
        })
    })?
    .ok_or(CollisionError::NoFreeName { candidate })?;
    Ok(Resolved {
        name,
        liveness_error,
    })
}

/// The first free numbered variant of `candidate`, for a candidate whose holder
/// cannot be read.
fn numbered_variant(
    relay: &mut Relay,
    candidate: AgentName,
    this_session_id: Option<&SessionId>,
) -> Result<Resolved, CollisionError> {
    let name = relay_core::naming::free_variant(&candidate, None, |variant| {
        Ok::<_, relay_core::Error>(match relay.agent_slot(variant)? {
            AgentSlot::Absent => true,
            AgentSlot::Registered(other) => {
                same_session(other.session_id.as_ref(), this_session_id)
            }
            AgentSlot::Unreadable => false,
        })
    })?
    .ok_or(CollisionError::NoFreeName { candidate })?;
    Ok(Resolved {
        name,
        liveness_error: None,
    })
}

/// Whether two registrations' session ids name the same session: only when
/// both are known and equal. Two absent ids are two unknown sessions.
fn same_session(a: Option<&SessionId>, b: Option<&SessionId>) -> bool {
    matches!((a, b), (Some(a), Some(b)) if a == b)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    /// Trace: FR-049-AC-1
    #[test]
    fn herdr_plugin_popup_context_identifies_only_its_underlying_tiled_pane() {
        assert_eq!(
            focused_pane_from_plugin_context(
                r#"{"focused_pane_id":"w1:p2","invocation_source":"keybinding","future":true}"#
            )
            .as_ref()
            .map(PaneId::as_str),
            Some("w1:p2")
        );
        assert_eq!(
            focused_pane_from_plugin_context(r#"{"focused_pane_id":"w1\n:p2"}"#),
            None
        );
        assert_eq!(
            focused_pane_from_plugin_context(r#"{"workspace_id":"w1"}"#),
            None
        );
        assert_eq!(focused_pane_from_plugin_context("not json"), None);
    }

    #[derive(Default)]
    struct FakeLocationHerdr {
        workspaces: Vec<WorkspaceEntry>,
        tabs: Vec<TabEntry>,
        panes: Vec<PaneEntry>,
        agent_names: std::collections::HashMap<String, String>,
        pane_sessions: std::collections::HashMap<String, String>,
        fail: RefCell<Option<ProcessError>>,
    }

    impl LocationHerdr for FakeLocationHerdr {
        fn workspace_list(&self) -> Result<Vec<WorkspaceEntry>, ProcessError> {
            if let Some(e) = self.fail.borrow().clone() {
                return Err(e);
            }
            Ok(self.workspaces.clone())
        }
        fn tab_list(&self, _workspace_id: &WorkspaceId) -> Result<Vec<TabEntry>, ProcessError> {
            Ok(self.tabs.clone())
        }
        fn pane_list(&self, _workspace_id: &WorkspaceId) -> Result<Vec<PaneEntry>, ProcessError> {
            Ok(self.panes.clone())
        }
        fn agent(&self, pane_id: &PaneId) -> Result<Option<HerdrAgent>, ProcessError> {
            let name = self.agent_names.get(pane_id.as_str()).cloned();
            let session_id = self
                .pane_sessions
                .get(pane_id.as_str())
                .and_then(|s| s.parse().ok());
            Ok((name.is_some() || session_id.is_some()).then_some(HerdrAgent { name, session_id }))
        }
    }

    fn env() -> HerdrEnv {
        HerdrEnv {
            pane_id: "wE:p1".parse().unwrap(),
            tab_id: "wE:t1".parse().unwrap(),
            workspace_id: "wE".parse().unwrap(),
        }
    }

    fn herdr_with(
        label: &str,
        tab_label: &str,
        tab_display_number: u64,
        tab_id: &str,
        panes: &[&str],
    ) -> FakeLocationHerdr {
        FakeLocationHerdr {
            workspaces: vec![WorkspaceEntry {
                workspace_id: "wE".parse().unwrap(),
                label: label.into(),
            }],
            tabs: vec![TabEntry {
                tab_id: tab_id.parse().unwrap(),
                label: tab_label.into(),
                display_number: tab_display_number,
            }],
            panes: panes
                .iter()
                .map(|id| PaneEntry {
                    pane_id: id.parse().unwrap(),
                    tab_id: tab_id.parse().unwrap(),
                })
                .collect(),
            agent_names: std::collections::HashMap::new(),
            pane_sessions: panes
                .iter()
                .map(|pane| (pane.to_string(), "session-test".to_owned()))
                .collect(),
            fail: RefCell::default(),
        }
    }

    #[test]
    fn derives_the_first_tab_lone_pane_name() {
        let herdr = herdr_with("QSL", "1", 1, "wE:t1", &["wE:p1"]);
        let loc = derive_from_herdr(&herdr, &env()).unwrap();
        assert_eq!(loc.name.unwrap().as_str(), "qsl");
        assert!(!loc.is_escalation_fork);
    }

    #[test]
    fn the_location_carries_the_workspace_and_tab_labels_and_the_tab_count() {
        let mut herdr = herdr_with("EA", "code", 1, "wE:t1", &["wE:p1"]);
        herdr.tabs.push(TabEntry {
            tab_id: "wE:t2".parse().unwrap(),
            label: "plan".into(),
            display_number: 2,
        });
        let loc = derive_from_herdr(&herdr, &env()).unwrap();
        let place = loc.place.unwrap();
        assert_eq!(place.workspace, "EA");
        assert_eq!(place.tab, "code");
        let ids = place.herdr_ids;
        let ids = ids.as_ref().expect("verified Herdr ids");
        assert_eq!(ids.workspace.as_str(), "wE");
        assert_eq!(ids.tab.as_str(), "wE:t1");
        assert_eq!(ids.session_id.as_str(), "session-test");
        assert!(ids.verified_at_ms > 0);
    }

    #[test]
    fn an_escalation_fork_or_unknown_location_has_no_place() {
        let fork = herdr_with("QSL", "esc-e7-1", 2, "wE:t1", &["wE:p1"]);
        assert_eq!(derive_from_herdr(&fork, &env()).unwrap().place, None);
        let herdr = herdr_with("QSL", "1", 1, "wE:t1", &["wE:p1"]);
        let mut e = env();
        e.workspace_id = "wZ".parse().unwrap();
        assert_eq!(derive_from_herdr(&herdr, &e).unwrap().place, None);
    }

    #[test]
    fn a_non_lone_pane_uses_its_own_stable_id_number_not_a_computed_rank() {
        // Two panes in the tab: our own is p5, sibling is p1. The suffix
        // must be the pane's own id (5), never a recomputed rank (e.g. "2nd
        // pane" -> p2) — the operator's 2026-09-25 correction.
        let herdr = herdr_with("QSL", "1", 1, "wE:t1", &["wE:p1", "wE:p5"]);
        let mut e = env();
        e.pane_id = "wE:p5".parse().unwrap();
        let name = derive_from_herdr(&herdr, &e).unwrap().name.unwrap();
        assert_eq!(name.as_str(), "qsl-p5");
    }

    #[test]
    fn tab_naming_uses_the_tab_ids_own_number_not_its_renumbering_display_number() {
        // Two tabs: t1 (the true first, by id) and t3 (ours) — herdr's
        // *display* number for t3 is 2 (renumbered after some other sibling
        // tab closed). The derived name must use the id number, 3, and must
        // not treat t3 as first just because its display number is low.
        let mut herdr = herdr_with("QSL", "2", 2, "wE:t3", &["wE:p1"]);
        herdr.tabs.push(TabEntry {
            tab_id: "wE:t1".parse().unwrap(),
            label: "1".into(),
            display_number: 1,
        });
        let mut e = env();
        e.tab_id = "wE:t3".parse().unwrap();
        let name = derive_from_herdr(&herdr, &e).unwrap().name.unwrap();
        assert_eq!(name.as_str(), "qsl-t3");
    }

    #[test]
    fn unknown_workspace_or_tab_or_pane_is_none_not_an_error() {
        let herdr = herdr_with("QSL", "1", 1, "wE:t1", &["wE:p1"]);
        let mut e = env();
        e.workspace_id = "wZ".parse().unwrap();
        let loc = derive_from_herdr(&herdr, &e).unwrap();
        assert_eq!(loc.name, None);
        assert!(!loc.is_escalation_fork);

        let mut e = env();
        e.tab_id = "wE:t9".parse().unwrap();
        assert_eq!(derive_from_herdr(&herdr, &e).unwrap().name, None);

        let mut e = env();
        e.pane_id = "wE:p9".parse().unwrap();
        assert_eq!(derive_from_herdr(&herdr, &e).unwrap().name, None);
    }

    #[test]
    fn esc_prefixed_tab_derives_to_none_and_is_flagged_as_an_escalation_fork() {
        let herdr = herdr_with("QSL", "esc-e7-1", 2, "wE:t1", &["wE:p1"]);
        let loc = derive_from_herdr(&herdr, &env()).unwrap();
        assert_eq!(loc.name, None);
        assert!(loc.is_escalation_fork);
    }

    #[test]
    fn herdr_failure_propagates() {
        let herdr = FakeLocationHerdr {
            fail: RefCell::new(Some(ProcessError::Spawn {
                program: "herdr".into(),
                reason: "not on PATH".into(),
            })),
            ..FakeLocationHerdr::default()
        };
        assert!(derive_from_herdr(&herdr, &env()).is_err());
    }

    // ---- resolve_collision -------------------------------------------------

    fn relay(test: &str) -> Option<Relay> {
        let url = std::env::var("IX_RELAY_TEST_URL").ok()?;
        let relay = Relay::connect(&url, Duration::from_secs(2)).expect("connect to test redis");
        Some(relay.with_prefix(format!(
            "test:relay-ops:naming:{}:{test}:",
            std::process::id()
        )))
    }

    fn name(s: &str) -> AgentName {
        s.parse().unwrap()
    }

    fn register(relay: &mut Relay, n: &str, machine: &str, session_id: Option<&str>) {
        register_at(relay, n, machine, session_id, "wE:p1");
    }

    fn register_at(
        relay: &mut Relay,
        n: &str,
        machine: &str,
        session_id: Option<&str>,
        pane: &str,
    ) {
        relay
            .register(&relay_core::Registration {
                name: name(n),
                machine: machine.parse().unwrap(),
                kind: "claude".into(),
                session_id: session_id.map(|s| s.parse().unwrap()),
                cwd: None,
                herdr_pane: Some(pane.parse().unwrap()),
                place: None,
                last_seen_ms: 1,
            })
            .unwrap();
    }

    /// No herdr calls configured to return anything: every `pane_session`
    /// lookup comes back `Ok(None)`, i.e. "that pane doesn't (or no longer)
    /// host the registration that once claimed it" — the common case for
    /// every collision test that isn't specifically about finding 8's live
    /// same-machine check.
    fn resolve_name(
        relay: &mut Relay,
        liveness: &impl PaneLiveness,
        candidate: AgentName,
        machine: &str,
        session: Option<&str>,
    ) -> Result<AgentName, CollisionError> {
        resolve_collision(
            relay,
            liveness,
            candidate,
            &machine.parse().unwrap(),
            session.map(|s| s.parse().unwrap()).as_ref(),
        )
        .map(|r| r.name)
    }

    fn no_herdr() -> FakeLocationHerdr {
        FakeLocationHerdr::default()
    }

    #[test]
    fn no_existing_registration_keeps_the_candidate() {
        let Some(mut r) = relay("no-existing") else {
            return;
        };
        let resolved =
            resolve_name(&mut r, &no_herdr(), name("qsl"), "node-a", Some("sid-1")).unwrap();
        assert_eq!(resolved, name("qsl"));
    }

    #[test]
    fn same_machine_restart_keeps_the_candidate_even_with_a_new_session() {
        let Some(mut r) = relay("same-machine-restart") else {
            return;
        };
        register(&mut r, "qsl", "node-a", Some("old-sid"));
        // `no_herdr()`: the old session's pane no longer reports that
        // session (or any session) — a genuine restart in the same spot.
        let resolved =
            resolve_name(&mut r, &no_herdr(), name("qsl"), "node-a", Some("new-sid")).unwrap();
        assert_eq!(resolved, name("qsl"), "same machine: no widening");
    }

    #[test]
    fn same_machine_a_still_live_other_session_gets_numbered_instead_of_sharing_the_name() {
        // The operator's 2026-09-25 correction, finding 8: same machine, different
        // session — but this time the other registration's own pane still
        // reports hosting exactly that session, live, right now.
        let Some(mut r) = relay("same-machine-live-collision") else {
            return;
        };
        register_at(&mut r, "qsl", "node-a", Some("old-sid"), "wE:p1");
        let herdr = FakeLocationHerdr {
            pane_sessions: [("wE:p1".to_owned(), "old-sid".to_owned())]
                .into_iter()
                .collect(),
            ..FakeLocationHerdr::default()
        };
        let resolved =
            resolve_name(&mut r, &herdr, name("qsl"), "node-a", Some("new-sid")).unwrap();
        assert_eq!(resolved, name("qsl-2"));
    }

    #[test]
    fn a_third_simultaneous_collision_gets_the_next_free_number() {
        let Some(mut r) = relay("same-machine-live-collision-3") else {
            return;
        };
        register_at(&mut r, "qsl", "node-a", Some("old-sid"), "wE:p1");
        register_at(&mut r, "qsl-2", "node-a", Some("other-sid"), "wE:p2");
        let herdr = FakeLocationHerdr {
            pane_sessions: [("wE:p1".to_owned(), "old-sid".to_owned())]
                .into_iter()
                .collect(),
            ..FakeLocationHerdr::default()
        };
        let resolved =
            resolve_name(&mut r, &herdr, name("qsl"), "node-a", Some("new-sid")).unwrap();
        assert_eq!(resolved, name("qsl-3"));
    }

    #[test]
    fn different_machine_and_different_session_widens_with_the_machine_suffix() {
        let Some(mut r) = relay("cross-machine") else {
            return;
        };
        register(&mut r, "qsl", "hub", Some("hub-sid"));
        let resolved = resolve_name(
            &mut r,
            &no_herdr(),
            name("qsl"),
            "node-a",
            Some("node-a-sid"),
        )
        .unwrap();
        assert_eq!(resolved, name("qsl-node-a"));
    }

    #[test]
    fn different_machine_but_same_session_id_keeps_the_candidate() {
        // Session ids are host-local in practice, but the rule as specified
        // requires *both* a different machine and a different session
        // before widening — this is the defensive edge the ticket calls out.
        let Some(mut r) = relay("cross-machine-same-session") else {
            return;
        };
        register(&mut r, "qsl", "hub", Some("shared-sid"));
        let resolved = resolve_name(
            &mut r,
            &no_herdr(),
            name("qsl"),
            "node-a",
            Some("shared-sid"),
        )
        .unwrap();
        assert_eq!(resolved, name("qsl"));
    }

    #[test]
    fn a_candidate_already_at_the_length_limit_still_gets_a_distinct_widened_name() {
        let Some(mut r) = relay("limit-widen") else {
            return;
        };
        let full = relay_core::naming::slugify(&"q".repeat(40));
        assert_eq!(full.as_str().len(), 32);
        r.register(&relay_core::Registration {
            name: full.clone(),
            machine: "hub".parse().unwrap(),
            kind: "claude".into(),
            session_id: Some("hub-sid".parse().unwrap()),
            cwd: None,
            herdr_pane: Some("wE:p1".parse().unwrap()),
            place: None,
            last_seen_ms: 1,
        })
        .unwrap();
        let widened = resolve_name(
            &mut r,
            &no_herdr(),
            full.clone(),
            "node-a",
            Some("node-a-sid"),
        )
        .unwrap();
        assert_ne!(
            widened, full,
            "truncation must not hand back the colliding name"
        );
        assert!(widened.as_str().ends_with("-node-a"), "{widened}");
        assert!(widened.as_str().len() <= 32);
    }

    #[test]
    fn a_taken_widened_name_falls_through_to_a_numbered_one() {
        let Some(mut r) = relay("widened-taken") else {
            return;
        };
        register(&mut r, "qsl", "hub", Some("hub-sid"));
        register(&mut r, "qsl-node-a", "node-b", Some("node-b-sid"));
        let resolved = resolve_name(
            &mut r,
            &no_herdr(),
            name("qsl"),
            "node-a",
            Some("node-a-sid"),
        )
        .unwrap();
        assert_eq!(resolved, name("qsl-2"));
    }

    #[test]
    fn re_deriving_keeps_the_numbered_name_this_session_already_holds() {
        let Some(mut r) = relay("numbered-stable") else {
            return;
        };
        register_at(&mut r, "qsl", "node-a", Some("old-sid"), "wE:p1");
        register_at(&mut r, "qsl-2", "node-a", Some("new-sid"), "wE:p2");
        let herdr = FakeLocationHerdr {
            pane_sessions: [("wE:p1".to_owned(), "old-sid".to_owned())]
                .into_iter()
                .collect(),
            ..FakeLocationHerdr::default()
        };
        let resolved =
            resolve_name(&mut r, &herdr, name("qsl"), "node-a", Some("new-sid")).unwrap();
        assert_eq!(resolved, name("qsl-2"), "not qsl-3: qsl-2 is already ours");
    }

    #[test]
    fn two_unknown_session_ids_are_not_the_same_caller() {
        let Some(mut r) = relay("none-none") else {
            return;
        };
        register(&mut r, "qsl", "hub", None);
        let resolved = resolve_name(&mut r, &no_herdr(), name("qsl"), "node-a", None).unwrap();
        assert_eq!(
            resolved,
            name("qsl-node-a"),
            "None == None must not mean 'ours'"
        );
    }

    #[test]
    fn without_herdr_a_registered_same_machine_session_is_assumed_live() {
        let Some(mut r) = relay("assume-hosted") else {
            return;
        };
        register_at(&mut r, "qsl", "node-a", Some("old-sid"), "wE:p1");
        let resolved = resolve_name(
            &mut r,
            &AssumeHosted,
            name("qsl"),
            "node-a",
            Some("new-sid"),
        )
        .unwrap();
        assert_eq!(resolved, name("qsl-2"));
    }

    #[test]
    fn a_same_machine_registration_with_no_session_or_pane_is_treated_as_live() {
        let Some(mut r) = relay("unknown-is-live") else {
            return;
        };
        // No session id (as `agent-msg register` makes it).
        r.register(&relay_core::Registration {
            name: name("qsl"),
            machine: "node-a".parse().unwrap(),
            kind: "claude".into(),
            session_id: None,
            cwd: None,
            herdr_pane: None,
            place: None,
            last_seen_ms: 1,
        })
        .unwrap();
        let with_herdr =
            resolve_name(&mut r, &no_herdr(), name("qsl"), "node-a", Some("new-sid")).unwrap();
        let without = resolve_name(
            &mut r,
            &AssumeHosted,
            name("qsl"),
            "node-a",
            Some("new-sid"),
        )
        .unwrap();
        assert_eq!(
            with_herdr,
            name("qsl-2"),
            "unknown means do not take the name"
        );
        assert_eq!(without, name("qsl-2"));
    }

    /// A liveness check that always fails, like a herdr that cannot be run.
    struct BrokenHerdr;

    impl PaneLiveness for BrokenHerdr {
        fn hosts(&self, _pane: &PaneId, _session_id: &SessionId) -> Result<bool, ProcessError> {
            Err(ProcessError::Reported {
                code: "boom".into(),
                message: "herdr is down".into(),
            })
        }
    }

    /// Every pane still hosts its session.
    struct StillHosted;

    impl PaneLiveness for StillHosted {
        fn hosts(&self, _pane: &PaneId, _session_id: &SessionId) -> Result<bool, ProcessError> {
            Ok(true)
        }
    }

    /// Tracing: TC-022-06
    /// ACs: FR-022-AC-6
    #[test]
    fn tc_022_06_every_variant_taken_is_no_free_name_not_the_candidate() {
        for holder_machine in ["node-a", "hub"] {
            let Some(mut r) = relay(&format!("no-free-{holder_machine}")) else {
                return;
            };
            register(&mut r, "qsl", holder_machine, Some("holder"));
            register(&mut r, "qsl-node-a", "hub", Some("other-holder"));
            for n in 2..=relay_core::naming::MAX_VARIANT_NUMBER {
                register(&mut r, &format!("qsl-{n}"), "hub", Some("other-holder"));
            }
            let err = resolve_name(&mut r, &StillHosted, name("qsl"), "node-a", Some("me"))
                .expect_err("no name is free");
            assert!(
                matches!(&err, CollisionError::NoFreeName { candidate } if *candidate == name("qsl")),
                "{holder_machine}: {err:?}"
            );
            let holder = r.agent(&name("qsl")).unwrap().unwrap();
            assert_eq!(
                holder.session_id.as_ref().map(|v| v.as_str()),
                Some("holder")
            );
        }
    }

    /// Tracing: TC-022-07
    /// ACs: FR-022-AC-7
    ///
    /// A herdr error while checking the holder treats it as live: the next free
    /// number is chosen, the error is handed back, the holder is untouched.
    #[test]
    fn tc_022_07_a_liveness_error_treats_the_holder_as_live() {
        let Some(mut r) = relay("liveness-error") else {
            return;
        };
        register(&mut r, "qsl", "node-a", Some("holder"));
        let resolved = resolve_collision(
            &mut r,
            &BrokenHerdr,
            name("qsl"),
            &"node-a".parse().unwrap(),
            Some(&"me".parse().unwrap()),
        )
        .unwrap();
        assert_eq!(resolved.name, name("qsl-2"));
        assert!(
            matches!(&resolved.liveness_error, Some(ProcessError::Reported { code, .. }) if code == "boom"),
            "{:?}",
            resolved.liveness_error
        );
        let holder = r.agent(&name("qsl")).unwrap().unwrap();
        assert_eq!(
            holder.session_id.as_ref().map(|v| v.as_str()),
            Some("holder")
        );
    }

    #[test]
    fn a_liveness_error_is_not_reported_when_no_check_was_needed() {
        let Some(mut r) = relay("no-check-needed") else {
            return;
        };
        let resolved = resolve_collision(
            &mut r,
            &BrokenHerdr,
            name("free"),
            &"node-a".parse().unwrap(),
            Some(&"me".parse().unwrap()),
        )
        .unwrap();
        assert!(resolved.liveness_error.is_none());
    }

    /// Answers each `herdr` call from a fixed table keyed by its first two args.
    struct TableRunner(Vec<(&'static str, &'static str)>);

    impl ProcessRunner for TableRunner {
        fn run(
            &self,
            _program: &str,
            args: &[&str],
            _timeout: Duration,
        ) -> Result<herdr_cli::Output, ProcessError> {
            let key = format!("{} {}", args[0], args[1]);
            let body = self
                .0
                .iter()
                .find(|(k, _)| *k == key)
                .map_or("{}", |(_, b)| *b);
            Ok(herdr_cli::Output {
                status_code: 0,
                success: true,
                stdout: body.as_bytes().to_vec(),
                stderr: Vec::new(),
            })
        }
    }

    fn real(tabs: &'static str) -> RealLocationHerdr<TableRunner> {
        RealLocationHerdr {
            runner: TableRunner(vec![
                (
                    "workspace list",
                    r#"{"result":{"workspaces":[{"workspace_id":"w1","label":"QSL"}]}}"#,
                ),
                ("tab list", tabs),
                (
                    "pane list",
                    r#"{"result":{"panes":[{"pane_id":"w1:p2","tab_id":"w1:t2"}]}}"#,
                ),
                (
                    "agent get",
                    r#"{"result":{"agent":{"agent_session":{"kind":"id","value":"s"}}}}"#,
                ),
            ]),
            timeout: Duration::from_secs(1),
        }
    }

    fn second_tab_env() -> HerdrEnv {
        HerdrEnv {
            pane_id: "w1:p2".parse().unwrap(),
            tab_id: "w1:t2".parse().unwrap(),
            workspace_id: "w1".parse().unwrap(),
        }
    }

    /// Name derivation needs every tab: one that does not decode (here, no
    /// `number`) must fail the listing, not vanish, or the lead in the second tab
    /// would derive the first tab's name and an address without its tab. As on
    /// `main`, nothing is derived (AGE-2027 second-round review, finding 1).
    #[test]
    fn a_tab_list_with_one_undecodable_tab_derives_nothing() {
        let herdr = real(
            r#"{"result":{"tabs":[{"tab_id":"w1:t1","label":"1"},{"tab_id":"w1:t2","label":"2","number":2}]}}"#,
        );
        assert!(derive_from_herdr(&herdr, &second_tab_env()).is_err());
        // The same listing, whole, derives the second tab's own name.
        let whole = real(
            r#"{"result":{"tabs":[{"tab_id":"w1:t1","label":"1","number":1},{"tab_id":"w1:t2","label":"2","number":2}]}}"#,
        );
        let loc = derive_from_herdr(&whole, &second_tab_env()).unwrap();
        assert_ne!(loc.name.map(|n| n.to_string()), Some("qsl".to_owned()));
    }

    /// A tab id outside its type's rule fails the naming list the same way.
    #[test]
    fn a_tab_list_with_an_empty_tab_id_derives_nothing() {
        let herdr = real(
            r#"{"result":{"tabs":[{"tab_id":"","label":"1","number":1},{"tab_id":"w1:t2","label":"2","number":2}]}}"#,
        );
        assert!(derive_from_herdr(&herdr, &second_tab_env()).is_err());
    }
}
