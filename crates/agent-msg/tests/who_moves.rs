//! FR-032: `agent-msg who` shows a lead whose pane now hosts another session as
//! undeliverable, and stops when the lead registers again or is relinked.
//! Redis-backed: skips without `IX_RELAY_TEST_URL` (`make test-redis`).
#![expect(
    clippy::unwrap_used,
    reason = "test code: a failed unwrap/expect is the failure signal"
)]

mod support;

use std::time::Duration;

use relay_core::{Message, Place, Registration, Relay, Target, moves::PaneReplaced};

fn registration(last_seen_ms: i64, pane: &str) -> Registration {
    Registration {
        name: "lead-a".parse().unwrap(),
        machine: "hub".parse().unwrap(),
        kind: "claude".into(),
        session_id: Some("sess-1".parse().unwrap()),
        cwd: None,
        herdr_pane: Some(pane.parse().unwrap()),
        place: Some(Place {
            workspace: "wsA".into(),
            tab: "1".into(),
            herdr_ids: None,
        }),
        last_seen_ms,
    }
}

fn who(url: &str, prefix: &str) -> String {
    let run = support::agent_msg(&["who"], Some("someone"), Some(url), Some(prefix), "");
    assert_eq!(run.status, 0, "who failed: {}", run.stderr);
    run.stdout
}

/// Tracing: TC-032-04
/// ACs: FR-032-AC-2, FR-032-AC-4
#[test]
fn tc_032_02_who_shows_a_replaced_pane_until_the_lead_registers_again() {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
        return;
    };
    let prefix = format!("test:{}:who-moves:", std::process::id());
    let mut relay = Relay::connect(&url, Duration::from_secs(2))
        .unwrap()
        .with_prefix(prefix.clone());
    relay.register(&registration(1_000, "w1:p3")).unwrap();
    // Mail waits for the lead: the marker means something while it does.
    relay
        .send(&Message {
            from: "sender".parse().unwrap(),
            to: Target::Agent("lead-a".parse().unwrap()),
            text: "hi".into(),
            sent_at_ms: 1,
            forwarded: false,
        })
        .unwrap();
    assert!(
        !who(&url, &prefix).contains("UNDELIVERABLE"),
        "a healthy lead"
    );

    let found = PaneReplaced {
        pane: "w1:p3".parse().unwrap(),
        seen_last_seen_ms: 1_000,
        since_ms: 2_000,
    };
    relay
        .mark_pane_replaced(&"lead-a".parse().unwrap(), &found)
        .unwrap();
    let listed = who(&url, &prefix);
    assert!(
        listed.contains("location unverified")
            && listed.contains("UNDELIVERABLE: pane w1:p3 now hosts another session"),
        "{listed}"
    );

    // It registers again from a live pane: the old finding no longer applies.
    relay.register(&registration(5_000, "w1:p3")).unwrap();
    assert!(!who(&url, &prefix).contains("UNDELIVERABLE"));

    // Relinked to another pane: also not described by the old finding.
    relay.register(&registration(1_000, "w2:p2")).unwrap();
    assert!(!who(&url, &prefix).contains("UNDELIVERABLE"));

    let _: () = redis::Client::open(url.as_str())
        .and_then(|c| c.get_connection())
        .and_then(|mut c| {
            redis::cmd("DEL")
                .arg(format!("{prefix}agents"))
                .arg(format!("{prefix}health:replaced"))
                .arg(format!("{prefix}inbox:lead-a"))
                .exec(&mut c)
        })
        .unwrap();
}

/// Tracing: TC-053-18
/// ACs: FR-053-AC-16
///
/// Once the watcher has moved a replaced registration's mail, its inbox is empty and
/// `who` no longer calls it undeliverable; mail sent to it afterwards brings the marker back.
#[test]
fn tc_053_18_who_stops_calling_a_registration_undeliverable_once_its_inbox_is_empty() {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
        return;
    };
    let prefix = format!("test:{}:who-moved-mail:", std::process::id());
    let mut relay = Relay::connect(&url, Duration::from_secs(2))
        .unwrap()
        .with_prefix(prefix.clone());
    let lead: relay_core::AgentName = "lead-a".parse().unwrap();
    relay.register(&registration(1_000, "w1:p3")).unwrap();
    let send = |relay: &mut Relay| {
        relay
            .send(&Message {
                from: "sender".parse().unwrap(),
                to: Target::Agent("lead-a".parse().unwrap()),
                text: "hi".into(),
                sent_at_ms: 1,
                forwarded: false,
            })
            .unwrap();
    };
    send(&mut relay);
    let found = PaneReplaced {
        pane: "w1:p3".parse().unwrap(),
        seen_last_seen_ms: 1_000,
        since_ms: 2_000,
    };
    relay.mark_pane_replaced(&lead, &found).unwrap();
    assert!(who(&url, &prefix).contains("UNDELIVERABLE"), "mail waits");

    relay
        .pop_inbox(&lead, std::num::NonZeroUsize::new(10).unwrap())
        .unwrap();
    assert!(
        !who(&url, &prefix).contains("UNDELIVERABLE"),
        "the mail is gone from the inbox"
    );

    send(&mut relay);
    assert!(
        who(&url, &prefix).contains("UNDELIVERABLE"),
        "new mail waits"
    );

    let _: () = redis::Client::open(url.as_str())
        .and_then(|c| c.get_connection())
        .and_then(|mut c| {
            redis::cmd("DEL")
                .arg(format!("{prefix}agents"))
                .arg(format!("{prefix}health:replaced"))
                .arg(format!("{prefix}inbox:lead-a"))
                .exec(&mut c)
        })
        .unwrap();
}
