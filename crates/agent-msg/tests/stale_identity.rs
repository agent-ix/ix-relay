//! A registry record and inherited pane variables cannot prove caller ownership.
#![allow(clippy::unwrap_used, reason = "test fixtures")]
mod support;
use relay_core::{AgentName, Registration};
use support::{location::Fixture, run};

#[test]
fn verified_caller_ignores_a_newer_stale_record_for_the_inherited_pane() {
    let Some(f) = Fixture::new("stale-registry-proof") else {
        return;
    };
    let old = Registration {
        name: "filament-code-old-15".parse().unwrap(),
        machine: "draco".parse().unwrap(),
        kind: "codex".into(),
        session_id: Some("old-session".parse().unwrap()),
        cwd: None,
        herdr_pane: Some("wD:p1".parse().unwrap()),
        place: None,
        last_seen_ms: i64::MAX,
    };
    let mut relay = f.relay();
    relay.register(&old).unwrap();
    for args in [vec!["whoami", "--json"], vec!["register"]] {
        let result = run(f.cmd(&args), "");
        assert_eq!(result.status, 0, "{}", result.stderr);
        assert!(result.stdout.contains("draco:misc:agent-skills"));
        assert!(!result.stdout.contains("filament"));
        assert_eq!(
            relay.agent(&old.name).unwrap().unwrap().last_seen_ms,
            i64::MAX
        );
    }
    let genuine = AgentName::for_session(
        &"draco".parse().unwrap(),
        &"sender-session".parse().unwrap(),
    );
    assert_eq!(
        relay
            .agent(&genuine)
            .unwrap()
            .unwrap()
            .herdr_pane
            .unwrap()
            .as_str(),
        "w1:p1"
    );
}

#[test]
fn unverified_or_different_session_cannot_adopt_a_live_pane() {
    let Some(f) = Fixture::new("unverified-caller-proof") else {
        return;
    };
    for (verified, session) in [(false, "sender-session"), (true, "different-session")] {
        f.state(false, verified);
        for args in [vec!["register"], vec!["send", ":code", "refuse"]] {
            let mut cmd = f.cmd(&args);
            cmd.env("CODEX_THREAD_ID", session);
            let result = run(cmd, "");
            assert_ne!(result.status, 0, "{}", result.stdout);
        }
    }
    assert!(f.relay().agents().unwrap().registrations.is_empty());
    assert!(f.relay().log(0, 100).unwrap().is_empty());
}
