//! The receipt selection in the history CLI exposes content-free evidence.

use std::process::Command;

use ix_trace_rs::trace;
use relay_core::{Message, Registration};
use relay_test_support::Bus;

#[test]
#[trace("FR-067-AC-1", "FR-067-AC-7", "FR-067-AC-8")]
fn receipts_cli_selection_is_read_only_and_keeps_ordinary_history_separate() {
    let Some(mut bus) = Bus::connect("receipt-cli") else {
        return;
    };
    bus.relay
        .register(&Registration {
            name: "receiver".parse().unwrap(),
            machine: "fixture".parse().unwrap(),
            kind: "codex".into(),
            session_id: None,
            cwd: None,
            herdr_pane: None,
            place: None,
            last_seen_ms: 0,
        })
        .unwrap();
    bus.relay
        .send(&Message {
            from: "sender".parse().unwrap(),
            to: "receiver".parse().unwrap(),
            text: "SECRET receipt fixture payload".into(),
            sent_at_ms: 42,
            forwarded: false,
        })
        .unwrap();
    let run = |extra: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_agent-msg"))
            .args(["--url", &bus.url, "--prefix", &bus.prefix, "log", "--json"])
            .args(extra)
            .output()
            .unwrap()
    };
    let receipt_output = run(&["--receipts"]);
    assert!(
        receipt_output.status.success(),
        "{}",
        String::from_utf8_lossy(&receipt_output.stderr)
    );
    let receipt_json: serde_json::Value = serde_json::from_slice(&receipt_output.stdout).unwrap();
    assert_eq!(receipt_json["events"].as_array().unwrap().len(), 1);
    assert_eq!(receipt_json["events"][0]["event"]["outcome"], "queued");
    assert!(!String::from_utf8_lossy(&receipt_output.stdout).contains("SECRET"));
    let ordinary_output = run(&[]);
    assert!(
        ordinary_output.status.success(),
        "{}",
        String::from_utf8_lossy(&ordinary_output.stderr)
    );
    let ordinary_json: serde_json::Value = serde_json::from_slice(&ordinary_output.stdout).unwrap();
    assert_eq!(ordinary_json.as_array().unwrap().len(), 1);
    assert_eq!(ordinary_json[0]["text"], "SECRET receipt fixture payload");
    assert_eq!(
        bus.relay.inbox_len(&"receiver".parse().unwrap()).unwrap(),
        1
    );
    assert_eq!(
        bus.relay
            .inflight_len(&"receiver".parse().unwrap())
            .unwrap(),
        0
    );
}

#[test]
#[trace("FR-067-AC-6")]
fn receipts_cli_unreachable_source_is_unavailable_evidence_not_empty_history() {
    let directory = tempfile::tempdir().unwrap();
    let unavailable = format!(
        "redis+unix://{}",
        directory.path().join("absent.sock").display()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_agent-msg"))
        .args([
            "--url",
            &unavailable,
            "--prefix",
            "isolated-missing-source:",
            "log",
            "--receipts",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["complete"], false);
    assert_eq!(report["reasons"], serde_json::json!(["source_unavailable"]));
    assert!(report["events"].as_array().unwrap().is_empty());
    assert_eq!(report["covered_since_ms"], serde_json::Value::Null);
}
