//! SessionStart and cold-send regressions against real Redis and controlled Herdr topology.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test fixtures")]
mod support;
use relay_core::AgentName;

use support::run;

use support::location::Fixture;
/// Trace: FR-049-AC-1
#[test]
fn genuine_startup_ignores_stale_pane_and_explicit_alias_then_caches_session() {
    let Some(f) = Fixture::new("location-startup") else {
        return;
    };
    let result = run(
        f.cmd(&["hook", "codex"]),
        r#"{"hook_event_name":"SessionStart","session_id":"sender-session","cwd":"/work"}"#,
    );
    assert_eq!(result.status, 0, "{}", result.stderr);
    let mut relay = f.relay();
    let regs = relay.agents().unwrap().registrations;
    assert_eq!(regs.len(), 1, "{}", result.stderr);
    assert_eq!(
        regs[0].full_address().unwrap().to_string(),
        "draco:misc:agent-skills"
    );
    assert_eq!(regs[0].herdr_pane.as_ref().unwrap().as_str(), "w1:p1");
    let who = run(f.cmd(&["whoami", "--json"]), "");
    assert_eq!(
        who.stdout,
        "{\"name\":\"draco:misc:agent-skills\",\"source\":\"location\",\"address\":\"draco:misc:agent-skills\"}\n"
    );
    let value: serde_json::Value = serde_json::from_str(&who.stdout).unwrap();
    assert_eq!(value["name"], "draco:misc:agent-skills");
    assert!(!who.stdout.contains("filament"));
    assert_eq!(
        relay
            .probe_session(&"sender-session".parse().unwrap())
            .unwrap()
            .name,
        Some(regs[0].name.clone())
    );
}
#[test]
fn cold_send_enrolls_local_and_remote_recipient_and_shortcuts_work() {
    let Some(f) = Fixture::new("location-cold-send") else {
        return;
    };
    f.publish("terra");
    for address in ["draco:misc:code", ":misc:code", ":code", "terra:laya:code"] {
        let result = run(f.cmd(&["send", address, "cold message"]), "");
        assert_eq!(result.status, 0, "{address}: {}", result.stderr);
        assert!(
            result.stdout.contains(if address.starts_with("terra") {
                "queued for terra:laya:code"
            } else {
                "queued for draco:misc:code"
            }),
            "{}",
            result.stdout
        );
    }
    let mut relay = f.relay();
    let local = AgentName::for_session(
        &"draco".parse().unwrap(),
        &"recipient-session".parse().unwrap(),
    );
    let remote =
        AgentName::for_session(&"terra".parse().unwrap(), &"terra-session".parse().unwrap());
    assert_eq!(relay.inbox_len(&local).unwrap(), 3);
    assert_eq!(relay.inbox_len(&remote).unwrap(), 1);
    assert!(
        relay
            .log(0, 10)
            .unwrap()
            .iter()
            .all(|m| m.from.as_str() == "draco:misc:agent-skills")
    );
}
#[test]
fn ambiguous_and_unverified_live_recipients_never_enqueue() {
    let Some(f) = Fixture::new("location-refuse") else {
        return;
    };
    f.state(true, true);
    let result = run(f.cmd(&["send", "draco:misc:code", "refuse"]), "");
    assert_ne!(result.status, 0);
    assert!(result.stderr.contains("multiple live leads"));
    f.state(false, false);
    f.publish("draco");
    let result = run(f.cmd(&["send", "draco:misc:code", "refuse"]), "");
    assert_ne!(result.status, 0);
    assert!(result.stderr.contains("session-verified"));
    assert!(f.relay().log(0, 10).unwrap().is_empty());
}
#[test]
fn bare_names_panes_and_four_part_addresses_are_rejected_without_queueing() {
    let Some(f) = Fixture::new("location-no-aliases") else {
        return;
    };
    for address in [
        "filament-code-old-15",
        "w1:p2",
        "draco:misc",
        "draco:misc:code:w1:p2",
    ] {
        let result = run(f.cmd(&["send", address, "refuse"]), "");
        assert_ne!(result.status, 0, "{address}");
    }
    assert!(f.relay().log(0, 10).unwrap().is_empty());
}

