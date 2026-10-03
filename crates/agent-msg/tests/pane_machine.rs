//! AGE-2026: pane ids repeat across machines, so an interactive command that
//! resolves "who am I" from `HERDR_PANE_ID` must only consider registrations
//! on its own machine — and never pop another machine's lead's inbox.
//! Redis-backed: skips without `IX_RELAY_TEST_URL` (`make test-redis`).
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: a failed unwrap/expect is the failure signal"
)]

mod support;

use std::time::Duration;

use relay_core::{Message, Registration, Relay};

fn registration(name: &str, machine: &str) -> Registration {
    Registration {
        name: name.parse().unwrap(),
        machine: machine.parse().unwrap(),
        kind: "claude".into(),
        session_id: Some(name.parse().unwrap()),
        cwd: None,
        herdr_pane: Some("w1:p1".parse().unwrap()),
        place: None,
        last_seen_ms: 1,
    }
}

/// `agent-msg inbox --json` as the lead sitting in pane `w1:p1` on `machine`.
fn inbox_texts(url: &str, prefix: &str, machine: &str) -> Vec<String> {
    let scratch = support::Scratch::new("ix-relay-pane-machine-");
    let session = if machine == "hub" {
        "hub-lead"
    } else {
        "node-a-lead"
    };
    let agent_case = format!(
        r#""w1:p1") echo '{{"result":{{"agent":{{"agent_session":{{"kind":"id","value":"{session}"}}}}}}}}' ;;"#
    );
    let herdr =
        support::fake_herdr_dir("[]", "[]", "[]", &agent_case, &scratch.join("calls"), false);
    let mut cmd = support::base_cmd(&["inbox", "--json"], None);
    cmd.env("IX_RELAY_URL", url)
        .env("IX_RELAY_PREFIX", prefix)
        .env("IX_RELAY_MACHINE", machine)
        .env("HERDR_PANE_ID", "w1:p1")
        .env("HERDR_TAB_ID", "w1:t1")
        .env("HERDR_WORKSPACE_ID", "w1");
    cmd.env(
        "PATH",
        format!("{}:{}", herdr.display(), std::env::var("PATH").unwrap()),
    );
    let run = support::run(cmd, "");
    assert_eq!(run.status, 0, "inbox failed: {}", run.stderr);
    let parsed: serde_json::Value = serde_json::from_str(run.stdout.trim()).expect("json stdout");
    parsed["messages"]
        .as_array()
        .expect("messages array")
        .iter()
        .map(|m| m["text"].as_str().expect("text").to_owned())
        .collect()
}

#[test]
fn the_same_pane_id_on_two_machines_resolves_to_each_machines_own_lead() {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
        return;
    };
    let prefix = format!("test:{}:pane-machine:", std::process::id());
    let mut relay = Relay::connect(&url, Duration::from_secs(2))
        .unwrap()
        .with_prefix(prefix.clone());
    relay.register(&registration("hub-lead", "hub")).unwrap();
    relay
        .register(&registration("node-a-lead", "Node-A.local"))
        .unwrap();
    for (to, text) in [("hub-lead", "for hub"), ("node-a-lead", "for node-a")] {
        relay
            .send(&Message {
                from: "sender".parse().unwrap(),
                to: to.parse().unwrap(),
                text: text.into(),
                sent_at_ms: 1,
            })
            .unwrap();
    }

    assert_eq!(inbox_texts(&url, &prefix, "hub"), ["for hub"]);
    assert_eq!(inbox_texts(&url, &prefix, "node-a"), ["for node-a"]);
    // Both inboxes were popped by their own lead only.
    assert!(inbox_texts(&url, &prefix, "hub").is_empty());
}
