//! Session reporting against the real Redis lane; never the message inbox.
#![expect(
    clippy::unwrap_used,
    reason = "test assertions fail on unexpected errors"
)]
use relay_core::{
    Error,
    work::{AgentWork, WorkBinding, WorkError, WorkFocus, WorkScope, WorkStatus},
};
use relay_test_support::Bus;

fn binding() -> WorkBinding {
    WorkBinding {
        machine: "luna".parse().unwrap(),
        pane: "w1:p1".parse().unwrap(),
        session: "session-a".parse().unwrap(),
        process_id: 42,
        kind: "codex".into(),
        address: "luna:SWM:code".into(),
    }
}
fn agent(id: &str) -> AgentWork {
    AgentWork {
        agent_id: id.into(),
        name: id.into(),
        ticket: "SWM-12".parse().unwrap(),
        role: "coder".into(),
        status: WorkStatus::Running,
        pane: None,
        session: None,
        reported_at_ms: 100,
    }
}

/// Trace: FR-048-AC-1, FR-048-AC-2
#[test]
fn scope_focus_and_concurrent_agents_survive_independent_updates_and_are_session_isolated() {
    let Some(mut bus) = Bus::connect("work-independent") else {
        return;
    };
    let owner = binding();
    bus.relay
        .report_work_scope(
            &owner,
            &WorkScope {
                projects: vec!["project-id".into()],
                reported_at_ms: 100,
                ..Default::default()
            },
        )
        .unwrap();
    let handles: Vec<_> = ["coder", "reviewer"]
        .into_iter()
        .map(|id| {
            let mut relay = bus.another_connection();
            let owner = owner.clone();
            std::thread::spawn(move || relay.report_work_agent(&owner, &agent(id)).unwrap())
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
    bus.relay
        .report_work_focus(
            &owner,
            &WorkFocus {
                summary: "Review credentials".into(),
                tickets: vec!["SWM-12".parse().unwrap()],
                reported_at_ms: 101,
            },
        )
        .unwrap();
    let got = bus.relay.work_report(&owner).unwrap().unwrap();
    assert_eq!(got.scope.unwrap().projects, ["project-id"]);
    assert_eq!(got.focus.unwrap().summary, "Review credentials");
    assert_eq!(
        got.agents
            .iter()
            .map(|a| a.agent_id.as_str())
            .collect::<Vec<_>>(),
        ["coder", "reviewer"]
    );
    for changed in [
        WorkBinding {
            session: "session-b".parse().unwrap(),
            ..owner.clone()
        },
        WorkBinding {
            process_id: 43,
            ..owner.clone()
        },
        WorkBinding {
            machine: "hub".parse().unwrap(),
            ..owner.clone()
        },
    ] {
        assert_eq!(bus.relay.work_report(&changed).unwrap(), None);
    }
    let renamed = WorkBinding {
        address: "luna:SWM:renamed".into(),
        ..owner.clone()
    };
    bus.relay
        .report_work_agent(&renamed, &agent("reviewer"))
        .unwrap();
    assert_eq!(
        bus.relay
            .work_report(&renamed)
            .unwrap()
            .unwrap()
            .agents
            .len(),
        2
    );
    let wrong_pane = WorkBinding {
        pane: "w1:p2".parse().unwrap(),
        ..owner.clone()
    };
    assert!(matches!(
        bus.relay.report_work_agent(&wrong_pane, &agent("bad")),
        Err(Error::Work(WorkError::IdentityMismatch))
    ));
    assert!(matches!(
        bus.relay.remove_work_agent(&wrong_pane, "coder"),
        Err(Error::Work(WorkError::IdentityMismatch))
    ));
    assert_eq!(
        bus.relay.work_report(&owner).unwrap().unwrap().agents.len(),
        2
    );
    bus.relay.remove_work_agent(&owner, "coder").unwrap();
    assert_eq!(
        bus.relay.work_report(&owner).unwrap().unwrap().agents[0].agent_id,
        "reviewer"
    );
    assert!(
        bus.keys().iter().all(|k| k.contains(":work:")),
        "reporting never registers agents or writes inboxes"
    );
}

/// Trace: FR-048-AC-2
#[test]
fn report_validation_and_read_bounds_refuse_unsafe_or_oversized_data() {
    let Some(mut bus) = Bus::connect("work-bounds") else {
        return;
    };
    let owner = binding();
    let bad = AgentWork {
        name: "name\nforged".into(),
        ..agent("bad")
    };
    assert!(matches!(
        bus.relay.report_work_agent(&owner, &bad),
        Err(Error::Work(WorkError::InvalidField { field: "name" }))
    ));
    assert_eq!(bus.relay.work_report(&owner).unwrap(), None);
    for n in 0..128 {
        bus.relay
            .report_work_agent(&owner, &agent(&n.to_string()))
            .unwrap();
    }
    assert!(matches!(
        bus.relay
            .report_work_agent(&owner, &agent("early-overflow")),
        Err(Error::Work(WorkError::AgentLimit))
    ));
    // Reserved fields remain writable even when all 128 agent slots are used.
    bus.relay
        .report_work_scope(
            &owner,
            &WorkScope {
                projects: vec!["project-id".into()],
                ..Default::default()
            },
        )
        .unwrap();
    bus.relay
        .report_work_focus(
            &owner,
            &WorkFocus {
                summary: "Working".into(),
                tickets: vec![],
                reported_at_ms: 100,
            },
        )
        .unwrap();
    assert!(matches!(
        bus.relay.report_work_agent(&owner, &agent("overflow")),
        Err(Error::Work(WorkError::AgentLimit))
    ));
    let key = "work:luna:session-a:42";
    bus.hset_raw(key, "focus", &"x".repeat(65_537));
    assert!(matches!(
        bus.relay.work_report(&owner),
        Err(Error::Work(WorkError::ReportLimit))
    ));
}

/// Trace: FR-048-AC-9
#[test]
fn whole_team_report_clears_narrow_selectors_without_erasing_agents() {
    let Some(mut bus) = Bus::connect("work-whole-team") else {
        return;
    };
    let owner = binding();
    bus.relay
        .report_work_scope(
            &owner,
            &WorkScope {
                projects: vec!["project".into()],
                ..Default::default()
            },
        )
        .unwrap();
    bus.relay
        .report_work_agent(&owner, &agent("coder"))
        .unwrap();
    bus.relay
        .report_work_scope(
            &owner,
            &WorkScope {
                reported_at_ms: 123,
                ..Default::default()
            },
        )
        .unwrap();
    let report = bus.relay.work_report(&owner).unwrap().unwrap();
    assert_eq!(
        report.scope,
        Some(WorkScope {
            reported_at_ms: 123,
            ..Default::default()
        })
    );
    assert_eq!(report.agents.len(), 1);
    assert_eq!(report.agents[0].agent_id, "coder");
}
