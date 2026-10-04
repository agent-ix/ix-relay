//! `herdr.pane.info` and `herdr.pane.status`: the pane-to-name mapping.
//!
//! Claude Code's OpenTelemetry metrics carry herdr's stable pane, tab and
//! workspace ids; the names live in herdr and the relay registry and change on
//! rename. This module keeps the join of the two for this machine's panes, so a
//! dashboard can show `machine:workspace:tab` and history keeps the name a pane
//! had at the time.
//!
//! The join is [`relay_ops::find::join`], the one `agent-msg find` uses, over the
//! herdr answer the tick's layout refresh already read: no herdr call of its own.

use relay_core::{AgentName, MachineName, PaneId, Registration, SessionId, TabId, WorkspaceId};
use relay_ops::find::{MachineView, join};

use super::{
    registry::{Attrs, Registry},
    series::{PANE_INFO, PANE_STATUS},
};

/// The most panes kept, so a push stays bounded whatever herdr reports.
pub const MAX_PANES: usize = 2_000;

/// One pane's series: the join of herdr's state and the registry, typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneSeries {
    machine: MachineName,
    pane: PaneId,
    tab: TabId,
    workspace: WorkspaceId,
    workspace_label: Option<String>,
    tab_label: Option<String>,
    session: Option<SessionId>,
    bus_name: Option<AgentName>,
    status: String,
}

/// This machine's live panes in `view`, joined with `regs`, one per pane id
/// (herdr may list a pane twice: the first listed is kept), sorted by pane id and
/// cut to [`MAX_PANES`]. Another machine's panes are that machine's watcher's to
/// publish, so a row of another machine is left out, and a registration of another
/// machine is not a pane's name here.
#[must_use]
pub fn panes_of(
    machine: &MachineName,
    view: &MachineView,
    regs: &[Registration],
) -> Vec<PaneSeries> {
    let mut panes: Vec<PaneSeries> = join(std::slice::from_ref(view), regs)
        .into_iter()
        .filter(|row| row.machine == *machine)
        .map(|row| PaneSeries {
            machine: row.machine,
            pane: row.pane_id,
            tab: row.tab_id,
            workspace: row.workspace_id,
            workspace_label: row.workspace_label,
            tab_label: row.tab_label,
            session: row.session_id,
            bus_name: row.bus_name,
            status: row.herdr_status,
        })
        .collect();
    // A stable sort keeps the first listed of a duplicated pane first.
    panes.sort_by(|a, b| a.pane.as_str().cmp(b.pane.as_str()));
    panes.dedup_by(|later, first| later.pane == first.pane);
    panes.truncate(MAX_PANES);
    panes
}

/// Adds `panes` to `registry`: an info series and a status series per pane.
pub fn fill(registry: &mut Registry, panes: &[PaneSeries]) {
    for p in panes {
        let info = Attrs::new()
            .with("machine", p.machine.as_str())
            .with("pane", p.pane.as_str())
            .with("tab", p.tab.as_str())
            .with("workspace", p.workspace.as_str())
            .with(
                "workspace_label",
                p.workspace_label.as_deref().unwrap_or(""),
            )
            .with("tab_label", p.tab_label.as_deref().unwrap_or(""))
            .with("session", p.session.as_ref().map_or("", SessionId::as_str))
            .with(
                "bus_name",
                p.bus_name.as_ref().map_or("", AgentName::as_str),
            );
        registry.gauge(&PANE_INFO, info, 1);
        let status = Attrs::new()
            .with("machine", p.machine.as_str())
            .with("pane", p.pane.as_str())
            .with("status", &p.status);
        registry.gauge(&PANE_STATUS, status, 1);
    }
}

#[cfg(test)]
mod tests {
    use relay_ops::find::{HerdrAgent, HerdrTab, HerdrWorkspace};

    use super::*;
    use crate::otlp::{
        registry::MAX_LABEL_CHARS,
        test_herdr::{self, panes_from},
    };

