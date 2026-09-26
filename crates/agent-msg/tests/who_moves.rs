//! FR-032: `agent-msg who` shows a lead whose pane now hosts another session as
//! undeliverable, and stops when the lead registers again or is relinked.
//! Redis-backed: skips without `IX_RELAY_TEST_URL` (`make test-redis`).
#![expect(
    clippy::unwrap_used,
    reason = "test code: a failed unwrap/expect is the failure signal"
)]

mod support;

use std::time::Duration;

use relay_core::{Place, Registration, Relay, moves::PaneReplaced};

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
            tabs_in_workspace: 1,
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
        listed.contains("lead-a")
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
                .exec(&mut c)
        })
        .unwrap();
}
