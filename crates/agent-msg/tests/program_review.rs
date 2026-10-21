//! FR-056 through FR-058: the program review status, focus, and server CLI contracts.
//! Redis-backed commands skip when `IX_RELAY_TEST_URL` is unset.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: a failed unwrap/expect is the failure signal"
)]

mod support;

use std::time::Duration;
use std::{collections::BTreeSet, num::NonZeroUsize};

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

fn assert_json_keys(value: &serde_json::Value, expected: &[&str]) {
    let actual = value
        .as_object()
        .expect("JSON value is an object")
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, expected.iter().copied().collect());
}

/// Trace: FR-056-AC-1, FR-056-AC-3, FR-056-AC-4, FR-056-AC-5
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
    assert_json_keys(
        &all,
        &["request_id", "created_at_ms", "requested", "deliveries"],
    );
    assert!(all["created_at_ms"].as_u64().is_some());
    let all_request_id = all["request_id"].as_str().expect("request id");
    assert!((1..=64).contains(&all_request_id.len()));
    assert!(
        all_request_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    );
    let hub: relay_core::MachineName = "hub".parse().unwrap();
    let code_name = relay_core::AgentName::for_session(&hub, &"s1".parse().unwrap());
    let sender_name = relay_core::AgentName::for_session(&hub, &"sender-session".parse().unwrap());
    let all_requested = all["requested"].as_array().unwrap();
    assert!(all_requested.len() <= 128);
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
    for delivery in all["deliveries"].as_array().unwrap() {
        assert_json_keys(delivery, &["agent", "outcome"]);
        assert!(delivery["agent"].as_str().is_some());
        assert!(delivery["outcome"].as_str().is_some());
    }
    let review_lane: relay_core::AgentName = "review-lane".parse().unwrap();
    for recipient in [&code_name, &review_lane] {
        let notice = relay
            .pop_inbox(recipient, NonZeroUsize::new(1).unwrap())
            .unwrap();
        let message = notice.messages.first().expect("one queued status notice");
        assert!(message.text.contains(all_request_id), "{}", message.text);
        assert!(
            message
                .text
                .contains(&format!("agent-msg status-reply {all_request_id}")),
            "{}",
            message.text
        );
    }

    let requested_args = ["status-request", "hub:EA:code"];
    let requested = run(&requested_args, "hub:EA:sender", &url, &prefix);
    assert_success(&requested, &requested_args);
    let request: serde_json::Value = serde_json::from_str(requested.stdout.trim()).unwrap();
    let request_id = request["request_id"].as_str().expect("request id");
    let requested_lane = request["requested"][0].as_str().expect("requested lane");
    assert!(!requested_lane.is_empty());
    assert_eq!(request["deliveries"][0]["outcome"], "queued");
    let request_notice = relay
        .pop_inbox(&code_name, NonZeroUsize::new(1).unwrap())
        .unwrap();
    assert_eq!(request_notice.messages.len(), 1);
    assert!(request_notice.messages[0].text.contains(request_id));

    let reviewer_request_args = ["status-request", "review-lane"];
    let reviewer_request = run(&reviewer_request_args, "hub:EA:sender", &url, &prefix);
    assert_success(&reviewer_request, &reviewer_request_args);
    let reviewer_request_report: serde_json::Value =
        serde_json::from_str(reviewer_request.stdout.trim()).unwrap();
    let reviewer_request_id = reviewer_request_report["request_id"]
        .as_str()
        .expect("reviewer request id");
    let reviewer_notice = relay
        .pop_inbox(&review_lane, NonZeroUsize::new(1).unwrap())
        .unwrap();
    assert_eq!(reviewer_notice.messages.len(), 1);
    assert!(
        reviewer_notice.messages[0]
            .text
            .contains(reviewer_request_id)
    );
    assert!(
        reviewer_notice.messages[0]
            .text
            .contains(&format!("agent-msg status-reply {reviewer_request_id}"))
    );
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
        "send lane-a review now",
    ];
    assert_eq!(relay.inbox_len(&code_name).unwrap(), 0);
    assert_eq!(relay.inbox_len(&review_lane).unwrap(), 0);
    let first = run(&reply_args, "hub:EA:code", &url, &prefix);
    assert_success(&first, &reply_args);
    assert_eq!(first.stdout.trim(), "reply recorded");
    assert_eq!(relay.inbox_len(&code_name).unwrap(), 0);
    assert_eq!(relay.inbox_len(&review_lane).unwrap(), 0);

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
    assert_json_keys(
        &report,
        &[
            "request_id",
            "created_at_ms",
            "requested",
            "replies",
            "nonresponders",
        ],
    );
    assert!(report["created_at_ms"].as_u64().is_some());
    let reply = &report["replies"][0];
    assert_json_keys(
        reply,
        &[
            "request_id",
            "agent",
            "tickets",
            "pull_requests",
            "blocker_ticket_ids",
            "lane_state",
            "spare_coder_slots",
            "owner_decisions",
            "submitted_at_ms",
        ],
    );
    for list in [
        "tickets",
        "pull_requests",
        "blocker_ticket_ids",
        "owner_decisions",
    ] {
        assert!(reply[list].as_array().is_some());
        assert!(reply[list].as_array().unwrap().len() <= 64);
        assert!(
            reply[list]
                .as_array()
                .unwrap()
                .iter()
                .all(serde_json::Value::is_string)
        );
    }
    assert_eq!(reply["blocker_ticket_ids"][0], "SWM-109");
    assert_eq!(reply["spare_coder_slots"], 2);
    assert_eq!(reply["owner_decisions"][0], "send lane-a review now");
    assert!(reply["submitted_at_ms"].as_u64().is_some());
    assert_eq!(report["replies"][0]["tickets"][0], "SWM-107");
    assert_eq!(report["replies"][0]["lane_state"], "active");

    let minimal_request_args = ["status-request", "hub:EA:code"];
    let minimal_request = run(&minimal_request_args, "hub:EA:sender", &url, &prefix);
    assert_success(&minimal_request, &minimal_request_args);
    let minimal_request: serde_json::Value =
        serde_json::from_str(minimal_request.stdout.trim()).unwrap();
    let minimal_request_id = minimal_request["request_id"].as_str().unwrap();
    let minimal_notice = relay
        .pop_inbox(&code_name, NonZeroUsize::new(1).unwrap())
        .unwrap();
    assert_eq!(minimal_notice.messages.len(), 1);
    let minimal_reply_args = [
        "status-reply",
        minimal_request_id,
        "--state",
        "idle",
        "--spare-coder-slots",
        "0",
    ];
    let minimal_reply = run(&minimal_reply_args, "hub:EA:code", &url, &prefix);
    assert_success(&minimal_reply, &minimal_reply_args);
    let minimal_collection_args = ["status-replies", minimal_request_id];
    let minimal_collection = run(&minimal_collection_args, "hub:EA:sender", &url, &prefix);
    assert_success(&minimal_collection, &minimal_collection_args);
    let minimal_report: serde_json::Value =
        serde_json::from_str(minimal_collection.stdout.trim()).unwrap();
    for list in [
        "tickets",
        "pull_requests",
        "blocker_ticket_ids",
        "owner_decisions",
    ] {
        assert_eq!(minimal_report["replies"][0][list], serde_json::json!([]));
    }

    let empty_prefix = format!("test:{}:program-review-empty:", std::process::id());
    let empty_sender = run(&["register"], "hub:EA:empty-sender", &url, &empty_prefix);
    assert_success(&empty_sender, &["register"]);
    let empty_request_args = ["status-request", "all"];
    let empty_request = run(
        &empty_request_args,
        "hub:EA:empty-sender",
        &url,
        &empty_prefix,
    );
    assert_success(&empty_request, &empty_request_args);
    let empty_request: serde_json::Value =
        serde_json::from_str(empty_request.stdout.trim()).unwrap();
    assert_json_keys(
        &empty_request,
        &["request_id", "created_at_ms", "requested", "deliveries"],
    );
    assert_eq!(empty_request["requested"], serde_json::json!([]));
    assert_eq!(empty_request["deliveries"], serde_json::json!([]));
    let empty_request_id = empty_request["request_id"].as_str().unwrap();
    let empty_collection_args = ["status-replies", empty_request_id];
    let empty_collection = run(
        &empty_collection_args,
        "hub:EA:empty-sender",
        &url,
        &empty_prefix,
    );
    assert_success(&empty_collection, &empty_collection_args);
    let empty_collection: serde_json::Value =
        serde_json::from_str(empty_collection.stdout.trim()).unwrap();
    assert_json_keys(
        &empty_collection,
        &[
            "request_id",
            "created_at_ms",
            "requested",
            "replies",
            "nonresponders",
        ],
    );
    for field in ["requested", "replies", "nonresponders"] {
        assert_eq!(empty_collection[field], serde_json::json!([]));
    }

    for missing in [
        [
            "status-reply",
            minimal_request_id,
            "--spare-coder-slots",
            "0",
        ],
        ["status-reply", minimal_request_id, "--state", "active"],
    ] {
        let missing_required = run(&missing, "hub:EA:code", &url, &prefix);
        assert_ne!(missing_required.status, 0);
    }

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

    let invalid_blocker_args = [
        "status-reply",
        request_id,
        "--state",
        "active",
        "--spare-coder-slots",
        "0",
        "--blocker-ticket",
        "not-a-ticket",
    ];
    let invalid_blocker = run(&invalid_blocker_args, "hub:EA:code", &url, &prefix);
    assert_ne!(invalid_blocker.status, 0);
}