    fn agent(pane: &str, tab: &str, ws: &str, status: &str, session: Option<&str>) -> HerdrAgent {
        HerdrAgent {
            kind: "unknown".into(),
            pane_id: pane.parse().unwrap(),
            tab_id: tab.parse().unwrap(),
            workspace_id: ws.parse().unwrap(),
            status: status.into(),
            session_name: None,
            session_id: session.map(|s| s.parse().unwrap()),
        }
    }

    fn view(machine: &str, agents: Vec<HerdrAgent>, tab_label: &str) -> MachineView {
        MachineView {
            machine: machine.parse().unwrap(),
            agents,
            workspaces: vec![HerdrWorkspace {
                workspace_id: "w1".parse().unwrap(),
                label: "QSL".into(),
                tab_count: 2,
            }],
            tabs: vec![
                HerdrTab {
                    tab_id: "w1:t1".parse().unwrap(),
                    label: tab_label.into(),
                },
                HerdrTab {
                    tab_id: "w1:t2".parse().unwrap(),
                    label: "docs".into(),
                },
            ],
            dropped: 0,
        }
    }

    fn reg(name: &str, machine: &str, pane: &str) -> Registration {
        Registration {
            name: name.parse().unwrap(),
            machine: machine.parse().unwrap(),
            kind: "claude".into(),
            session_id: None,
            cwd: None,
            herdr_pane: Some(pane.parse().unwrap()),
            place: None,
            last_seen_ms: 0,
        }
    }

    fn hub() -> MachineName {
        "hub".parse().unwrap()
    }

