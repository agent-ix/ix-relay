//! Session reporting against the real Redis lane; never the message inbox.
#![expect(
    clippy::unwrap_used,
    reason = "test assertions fail on unexpected errors"
)]
use ix_trace_rs::trace;
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
        bus.keys().iter().all(|k| k.contains(":work:")
            || k == &bus.key("work-events")
            || k == &bus.key("work-events-start")
            || k.contains(":work-activity:")),
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

/// Provenance: SWM-180; atomic history preserves short stages and removals.
#[trace("FR-066-AC-1", "FR-066-AC-2", "FR-066-AC-3")]
#[test]
fn work_history_is_ordered_read_only_private_and_session_qualified() {
    let Some(mut bus) = Bus::connect("work-history") else {
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
    let mut assignment = agent("child-a");
    bus.relay.report_work_agent(&owner, &assignment).unwrap();
    assignment.role = "reviewer".into();
    bus.relay.report_work_agent(&owner, &assignment).unwrap();
    bus.relay.report_work_activity(&owner, true).unwrap();
    bus.relay.remove_work_agent(&owner, "child-a").unwrap();
    bus.relay
        .report_work_focus(
            &owner,
            &WorkFocus {
                summary: "private focus text".into(),
                tickets: vec![],
                reported_at_ms: 101,
            },
        )
        .unwrap();
    let first = bus.relay.work_events("0-0", 2).unwrap();
    assert_eq!(first.events.len(), 2);
    assert!(first.has_more);
    assert_eq!(first.version, 1);
    assert!(first.available_from_ms.is_some());
    let second = bus.relay.work_events(&first.next_cursor, 100).unwrap();
    assert_eq!(second.events.len(), 3);
    assert!(!second.has_more);
    assert_eq!(
        bus.relay.work_events("0-0", 2).unwrap().events,
        first.events
    );
    assert!(
        first
            .events
            .iter()
            .chain(&second.events)
            .all(|e| e.binding == relay_core::work_history::Binding::from(&owner))
    );
    assert!(
        matches!(&second.events[0].change,relay_core::work_history::WorkChange::Agent{agent} if agent.role=="reviewer")
    );
    assert!(matches!(
        second.events[2].change,
        relay_core::work_history::WorkChange::Remove { .. }
    ));
    assert!(
        !serde_json::to_string(&second)
            .unwrap()
            .contains("private focus text")
    );
    assert!(
        bus.relay
            .work_report(&owner)
            .unwrap()
            .unwrap()
            .agents
            .is_empty()
    );
    let snapshot = bus.relay.work_snapshot(&owner, 1000).unwrap();
    assert_eq!(snapshot.paused, Some(true));
    assert_eq!(
        snapshot.binding,
        relay_core::work_history::Binding::from(&owner)
    );
    assert!(
        !serde_json::to_string(&snapshot)
            .unwrap()
            .contains("private focus text")
    );
    assert!(bus.relay.work_events("bad", 10).is_err());
    assert!(bus.relay.work_events("0-0", 1001).is_err());
}

/// Provenance: SWM-180; invalid history storage cannot partially update work state.
#[trace("FR-066-AC-1", "FR-066-AC-4")]
#[test]
fn invalid_history_key_refuses_state_mutation_and_count_retention_is_explicit() {
    let Some(mut bus) = Bus::connect("work-history-bounds") else {
        return;
    };
    let owner = binding();
    redis::cmd("SET")
        .arg(bus.key("work-events"))
        .arg("wrong-type")
        .query::<()>(&mut bus.raw)
        .unwrap();
    assert!(bus.relay.report_work_agent(&owner, &agent("a")).is_err());
    assert!(bus.relay.work_report(&owner).unwrap().is_none());
    redis::cmd("DEL")
        .arg(bus.key("work-events"))
        .query::<()>(&mut bus.raw)
        .unwrap();
    for _ in 0..4100 {
        bus.relay.report_work_activity(&owner, false).unwrap();
    }
    let count: usize = redis::cmd("XLEN")
        .arg(bus.key("work-events"))
        .query(&mut bus.raw)
        .unwrap();
    assert_eq!(count, 4096);
    let page = bus.relay.work_events("1-0", 1).unwrap();
    assert!(page.truncated);
    assert!(page.available_from_ms.is_some());
}

#[trace("FR-066-AC-2", "FR-066-AC-4")]
#[test]
fn idle_history_excludes_expired_events_and_advances_empty_cursors_without_writes() {
    let Some(mut bus) = Bus::connect("work-history-idle") else {
        return;
    };
    let owner = binding();
    // Seed relative to the server clock, without waiting a week or publishing a new event.
    let seeded: Vec<String> = redis::Script::new(
        r#"
local t = redis.call('TIME')
local now = tonumber(t[1]) * 1000 + math.floor(tonumber(t[2]) / 1000)
local times = {now - 691200000, now - 604740000, now - 1000}
local ids = {}
for _, at in ipairs(times) do
  local ms = string.format('%.0f', at)
  ids[#ids + 1] = redis.call('XADD', KEYS[1], ms .. '-0',
    'at_ms', ms, 'binding', ARGV[1], 'change', ARGV[2])
end
redis.call('SET', KEYS[2], string.format('%.0f', times[1]))
return ids
"#,
    )
    .key(bus.key("work-events"))
    .key(bus.key("work-events-start"))
    .arg(serde_json::to_string(&owner).unwrap())
    .arg(
        serde_json::to_string(&relay_core::work_history::WorkChange::Activity { paused: true })
            .unwrap(),
    )
    .invoke(&mut bus.raw)
    .unwrap();
    let key = bus.key("work-events");
    let before: Vec<u8> = relay_test_support::canonical_dump(&mut bus.raw, &key);
    let first = bus.relay.work_events(&seeded[0], 1).unwrap();
    assert!(first.truncated);
    assert!(first.has_more);
    assert_eq!(first.events[0].id, seeded[1]);
    assert_eq!(first.available_from_ms, Some(first.events[0].at_ms));
    assert!(first.events[0].at_ms >= first.observed_at_ms - 604800000);
    let bootstrap = bus.relay.work_events("0-0", 10).unwrap();
    assert!(!bootstrap.truncated);
    assert_eq!(bootstrap.events.len(), 2);
    assert_eq!(bootstrap.events[0].id, seeded[1]);
    assert_eq!(
        bus.relay.work_events(&seeded[0], 1).unwrap().events,
        first.events
    );
    let second = bus.relay.work_events(&first.next_cursor, 1).unwrap();
    assert!(!second.truncated);
    assert!(!second.has_more);
    assert_eq!(second.events[0].id, seeded[2]);
    let end = bus.relay.work_events(&second.next_cursor, 1).unwrap();
    assert!(end.events.is_empty());
    assert!(!end.has_more);
    assert_eq!(end.next_cursor, second.next_cursor);
    let key = bus.key("work-events");
    let after: Vec<u8> = relay_test_support::canonical_dump(&mut bus.raw, &key);
    assert_eq!(
        before, after,
        "history reads do not trim or acknowledge entries"
    );
    // Leave only the expired row: empty coverage still reports the age floor and progresses.
    redis::cmd("XDEL")
        .arg(bus.key("work-events"))
        .arg(&seeded[1..])
        .query::<usize>(&mut bus.raw)
        .unwrap();
    let empty = bus.relay.work_events(&seeded[0], 1).unwrap();
    assert!(empty.events.is_empty());
    assert!(!empty.has_more);
    assert!(empty.truncated);
    assert_eq!(
        empty.available_from_ms,
        Some(empty.observed_at_ms - 604800000)
    );
    assert_ne!(empty.next_cursor, seeded[0]);
    let again = bus.relay.work_events(&empty.next_cursor, 1).unwrap();
    assert!(again.events.is_empty());
    assert!(!again.has_more);
    assert!(bus.relay.work_report(&owner).unwrap().is_none());
    // A retained stream ID cannot smuggle an expired payload timestamp into coverage.
    redis::cmd("XADD")
        .arg(bus.key("work-events"))
        .arg("*")
        .arg("at_ms")
        .arg(seeded[0].split_once('-').unwrap().0)
        .arg("binding")
        .arg(serde_json::to_string(&owner).unwrap())
        .arg("change")
        .arg(
            serde_json::to_string(&relay_core::work_history::WorkChange::Activity { paused: true })
                .unwrap(),
        )
        .query::<String>(&mut bus.raw)
        .unwrap();
    assert!(matches!(
        bus.relay.work_events("0-0", 1),
        Err(Error::Work(WorkError::InvalidField {
            field: "history timestamp"
        }))
    ));
}

#[trace("FR-066-AC-3")]
#[test]
fn replacement_sessions_and_processes_keep_separate_history_and_current_state() {
    let Some(mut bus) = Bus::connect("work-history-incarnations") else {
        return;
    };
    let original = binding();
    let replacement = WorkBinding {
        session: "session-b".parse().unwrap(),
        ..original.clone()
    };
    let restarted = WorkBinding {
        process_id: 43,
        ..replacement.clone()
    };
    for (owner, id) in [
        (&original, "original"),
        (&replacement, "replacement"),
        (&restarted, "restarted"),
    ] {
        bus.relay.report_work_agent(owner, &agent(id)).unwrap();
    }
    bus.relay
        .remove_work_agent(&replacement, "replacement")
        .unwrap();
    let page = bus.relay.work_events("0-0", 10).unwrap();
    assert_eq!(page.events.len(), 4);
    for (event, owner) in
        page.events
            .iter()
            .zip([&original, &replacement, &restarted, &replacement])
    {
        assert_eq!(
            event.binding,
            relay_core::work_history::Binding::from(owner)
        );
    }
    for (owner, expected) in [
        (&original, vec!["original"]),
        (&replacement, vec![]),
        (&restarted, vec!["restarted"]),
    ] {
        let snapshot = bus.relay.work_snapshot(owner, page.observed_at_ms).unwrap();
        assert_eq!(
            snapshot.binding,
            relay_core::work_history::Binding::from(owner)
        );
        assert_eq!(
            bus.relay
                .work_report(owner)
                .unwrap()
                .unwrap()
                .agents
                .iter()
                .map(|a| a.agent_id.as_str())
                .collect::<Vec<_>>(),
            expected
        );
    }
    assert_eq!(
        bus.relay.work_events("0-0", 10).unwrap().events,
        page.events
    );
}