/// Trace: FR-057-AC-1, FR-057-AC-2, FR-057-AC-3, FR-057-AC-4, FR-057-AC-7
#[test]
fn tc_057_focus_commands_set_clear_and_render_stale_focus() {
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:program-review-focus:", std::process::id());
    let registered = run(&["register"], "hub:EA:code", &url, &prefix);
    assert_success(&registered, &["register"]);
    let mut relay = Relay::connect(&url, Duration::from_secs(2))
        .unwrap()
        .with_prefix(prefix.clone());
    let other_lane: relay_core::AgentName = "other-lane".parse().unwrap();
    relay
        .register(&Registration {
            name: other_lane.clone(),
            machine: "hub".parse().unwrap(),
            kind: "codex".into(),
            session_id: Some("other-lane-session".parse().unwrap()),
            cwd: None,
            herdr_pane: None,
            place: Some(Place {
                workspace: "EA".into(),
                tab: "other".into(),
                herdr_ids: None,
            }),
            last_seen_ms: 1,
        })
        .unwrap();

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
    let focus_line = listed
        .stdout
        .lines()
        .find(|line| line.starts_with("hub:EA:code  SWM-108-lane-focus  set "))
        .expect("focus line includes its set time");
    assert!(
        focus_line
            .split("  set ")
            .nth(1)
            .is_some_and(|time| !time.trim().is_empty())
    );
    assert!(listed.stdout.contains("STALE"), "{}", listed.stdout);
    assert!(
        listed.stdout.contains("hub:EA:other  focus unset"),
        "{}",
        listed.stdout
    );
    assert_eq!(relay.inbox_len(&other_lane).unwrap(), 0);
    relay
        .send(&relay_core::Message {
            from: "hub:EA:code".parse().unwrap(),
            to: relay_core::Target::Agent(other_lane.clone()),
            text: "SWM-108 open ticket needs review".into(),
            sent_at_ms: 1,
            forwarded: false,
        })
        .unwrap();
    let with_activity = run(&["who", "--focus"], "hub:EA:code", &url, &prefix);
    assert_success(&with_activity, &["who", "--focus"]);
    assert!(with_activity.stdout.contains("hub:EA:other  focus unset"));
    assert_eq!(relay.inbox_len(&other_lane).unwrap(), 1);
    relay
        .set_focus(&other_lane, Some("Other lane focus"))
        .unwrap();
    let both_focused = run(&["who", "--focus"], "hub:EA:code", &url, &prefix);
    assert_success(&both_focused, &["who", "--focus"]);
    assert!(
        both_focused
            .stdout
            .contains("hub:EA:other  Other lane focus  set ")
    );

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
    assert!(
        after_clear
            .stdout
            .contains("hub:EA:other  Other lane focus  set "),
        "clearing the caller's focus changed another lane: {}",
        after_clear.stdout
    );
}

