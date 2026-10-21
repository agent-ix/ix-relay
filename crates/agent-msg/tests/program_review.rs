//! FR-056 through FR-058: the program review status, focus, and server CLI contracts.
//! Redis-backed commands skip when `IX_RELAY_TEST_URL` is unset.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: a failed unwrap/expect is the failure signal"
)]

mod support;

use std::time::Duration;

use relay_core::{Place, Registration, Relay};

fn test_url() -> Option<String> {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
        return None;
    };
    Some(url)
}

fn run(args: &[&str], name: &str, url: &str, prefix: &str) -> support::Run {
    support::agent_msg(args, Some(name), Some(url), Some(prefix), "")
}

fn assert_success(result: &support::Run, args: &[&str]) {
    assert_eq!(
        result.status, 0,
        "agent-msg {args:?} failed: {}",
        result.stderr
    );
}

/// Trace: FR-056-AC-1, FR-056-AC-2, FR-056-AC-3, FR-056-AC-4
#[test]
fn tc_056_status_commands_parse_arguments_and_report_the_collection_contract() {
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:program-review-status:", std::process::id());
    for name in ["hub:EA:code", "hub:EA:sender"] {
        let result = run(&["register"], name, &url, &prefix);
        assert_success(&result, &["register"]);
    }

    let mut relay = Relay::connect(&url, Duration::from_secs(2))
        .unwrap()
        .with_prefix(prefix.clone());
    relay
        .register(&Registration {
            name: "review-lane".parse().unwrap(),
            machine: "hub".parse().unwrap(),
            kind: "codex".into(),
            session_id: Some("review-lane-session".parse().unwrap()),
            cwd: None,
            herdr_pane: None,
            place: Some(Place {
                workspace: "EA".into(),
                tab: "review".into(),
                herdr_ids: None,
            }),
            last_seen_ms: 1,
        })
        .unwrap();

    for target in ["hub:EA:missing", "hub:EA:code,hub:EA:code", "hub:EA:sender"] {
        let args = ["status-request", target];
        let refused = run(&args, "hub:EA:sender", &url, &prefix);
        assert_ne!(refused.status, 0, "target {target:?} was accepted");
        assert!(
            !refused.stderr.is_empty(),
            "target {target:?}: {}",
            refused.stderr
        );
    }

    let all_args = ["status-request", "all"];
    let all_result = run(&all_args, "hub:EA:sender", &url, &prefix);
    assert_success(&all_result, &all_args);
    let all: serde_json::Value = serde_json::from_str(all_result.stdout.trim()).unwrap();
    let hub: relay_core::MachineName = "hub".parse().unwrap();
    let code_name = relay_core::AgentName::for_session(&hub, &"s1".parse().unwrap());
    let sender_name = relay_core::AgentName::for_session(&hub, &"sender-session".parse().unwrap());
    let all_requested = all["requested"].as_array().unwrap();
    assert_eq!(all_requested.len(), 2);
    assert!(all_requested.contains(&serde_json::json!("review-lane")));
    assert!(all_requested.contains(&serde_json::json!(code_name.as_str())));
    assert!(!all_requested.contains(&serde_json::json!(sender_name.as_str())));
    assert!(
        all["deliveries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|delivery| delivery["outcome"] == "queued")
    );

    let requested_args = ["status-request", "hub:EA:code"];
    let requested = run(&requested_args, "hub:EA:sender", &url, &prefix);
    assert_success(&requested, &requested_args);
    let request: serde_json::Value = serde_json::from_str(requested.stdout.trim()).unwrap();
    let request_id = request["request_id"].as_str().expect("request id");
    let requested_lane = request["requested"][0].as_str().expect("requested lane");
    assert!(!requested_lane.is_empty());
    assert_eq!(request["deliveries"][0]["outcome"], "queued");

    let reviewer_request_args = ["status-request", "review-lane"];
    let reviewer_request = run(&reviewer_request_args, "hub:EA:sender", &url, &prefix);
    assert_success(&reviewer_request, &reviewer_request_args);
    let reviewer_request_report: serde_json::Value =
        serde_json::from_str(reviewer_request.stdout.trim()).unwrap();
    let reviewer_request_id = reviewer_request_report["request_id"]
        .as_str()
        .expect("reviewer request id");
    let unrequested_args = [
        "status-reply",
        reviewer_request_id,
        "--state",
        "idle",
        "--spare-coder-slots",
        "1",
    ];
    let unrequested = run(&unrequested_args, "hub:EA:code", &url, &prefix);
    assert_ne!(unrequested.status, 0);
    assert!(!unrequested.stderr.is_empty(), "{}", unrequested.stderr);

    let replies_args = ["status-replies", request_id];
    let partial = run(&replies_args, "hub:EA:sender", &url, &prefix);
    assert_success(&partial, &replies_args);
    let partial_report: serde_json::Value = serde_json::from_str(partial.stdout.trim()).unwrap();
    assert_eq!(partial_report["request_id"], request_id);
    assert_eq!(
        partial_report["requested"],
        serde_json::json!([requested_lane])
    );
    assert_eq!(partial_report["replies"], serde_json::json!([]));
    assert_eq!(
        partial_report["nonresponders"],
        serde_json::json!([requested_lane])
    );

    let reply_args = [
        "status-reply",
        request_id,
        "--ticket",
        "SWM-107",
        "--pull-request",
        "https://github.com/agent-ix/ix-relay/pull/26",
        "--blocker-ticket",
        "SWM-109",
        "--state",
        "active",
        "--spare-coder-slots",
        "2",
        "--owner-decision",
        "none",
    ];
    let first = run(&reply_args, "hub:EA:code", &url, &prefix);
    assert_success(&first, &reply_args);
    assert_eq!(first.stdout.trim(), "reply recorded");

    let changed_reply_args = [
        "status-reply",
        request_id,
        "--ticket",
        "SWM-999",
        "--state",
        "blocked",
        "--spare-coder-slots",
        "0",
    ];
    let changed_duplicate = run(&changed_reply_args, "hub:EA:code", &url, &prefix);
    assert_success(&changed_duplicate, &changed_reply_args);
    assert_eq!(
        changed_duplicate.stdout.trim(),
        "reply already recorded; the first reply was kept"
    );

    let collected = run(&replies_args, "hub:EA:sender", &url, &prefix);
    assert_success(&collected, &replies_args);
    let report: serde_json::Value = serde_json::from_str(collected.stdout.trim()).unwrap();
    assert_eq!(report["request_id"], request_id);
    assert_eq!(report["requested"], serde_json::json!([requested_lane]));
    assert_eq!(report["nonresponders"], serde_json::json!([]));
    assert_eq!(report["replies"].as_array().unwrap().len(), 1);
    assert_eq!(report["replies"][0]["tickets"][0], "SWM-107");
    assert_eq!(report["replies"][0]["lane_state"], "active");
    assert_eq!(report["replies"][0]["spare_coder_slots"], 2);

    let unknown_request_args = ["status-request", "hub:EA:code"];
    let unknown_request = run(&unknown_request_args, "hub:EA:sender", &url, &prefix);
    assert_success(&unknown_request, &unknown_request_args);
    let unknown: serde_json::Value = serde_json::from_str(unknown_request.stdout.trim()).unwrap();
    let unknown_id = unknown["request_id"].as_str().expect("request id");
    let unknown_reply_args = ["status-reply", unknown_id, "--state", "idle"];
    let unknown_reply = run(&unknown_reply_args, "hub:EA:code", &url, &prefix);
    assert_success(&unknown_reply, &unknown_reply_args);
    let unknown_collection_args = ["status-replies", unknown_id];
    let unknown_collection = run(&unknown_collection_args, "hub:EA:sender", &url, &prefix);
    assert_success(&unknown_collection, &unknown_collection_args);
    let unknown_report: serde_json::Value =
        serde_json::from_str(unknown_collection.stdout.trim()).unwrap();
    assert_eq!(
        unknown_report["replies"][0]["spare_coder_slots"],
        serde_json::Value::Null
    );

    let invalid_state = [
        "status-reply",
        request_id,
        "--state",
        "unknown",
        "--spare-coder-slots",
        "0",
    ];
    let refused = run(&invalid_state, "hub:EA:code", &url, &prefix);
    assert_ne!(refused.status, 0);
    assert!(
        refused.stderr.contains("invalid value"),
        "{}",
        refused.stderr
    );
}

/// Trace: FR-057-AC-1, FR-057-AC-2, FR-057-AC-3
#[test]
fn tc_057_focus_commands_set_clear_and_render_stale_focus() {
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:program-review-focus:", std::process::id());
    let registered = run(&["register"], "hub:EA:code", &url, &prefix);
    assert_success(&registered, &["register"]);

    let set_args = ["focus", "SWM-108-lane-focus"];
    let set = run(&set_args, "hub:EA:code", &url, &prefix);
    assert_success(&set, &set_args);
    assert_eq!(set.stdout.trim(), "focus updated");
    let who_args = ["who", "--focus", "--stale-after", "0s"];
    let listed = run(&who_args, "hub:EA:code", &url, &prefix);
    assert_success(&listed, &who_args);
    assert!(
        listed
            .stdout
            .contains("hub:EA:code  SWM-108-lane-focus  set "),
        "{}",
        listed.stdout
    );
    assert!(listed.stdout.contains("STALE"), "{}", listed.stdout);

    let clear_args = ["focus", "--clear"];
    let cleared = run(&clear_args, "hub:EA:code", &url, &prefix);
    assert_success(&cleared, &clear_args);
    assert_eq!(cleared.stdout.trim(), "focus cleared");
    let after_clear = run(&["who", "--focus"], "hub:EA:code", &url, &prefix);
    assert_success(&after_clear, &["who", "--focus"]);
    assert!(
        after_clear.stdout.contains("hub:EA:code  focus unset"),
        "{}",
        after_clear.stdout
    );
}

/// Trace: FR-058-AC-1, FR-058-AC-2
#[test]
fn tc_058_capacity_commands_record_clear_and_group_server_lanes() {
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:program-review-capacity:", std::process::id());
    let registered = run(&["register"], "hub:EA:code", &url, &prefix);
    assert_success(&registered, &["register"]);

    let mut relay = Relay::connect(&url, Duration::from_secs(2))
        .unwrap()
        .with_prefix(prefix.clone());
    relay
        .register(&Registration {
            name: "node-a:EA:reviewer".parse().unwrap(),
            machine: "node-a".parse().unwrap(),
            kind: "codex".into(),
            session_id: Some("reviewer-session".parse().unwrap()),
            cwd: None,
            herdr_pane: None,
            place: Some(Place {
                workspace: "EA".into(),
                tab: "reviewer".into(),
                herdr_ids: None,
            }),
            last_seen_ms: 1,
        })
        .unwrap();

    let set_args = [
        "server-capacity",
        "set",
        "node-a",
        "--cores",
        "8",
        "--ram-gib",
        "32",
        "--max-agents",
        "4",
        "--owner-rule",
        "review team",
    ];
    let set = run(&set_args, "hub:EA:code", &url, &prefix);
    assert_success(&set, &set_args);
    assert!(
        set.stdout
            .contains("capacity recorded for node-a (8 cores, 32768 MiB RAM)"),
        "{}",
        set.stdout
    );

    let who_args = ["who", "--servers"];
    let listed = run(&who_args, "hub:EA:code", &url, &prefix);
    assert_success(&listed, &who_args);
    assert!(
        listed.stdout.contains("hub: cores unset") || listed.stdout.contains("hub: capacity unset"),
        "{}",
        listed.stdout
    );
    assert!(listed.stdout.contains("node-a: cores=8, RAM=32768 MiB, max-agents=4, rules=review team, lanes=[node-a:EA:reviewer]"), "{}", listed.stdout);

    let clear_args = ["server-capacity", "clear", "node-a"];
    let cleared = run(&clear_args, "hub:EA:code", &url, &prefix);
    assert_success(&cleared, &clear_args);
    assert_eq!(cleared.stdout.trim(), "capacity cleared for node-a");
    let after_clear = run(&who_args, "hub:EA:code", &url, &prefix);
    assert_success(&after_clear, &who_args);
    assert!(
        after_clear
            .stdout
            .contains("node-a: capacity unset, lanes=[node-a:EA:reviewer]"),
        "{}",
        after_clear.stdout
    );
}
