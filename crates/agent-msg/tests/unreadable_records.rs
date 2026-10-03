//! A registration an older binary stored with an id outside its type's rule (an
//! empty, over-long or control-bearing session id the hook stored unchecked, a
//! zero-width character in a pane) is registered but unreadable to this binary.
//! `main` decoded such records and `send` queued to them; this binary must too,
//! and `who` must say what it skipped (AGE-2027 PR review, finding 1).
//! Redis-backed: skips without `IX_RELAY_TEST_URL`.
#![expect(
    clippy::unwrap_used,
    reason = "test code: a failed unwrap/expect is the failure signal"
)]

mod support;

/// `(name, session_id JSON, herdr_pane JSON)`.
fn planted() -> Vec<(&'static str, String, String)> {
    let long = format!("\"{}\"", "x".repeat(300));
    vec![
        ("empty-sid", "\"\"".into(), "\"w1:p1\"".into()),
        ("long-sid", long, "\"w1:p2\"".into()),
        ("tab-sid", "\"a\\tb\"".into(), "\"w1:p3\"".into()),
        ("zw-sid", "\"a\\u200bb\"".into(), "\"w1:p4\"".into()),
        ("zw-pane", "\"s-ok\"".into(), "\"w1:p\\u200b5\"".into()),
    ]
}

fn plant(url: &str, prefix: &str) {
    let mut raw = redis::Client::open(url).unwrap().get_connection().unwrap();
    for (name, session, pane) in planted() {
        let json = format!(
            r#"{{"name":"{name}","machine":"hub","kind":"claude","session_id":{session},"cwd":null,"herdr_pane":{pane},"place":null,"last_seen_ms":1}}"#
        );
        let _: () = redis::Commands::hset(&mut raw, format!("{prefix}agents"), name, json).unwrap();
    }
}

fn url() -> Option<String> {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
        return None;
    };
    Some(url)
}

/// Tracing: TC-008-06
/// ACs: FR-008-AC-6
///
/// `send` to a name whose record is unreadable queues, as it did on `main`
/// (rc 0), and never fails with a corrupt-record error for a name that was
/// accepted before.
#[test]
fn unreadable_legacy_names_are_never_routing_targets() {
    let Some(url) = url() else { return };
    let prefix = format!("test:{}:unreadable:send:", std::process::id());
    plant(&url, &prefix);
    for (name, _, _) in planted() {
        let r = support::agent_msg(
            &["send", name, "hi"],
            Some("hub:EA:sender"),
            Some(&url),
            Some(&prefix),
            "",
        );
        assert_ne!(r.status, 0, "send {name}: {}", r.stderr);
        assert!(r.stderr.contains("invalid lead address"), "{}", r.stderr);
        let mut relay = relay_core::Relay::connect(&url, std::time::Duration::from_secs(2))
            .unwrap()
            .with_prefix(&prefix);
        assert_eq!(relay.inbox_len(&name.parse().unwrap()).unwrap(), 0);
    }
}

/// Tracing: TC-008-06
/// ACs: FR-008-AC-6
///
/// `who` still succeeds and says, per record, that it skipped it.
#[test]
fn who_reports_each_unreadable_record_it_skips() {
    let Some(url) = url() else { return };
    let prefix = format!("test:{}:unreadable:who:", std::process::id());
    plant(&url, &prefix);
    let r = support::agent_msg(&["who"], Some("someone"), Some(&url), Some(&prefix), "");
    assert_eq!(r.status, 0, "{}", r.stderr);
    for (name, _, _) in planted() {
        assert!(
            r.stderr
                .contains(&format!("skipped registry record \"{name}\"")),
            "{name} not reported: {}",
            r.stderr
        );
    }
}
