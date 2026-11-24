//! This host's lanes (FR-069): every herdr pane that runs an agent plus every
//! lead registration of the host, keyed by address `server:workspace:tab`, and
//! the lane state series read from what the watcher itself sees.

use std::{collections::BTreeMap, time::Instant};

use relay_core::{Address, MachineName, PaneId, Registration};
use relay_ops::find::{MachineView, join};

use super::pane::{BlockedReason, Harness, PaneClass};

/// The most lanes exported per push; more are counted in `lane.dropped`.
pub const MAX_LANES: usize = 50;

/// One lane of this host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lane {
    /// `server:workspace:tab`, the join key of every lane series.
    pub address: String,
    /// The pane to read its tail from, when it has one.
    pub pane: Option<PaneId>,
    /// The harness it runs.
    pub harness: Harness,
    /// herdr's status for the pane, when a pane was seen.
    pub herdr_status: Option<String>,
}

/// The lane registry of `machine`: the union by address of the agent panes in
/// `view` and the registrations of `machine` that have a place. Sorted by
/// address; a pane wins over a registration of the same address.
#[must_use]
pub fn lanes_of(machine: &MachineName, view: &MachineView, regs: &[Registration]) -> Vec<Lane> {
    let mut by_address: BTreeMap<String, Lane> = BTreeMap::new();
    let kinds: BTreeMap<&PaneId, &str> = view
        .agents
        .iter()
        .map(|a| (&a.pane_id, a.kind.as_str()))
        .collect();
    for row in join(std::slice::from_ref(view), regs) {
        if row.machine != *machine {
            continue;
        }
        let address = match (&row.workspace_label, &row.tab_label) {
            (Some(ws), Some(tab)) => Address::new(machine.as_str(), ws, Some(tab)).to_string(),
            _ => format!("{machine}:{}:{}", row.workspace_id, row.tab_id),
        };
        let kind = kinds.get(&row.pane_id).copied().unwrap_or("");
        by_address.entry(address.clone()).or_insert(Lane {
            address,
            harness: Harness::from_kind(kind),
            pane: Some(row.pane_id),
            herdr_status: Some(row.herdr_status),
        });
    }
    for reg in regs.iter().filter(|r| r.machine == *machine) {
        let Some(place) = &reg.place else { continue };
        let address = place.tab_address(machine).to_string();
        by_address.entry(address.clone()).or_insert(Lane {
            address,
            harness: Harness::from_kind(&reg.kind),
            pane: reg.herdr_pane.clone(),
            herdr_status: None,
        });
    }
    by_address.into_values().collect()
}

/// The `state` of a lane (FR-045); there is no `unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaneState {
    /// Recent prompt or tool activity, an open tool call, a compiler, or herdr working.
    Working,
    /// None of the others, including a lane with no event.
    Idle,
    /// A blocked event, a question stop or a pane class that maps to a reason.
    Blocked,
    /// A `pre_tool` open past `max_tool_runtime`.
    Stuck,
    /// The session ended.
    Ended,
}

impl LaneState {
    /// The `state` attribute value.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            LaneState::Working => "working",
            LaneState::Idle => "idle",
            LaneState::Blocked => "blocked",
            LaneState::Stuck => "stuck",
            LaneState::Ended => "ended",
        }
    }
}

/// What the newest hook event says of a lane at the moment of a push (FR-045).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookSignal {
    /// Blocked on a person, with this reason.
    Blocked(BlockedReason),
    /// Idle: an `idle` or `stop` event, or activity older than the working window.
    Idle,
    /// A recent prompt or post-tool event, or an open tool call within its runtime.
    Working,
    /// A tool call open past `max_tool_runtime`.
    Stuck,
    /// A `session_end` event.
    Ended,
}

/// What the collector knows of a lane beyond the registry.
#[derive(Debug, Clone, Copy, Default)]
pub struct Signals {
    /// The class of the last pane tail and when it was read.
    pub pane: Option<(PaneClass, Instant)>,
    /// The newest hook event as a verdict and when it happened.
    pub hook: Option<(HookSignal, Instant)>,
}

impl Signals {
    /// The state and blocked reason of a lane. The newest evidence wins: a pane
    /// class read after the newest hook event decides alone (a class that maps to
    /// no reason hands the lane to herdr's status), otherwise the hook event's
    /// verdict stands, and a lane with no hook event is read from its pane class,
    /// then herdr. A compiler attributed to the lane keeps an idle lane working.
    #[must_use]
    pub fn resolve(
        &self,
        herdr_status: Option<&str>,
        compiling: bool,
    ) -> (LaneState, Option<BlockedReason>) {
        let busy = compiling || herdr_status == Some("working");
        let busy_state = if busy {
            LaneState::Working
        } else {
            LaneState::Idle
        };
        let pane_newer = match (self.pane, self.hook) {
            (Some((_, p)), Some((_, h))) => p > h,
            (Some(_), None) => true,
            _ => false,
        };
        if pane_newer {
            return match self.pane.and_then(|(class, _)| class.reason()) {
                Some(r) => (LaneState::Blocked, Some(r)),
                None => (busy_state, None),
            };
        }
        match self.hook {
            Some((HookSignal::Blocked(r), _)) => (LaneState::Blocked, Some(r)),
            Some((HookSignal::Ended, _)) => (LaneState::Ended, None),
            Some((HookSignal::Stuck, _)) => (LaneState::Stuck, None),
            Some((HookSignal::Working, _)) => (LaneState::Working, None),
            Some((HookSignal::Idle, _)) => (
                if compiling {
                    LaneState::Working
                } else {
                    LaneState::Idle
                },
                None,
            ),
            None => match self.pane.and_then(|(class, _)| class.reason()) {
                Some(r) => (LaneState::Blocked, Some(r)),
                None => (busy_state, None),
            },
        }
    }