    fn attr<'a>(point: &'a crate::otlp::registry::Point, key: &str) -> &'a str {
        &point
            .attrs
            .pairs()
            .iter()
            .find(|(k, _)| *k == key)
            .unwrap()
            .1
    }

    /// An unregistered pane has an empty `bus_name`, a registration on another
    /// machine with the same pane id is not this pane's name, and a view of
    /// another machine adds no series.
    ///
    /// Tracing: TC-045-04
    /// ACs: FR-045-AC-4
    #[test]
    fn tc_045_04_only_this_machines_panes_are_published_and_named_by_this_machines_leads() {
        let v = view(
            "hub",
            vec![
                agent("w1:p1", "w1:t1", "w1", "idle", Some("sess-1")),
                agent("w1:p2", "w1:t2", "w1", "working", None),
            ],
            "review",
        );
        let mut qsl_lead = reg("qsl-lead", "hub", "w1:p1");
        qsl_lead.session_id = Some("sess-1".parse().unwrap());
        let regs = [qsl_lead, reg("elsewhere", "node-a", "w1:p2")];
        let panes = panes_of(&hub(), &v, &regs);
        assert_eq!(panes.len(), 2);
        assert_eq!(
            panes[0].bus_name.as_ref().map(AgentName::as_str),
            Some("qsl-lead")
        );
        assert_eq!(
            panes[1].bus_name, None,
            "node-a's lead is not hub's pane's name"
        );
        let other = view(
            "node-a",
            vec![agent("w1:p1", "w1:t1", "w1", "idle", None)],
            "t",
        );
        assert!(panes_of(&hub(), &other, &[]).is_empty());
    }

    /// herdr may list one pane twice, even with different sessions: one series per
    /// pane, the first listed, so a join on `(machine, pane)` is one to one.
    ///
    /// Tracing: TC-045-03
    /// ACs: FR-045-AC-3
    #[test]
    fn tc_045_03_a_pane_herdr_lists_twice_is_one_series() {
        let v = view(
            "hub",
            vec![
                agent("w1:p1", "w1:t1", "w1", "idle", Some("first")),
                agent("w1:p1", "w1:t1", "w1", "working", Some("second")),
            ],
            "t",
        );
        let panes = panes_of(&hub(), &v, &[]);
        assert_eq!(panes.len(), 1);
        assert_eq!(
            panes[0].session.as_ref().map(SessionId::as_str),
            Some("first")
        );
    }

    /// Tracing: TC-045-03
    /// ACs: FR-045-AC-3
    #[test]
    fn tc_045_03_the_pane_set_is_bounded() {
        let agents = (0..MAX_PANES + 10)
            .map(|n| agent(&format!("w1:p{n}"), "w1:t1", "w1", "idle", None))
            .collect();
        assert_eq!(
            panes_of(&hub(), &view("hub", agents, "t"), &[]).len(),
            MAX_PANES
        );
    }

    /// The info series carries the eight names in order; the status series only
    /// the join key and the status.
    ///
    /// Tracing: TC-045-03
    /// ACs: FR-045-AC-3
    #[test]
    fn tc_045_03_the_series_carry_their_documented_attributes() {
        let v = view(
            "hub",
            vec![agent("w1:p1", "w1:t1", "w1", "idle", Some("s1"))],
            "review",
        );
        let mut qsl_lead = reg("qsl-lead", "hub", "w1:p1");
        qsl_lead.session_id = Some("s1".parse().unwrap());
        let panes = panes_of(&hub(), &v, &[qsl_lead]);
        let mut r = Registry::new(2, 1);
        fill(&mut r, &panes);
        let keys = |p: &crate::otlp::registry::Point| {
            p.attrs.pairs().iter().map(|(k, _)| *k).collect::<Vec<_>>()
        };
        assert_eq!(
            keys(&r.points()[0]),
            [
                "machine",
                "pane",
                "tab",
                "workspace",
                "workspace_label",
                "tab_label",
                "session",
                "bus_name"
            ]
        );
        assert_eq!(attr(&r.points()[0], "tab_label"), "review");
        assert_eq!(attr(&r.points()[0], "bus_name"), "qsl-lead");
        assert_eq!(keys(&r.points()[1]), ["machine", "pane", "status"]);
        assert_eq!(attr(&r.points()[1], "status"), "idle");
    }

    /// Hostile labels reach the registry neutralised and bounded.
    ///
    /// Tracing: TC-045-03
    /// ACs: FR-045-AC-3
    #[test]
    fn tc_045_03_hostile_herdr_labels_are_neutralised_and_bounded() {
        let mut v = view(
            "hub",
            vec![agent("w1:p1", "w1:t1", "w1", "idle", None)],
            "x",
        );
        v.tabs[0].label = "a\"b\\c\nd\u{1b}[31m\u{202e}e\u{200b}f".into();
        v.workspaces[0].label = "W".repeat(10_240);
        let mut r = Registry::new(2, 1);
        fill(&mut r, &panes_of(&hub(), &v, &[]));
        assert_eq!(
            attr(&r.points()[0], "tab_label"),
            "a\"b\\c\u{FFFD}d\u{FFFD}[31m\u{FFFD}e\u{FFFD}f"
        );
        assert_eq!(
            attr(&r.points()[0], "workspace_label").chars().count(),
            MAX_LABEL_CHARS
        );
    }

    /// Through the watcher's own layout code and herdr's recorded output: after a
    /// tab is renamed in herdr, the next read has the new `tab_label` and no
    /// trace of the old one.
    ///
    /// Tracing: TC-045-05
    /// ACs: FR-045-AC-5
    #[test]
    fn tc_045_05_a_tab_renamed_in_herdr_changes_tab_label_in_the_next_read() {
        let labels = |panes: &[PaneSeries]| {
            panes
                .iter()
                .filter_map(|p| p.tab_label.clone())
                .collect::<Vec<_>>()
        };
        let before = labels(&panes_from(test_herdr::start()));
        assert!(before.iter().any(|l| l == "tabB"), "{before:?}");
        let after = labels(&panes_from(test_herdr::tab_renamed()));
        assert!(after.iter().any(|l| l == "tabB-renamed"), "{after:?}");
        assert!(!after.iter().any(|l| l == "tabB"), "{after:?}");
    }
}