/// Trace: FR-058-AC-1, FR-058-AC-2, FR-058-AC-3, FR-058-AC-4, FR-058-AC-6
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
    let empty_server_args = [
        "server-capacity",
        "set",
        "capacity-only",
        "--cores",
        "2",
        "--ram-gib",
        "8",
    ];
    let empty_server = run(&empty_server_args, "hub:EA:code", &url, &prefix);
    assert_success(&empty_server, &empty_server_args);
    assert!(
        empty_server
            .stdout
            .contains("capacity recorded for capacity-only")
    );

    let who_args = ["who", "--servers"];
    let listed = run(&who_args, "hub:EA:code", &url, &prefix);
    assert_success(&listed, &who_args);
    assert!(
        listed
            .stdout
            .contains("hub: capacity unset, lanes=[hub:EA:code]"),
        "{}",
        listed.stdout
    );
    assert!(listed.stdout.contains("node-a: cores=8, RAM=32768 MiB, max-agents=4, rules=review team, lanes=[node-a:EA:reviewer]"), "{}", listed.stdout);
    assert!(
        listed.stdout.contains(
            "capacity-only: cores=2, RAM=8192 MiB, max-agents=unset, rules=none, lanes=[none]"
        ),
        "{}",
        listed.stdout
    );

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
    assert!(
        after_clear.stdout.contains(
            "capacity-only: cores=2, RAM=8192 MiB, max-agents=unset, rules=none, lanes=[none]"
        ),
        "clearing node-a changed the other server's capacity: {}",
        after_clear.stdout
    );
}