#[test]
fn all_excludes_the_canonical_sender_and_queues_one_copy_per_recipient() {
    let Some(f) = Fixture::new("location-broadcast") else {
        return;
    };
    assert_eq!(
        run(f.cmd(&["send", "draco:misc:code", "enroll"]), "").status,
        0
    );
    assert_eq!(run(f.cmd(&["send", "all", "broadcast"]), "").status, 0);
    let mut relay = f.relay();
    let sender = AgentName::for_session(
        &"draco".parse().unwrap(),
        &"sender-session".parse().unwrap(),
    );
    let recipient = AgentName::for_session(
        &"draco".parse().unwrap(),
        &"recipient-session".parse().unwrap(),
    );
    assert_eq!(relay.inbox_len(&sender).unwrap(), 0);
    assert_eq!(relay.inbox_len(&recipient).unwrap(), 2);
}
#[test]
fn post_tool_stop_and_subagent_hooks_preserve_delivery_semantics() {
    let Some(f) = Fixture::new("location-hook-delivery") else {
        return;
    };
    assert_eq!(
        run(f.cmd(&["send", "draco:misc:code", "first"]), "").status,
        0
    );
    let sub = run(
        f.cmd(&["hook", "codex"]),
        r#"{"hook_event_name":"PostToolUse","session_id":"recipient-session","agent_id":"subagent"}"#,
    );
    assert_eq!(sub.status, 0);
    assert!(sub.stdout.is_empty());
    let first = run(
        f.cmd(&["hook", "codex"]),
        r#"{"hook_event_name":"SessionStart","session_id":"recipient-session"}"#,
    );
    assert_eq!(first.status, 0);
    assert!(
        first.stdout.contains("draco:misc:agent-skills: first"),
        "{} {}",
        first.stdout,
        first.stderr
    );
    assert_eq!(
        run(f.cmd(&["send", "draco:misc:code", "second"]), "").status,
        0
    );
    let second = run(
        f.cmd(&["hook", "codex"]),
        r#"{"hook_event_name":"PostToolUse","session_id":"recipient-session"}"#,
    );
    assert!(second.stdout.contains("second"));
    assert_eq!(
        run(f.cmd(&["send", "draco:misc:code", "third"]), "").status,
        0
    );
    let third = run(
        f.cmd(&["hook", "codex"]),
        r#"{"hook_event_name":"Stop","session_id":"recipient-session"}"#,
    );
    let value: serde_json::Value = serde_json::from_str(&third.stdout).unwrap();
    assert_eq!(value["decision"], "block");
    assert!(value["reason"].as_str().unwrap().contains("third"));
    let id = AgentName::for_session(
        &"draco".parse().unwrap(),
        &"recipient-session".parse().unwrap(),
    );
    assert_eq!(f.relay().inbox_len(&id).unwrap(), 0);
}
#[test]
fn concurrent_cold_sends_share_one_session_registration_without_duplicate_messages() {
    let Some(f) = Fixture::new("location-concurrent") else {
        return;
    };
    let commands: Vec<_> = (0..8)
        .map(|_| f.cmd(&["send", "draco:misc:code", "concurrent"]))
        .collect();
    let results = std::thread::scope(|scope| {
        commands
            .into_iter()
            .map(|cmd| scope.spawn(|| run(cmd, "")))
            .collect::<Vec<_>>()
            .into_iter()
            .map(|t| t.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert!(
        results.iter().all(|r| r.status == 0),
        "{:?}",
        results.iter().map(|r| &r.stderr).collect::<Vec<_>>()
    );
    let mut relay = f.relay();
    let id = AgentName::for_session(
        &"draco".parse().unwrap(),
        &"recipient-session".parse().unwrap(),
    );
    assert_eq!(relay.agents().unwrap().registrations.len(), 2);
    assert_eq!(relay.inbox_len(&id).unwrap(), 8);
    assert_eq!(relay.log(0, 20).unwrap().len(), 8);
}

#[test]
fn renamed_tabs_keep_the_session_queue_and_preserve_public_history_snapshots() {
    let Some(f) = Fixture::new("location-rename-history") else {
        return;
    };
    let sent = run(f.cmd(&["send", "draco:misc:code", "before rename"]), "");
    assert_eq!(sent.status, 0, "{}", sent.stderr);
    let mut state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(f.dir.join("state.json")).unwrap()).unwrap();
    state["draco"]["tabs"][1]["label"] = "review".into();
    std::fs::write(
        f.dir.join("state.json"),
        serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();
    // The owning watcher, not the sender, publishes topology changes.
    f.publish("draco");
    let old = run(
        f.cmd(&["send", "draco:misc:code", "refuse old address"]),
        "",
    );
    assert_ne!(old.status, 0);
    let new = run(f.cmd(&["send", ":review", "after rename"]), "");
    assert_eq!(new.status, 0, "{}", new.stderr);
    let mut relay = f.relay();
    let id = AgentName::for_session(
        &"draco".parse().unwrap(),
        &"recipient-session".parse().unwrap(),
    );
    assert_eq!(relay.inbox_len(&id).unwrap(), 2);
    assert_eq!(
        relay
            .agent(&id)
            .unwrap()
            .unwrap()
            .full_address()
            .unwrap()
            .to_string(),
        "draco:misc:review"
    );
    let messages = relay.log(0, 10).unwrap();
    let before = messages.iter().find(|m| m.text == "before rename").unwrap();
    let after = messages.iter().find(|m| m.text == "after rename").unwrap();
    assert_eq!(before.to.to_string(), "draco:misc:code");
    assert_eq!(after.to.to_string(), "draco:misc:review");
    assert!(
        messages
            .iter()
            .all(|m| m.from.as_str() == "draco:misc:agent-skills")
    );
}

#[test]
fn one_session_claimed_by_multiple_panes_is_not_enrolled_even_in_separate_tabs() {
    let Some(f) = Fixture::new("location-duplicate-session") else {
        return;
    };
    let mut state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(f.dir.join("state.json")).unwrap()).unwrap();
    state["draco"]["agents"][0]["agent_session"]["value"] = "recipient-session".into();
    std::fs::write(
        f.dir.join("state.json"),
        serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();
    let result = run(f.cmd(&["send", "draco:misc:code", "refuse"]), "");
    assert_ne!(result.status, 0);
    assert!(
        result.stderr.contains("multiple live leads"),
        "{}",
        result.stderr
    );
    assert!(f.relay().agents().unwrap().registrations.is_empty());
    assert!(f.relay().log(0, 100).unwrap().is_empty());
}

#[test]
fn cold_send_repairs_an_unreadable_genuine_session_record_from_live_evidence() {
    let Some(f) = Fixture::new("location-corrupt-enrollment") else {
        return;
    };
    let id = AgentName::for_session(
        &"draco".parse().unwrap(),
        &"recipient-session".parse().unwrap(),
    );
    let mut raw = redis::Client::open(f.url.as_str())
        .unwrap()
        .get_connection()
        .unwrap();
    let _: () = redis::Commands::hset(
        &mut raw,
        format!("{}agents", f.prefix),
        id.as_str(),
        "not JSON",
    )
    .unwrap();
    let result = run(f.cmd(&["send", "draco:misc:code", "repaired"]), "");
    assert_eq!(result.status, 0, "{}", result.stderr);
    let mut relay = f.relay();
    assert_eq!(relay.inbox_len(&id).unwrap(), 1);
    assert_eq!(
        relay
            .agent(&id)
            .unwrap()
            .unwrap()
            .full_address()
            .unwrap()
            .to_string(),
        "draco:misc:code"
    );
}

#[test]
fn malformed_topology_and_unaddressable_labels_never_create_fallback_identities() {
    let Some(f) = Fixture::new("location-invalid-topology") else {
        return;
    };
    let original: serde_json::Value =
        serde_json::from_slice(&std::fs::read(f.dir.join("state.json")).unwrap()).unwrap();
    for changed in ["invalid-id", "colon-label", "overlong-label"] {
        let mut state = original.clone();
        match changed {
            "invalid-id" => state["draco"]["agents"][1]["pane_id"] = "".into(),
            "colon-label" => state["draco"]["tabs"][1]["label"] = "bad:label".into(),
            _ => state["draco"]["tabs"][1]["label"] = "x".repeat(200).into(),
        }
        std::fs::write(
            f.dir.join("state.json"),
            serde_json::to_vec(&state).unwrap(),
        )
        .unwrap();
        let result = run(f.cmd(&["send", "draco:misc:code", "refuse"]), "");
        assert_ne!(result.status, 0, "{changed}: {}", result.stdout);
        assert!(f.relay().log(0, 100).unwrap().is_empty());
        assert!(
            f.relay()
                .agents()
                .unwrap()
                .registrations
                .iter()
                .all(|registration| {
                    registration.name
                        == AgentName::for_session(
                            &"draco".parse().unwrap(),
                            &"sender-session".parse().unwrap(),
                        )
                        && registration.full_address().unwrap().to_string()
                            == "draco:misc:agent-skills"
                })
        );
    }
}

/// Trace: FR-049-AC-1
#[test]
fn relay_hit_sends_with_no_herdr_or_ssh_access_for_sender_or_recipient() {
    let Some(f) = Fixture::new("location-relay-only") else {
        return;
    };
    f.publish("draco");
    f.publish("terra");
    std::fs::write(
        f.dir.join("herdr"),
        "#!/bin/sh\necho forbidden-herdr >&2\nexit 99\n",
    )
    .unwrap();
    std::fs::write(
        f.dir.join("ssh"),
        "#!/bin/sh\necho forbidden-ssh >&2\nexit 99\n",
    )
    .unwrap();
    for address in ["terra:laya:code", ":code", ":misc:code"] {
        let result = run(f.cmd(&["send", address, "redis-only proof"]), "");
        assert_eq!(result.status, 0, "{address}: {}", result.stderr);
        assert!(!result.stderr.contains("forbidden"));
    }
    let found = run(f.cmd(&["find", "laya", "--json"]), "");
    assert_eq!(found.status, 0, "{}", found.stderr);
    assert_eq!(
        found.stdout,
        "{\"address\":\"terra:laya:code\",\"bus_name\":\"terra:laya:code\",\"machine\":\"terra\",\"workspace_label\":\"laya\",\"tab_label\":\"code\",\"pane_id\":\"w1:p1\",\"session_name\":null,\"herdr_status\":\"unknown\",\"on_bus\":true}\n"
    );
    assert!(found.stdout.contains("terra:laya:code"), "{}", found.stdout);
    assert!(!found.stderr.contains("forbidden"));
    let missing = run(f.cmd(&["find", "no-such-workspace", "--json"]), "");
    assert_eq!(missing.status, 0, "{}", missing.stderr);
    assert_eq!(missing.stdout, "");
    assert_eq!(f.relay().log(0, 10).unwrap().len(), 3);
}

#[test]
fn cold_remote_send_waits_for_destination_watcher_without_remote_herdr() {
    let Some(f) = Fixture::new("location-remote-refresh") else {
        return;
    };
    f.publish("draco");
    std::fs::write(
        f.dir.join("herdr"),
        "#!/bin/sh\necho forbidden-herdr >&2\nexit 99\n",
    )
    .unwrap();
    std::fs::write(
        f.dir.join("ssh"),
        "#!/bin/sh\necho forbidden-ssh >&2\nexit 99\n",
    )
    .unwrap();
    std::thread::scope(|scope| {
        let sender = scope.spawn(|| {
            run(
                f.cmd(&["send", "terra:laya:code", "cold redis-only proof"]),
                "",
            )
        });
        let mut raw = redis::Client::open(f.url.as_str())
            .unwrap()
            .get_connection()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let requested: bool =
                redis::Commands::exists(&mut raw, format!("{}directory-refresh:terra", f.prefix))
                    .unwrap();
            if requested {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "remote refresh not requested"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        f.publish("terra");
        let result = sender.join().unwrap();
        assert_eq!(result.status, 0, "{}", result.stderr);
        assert!(result.stdout.contains("queued for terra:laya:code"));
    });
    assert_eq!(f.relay().log(0, 10).unwrap().len(), 1);
}