    /// The blocked reason in force, if any.
    #[cfg(test)]
    #[must_use]
    pub fn blocked_reason(&self) -> Option<BlockedReason> {
        self.resolve(None, false).1
    }

    /// The state of the lane.
    #[cfg(test)]
    #[must_use]
    pub fn state(&self, herdr_status: Option<&str>, compiling: bool) -> LaneState {
        self.resolve(herdr_status, compiling).0
    }
}

#[cfg(test)]
mod tests {
    use ix_trace_rs::trace;
    use relay_ops::find::{HerdrAgent, HerdrTab, HerdrWorkspace};

    use super::*;

    fn view() -> MachineView {
        let agent = |pane: &str, tab: &str, kind: &str| HerdrAgent {
            kind: kind.into(),
            pane_id: pane.parse().unwrap(),
            tab_id: tab.parse().unwrap(),
            workspace_id: "w1".parse().unwrap(),
            status: "working".into(),
            session_name: None,
            session_id: None,
        };
        MachineView {
            machine: "hub".parse().unwrap(),
            agents: vec![
                agent("w1:p1", "w1:t1", "claude"),
                agent("w1:p2", "w1:t2", "codex"),
            ],
            workspaces: vec![HerdrWorkspace {
                workspace_id: "w1".parse().unwrap(),
                label: "QSL".into(),
                tab_count: 2,
            }],
            tabs: vec![
                HerdrTab {
                    tab_id: "w1:t1".parse().unwrap(),
                    label: "plan".into(),
                },
                HerdrTab {
                    tab_id: "w1:t2".parse().unwrap(),
                    label: "code".into(),
                },
            ],
            dropped: 0,
        }
    }

    fn reg(name: &str, machine: &str, workspace: &str, tab: &str) -> Registration {
        Registration {
            name: name.parse().unwrap(),
            machine: machine.parse().unwrap(),
            kind: "claude".into(),
            session_id: None,
            cwd: None,
            herdr_pane: None,
            place: Some(relay_core::Place {
                workspace: workspace.into(),
                tab: tab.into(),
                herdr_ids: None,
            }),
            last_seen_ms: 0,
        }
    }

    #[trace("FR-069-AC-11")]
    #[test]
    fn tc_069_11_the_registry_is_the_union_of_panes_and_registrations_by_address() {
        let hub: MachineName = "hub".parse().unwrap();
        let regs = [
            reg("qsl-plan", "hub", "QSL", "plan"), // same address as a pane
            reg("only-reg", "hub", "EA", "review"), // registration with no pane
            reg("elsewhere", "other", "QSL", "x"), // another host
        ];
        let lanes = lanes_of(&hub, &view(), &regs);
        let addresses: Vec<&str> = lanes.iter().map(|l| l.address.as_str()).collect();
        assert_eq!(addresses, ["hub:EA:review", "hub:QSL:code", "hub:QSL:plan"]);
        let code = lanes.iter().find(|l| l.address == "hub:QSL:code").unwrap();
        assert_eq!(code.harness, Harness::Codex);
        assert!(code.pane.is_some());
        let review = lanes.iter().find(|l| l.address == "hub:EA:review").unwrap();
        assert!(review.pane.is_none() && review.herdr_status.is_none());
    }

    #[trace("FR-069-AC-8")]
    #[test]
    fn tc_069_08_a_hook_event_newer_than_the_pane_tail_takes_precedence() {
        let t0 = Instant::now();
        let later = t0 + std::time::Duration::from_secs(5);
        // A dialog read at t0, an idle event at t0+5: the hook state (not blocked) wins.
        let s = Signals {
            pane: Some((PaneClass::Dialog, t0)),
            hook: Some((HookSignal::Idle, later)),
        };
        assert_eq!(s.blocked_reason(), None);
        assert_eq!(s.state(Some("working"), false), LaneState::Idle);
        // A pane tail newer than the hook event is used.
        let s = Signals {
            pane: Some((PaneClass::Dialog, later)),
            hook: Some((HookSignal::Idle, t0)),
        };
        assert_eq!(s.blocked_reason(), Some(BlockedReason::Dialog));
        assert_eq!(s.state(None, false), LaneState::Blocked);
        // A newer blocked event names its own reason.
        let s = Signals {
            pane: Some((PaneClass::Login, t0)),
            hook: Some((HookSignal::Blocked(BlockedReason::Permission), later)),
        };
        assert_eq!(s.blocked_reason(), Some(BlockedReason::Permission));
        assert_eq!(
            Signals::default().state(Some("idle"), false),
            LaneState::Idle
        );
    }
}
