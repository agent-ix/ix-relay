//! Interactive identity must not adopt a stale inherited pane's registration.
mod support;

use std::time::Duration;

use relay_core::{AgentSlot, Registration, Relay};
use support::{Scratch, base_cmd, fake_herdr_dir, run};

#[test]
fn stale_or_unverified_callers_cannot_refresh_or_send_as_the_registry_holder() {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!("skipping: IX_RELAY_TEST_URL unset");
        return;
    };
    // Cover a missing pane, missing live session, replaced session, missing
    // registered session, different Codex caller, and unavailable Herdr.
    for (case, registered_session, live_session, caller, fail) in [
        ("gone", Some("old"), None, Some("api"), false),
        ("unverified", Some("old"), Some(""), None, false),
        ("replaced", Some("old"), Some("new"), None, false),
        ("manual", None, Some("old"), None, false),
        ("api", Some("old"), Some("old"), Some("api"), false),
        ("unavailable", Some("old"), Some("old"), None, true),
    ] {
        let prefix = format!("test:{}:stale-identity-{case}:", std::process::id());
        let mut relay = Relay::connect(&url, Duration::from_secs(3))
            .unwrap()
            .with_prefix(prefix.clone());
        let reg = Registration {
            name: "old-lead".parse().unwrap(),
            machine: "draco".parse().unwrap(),
            kind: "codex".into(),
            session_id: registered_session.map(|s| s.parse().unwrap()),
            cwd: None,
            herdr_pane: Some("wD:p1".parse().unwrap()),
            place: None,
            last_seen_ms: 1,
        };
        relay.register(&reg).unwrap();
        let before = serde_json::to_value(&reg).unwrap();
        let scratch = Scratch::new("ix-relay-stale-identity-");
        let agent_case = live_session.map_or_else(String::new, |session| {
            format!(r#""wD:p1") echo '{{"result":{{"agent":{{"agent_session":{{"kind":"id","value":"{session}"}}}}}}}}' ;;"#)
        });
        let herdr = fake_herdr_dir("[]", "[]", "[]", &agent_case, &scratch.join("calls"), fail);
        for args in [
            vec!["whoami", "--json"],
            vec!["register", "--kind", "codex"],
            vec!["send", "old-lead", "must not be queued"],
        ] {
            let mut cmd = base_cmd(&args, None);
            cmd.env("IX_RELAY_URL", &url)
                .env("IX_RELAY_PREFIX", &prefix)
                .env("IX_RELAY_MACHINE", "draco")
                .env("HERDR_PANE_ID", "wD:p1")
                .env("HERDR_TAB_ID", "wD:t1")
                .env("HERDR_WORKSPACE_ID", "wD")
                .env(
                    "PATH",
                    format!("{}:{}", herdr.display(), std::env::var("PATH").unwrap()),
                );
            if let Some(caller) = caller {
                cmd.env("CODEX_THREAD_ID", caller);
            }
            let result = run(cmd, "");
            if args[0] == "whoami" {
                assert_eq!(result.status, 0, "{case}: {}", result.stderr);
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(&result.stdout).unwrap(),
                    serde_json::json!({"lead": false})
                );
            } else {
                assert_ne!(result.status, 0, "{case}: {}", result.stdout);
            }
            assert!(
                result
                    .stderr
                    .contains("manual registration cannot repair it"),
                "{case}: {}",
                result.stderr
            );
            let AgentSlot::Registered(after) = relay.agent_slot(&reg.name).unwrap() else {
                panic!("holder missing")
            };
            assert_eq!(
                serde_json::to_value(after).unwrap(),
                before,
                "{case}: registry mutated"
            );
            assert_eq!(
                relay.inbox_len(&reg.name).unwrap(),
                0,
                "{case}: message queued"
            );
        }
    }
}

#[test]
fn a_codex_caller_cannot_derive_another_live_sessions_name_before_registration() {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!("skipping: IX_RELAY_TEST_URL unset");
        return;
    };
    let scratch = Scratch::new("ix-relay-caller-mismatch-");
    let herdr = fake_herdr_dir(
        r#"[{"workspace_id":"wD","label":"other"}]"#,
        r#"[{"tab_id":"wD:t1","label":"1","number":1}]"#,
        r#"[{"pane_id":"wD:p1","tab_id":"wD:t1"}]"#,
        r#""wD:p1") echo '{"result":{"agent":{"agent_session":{"kind":"id","value":"terminal"}}}}' ;;"#,
        &scratch.join("calls"),
        false,
    );
    let mut cmd = base_cmd(&["register", "--kind", "codex"], None);
    let prefix = format!("test:{}:caller-mismatch:", std::process::id());
    cmd.env("IX_RELAY_URL", &url)
        .env("IX_RELAY_PREFIX", &prefix)
        .env("CODEX_THREAD_ID", "api")
        .env("HERDR_PANE_ID", "wD:p1")
        .env("HERDR_TAB_ID", "wD:t1")
        .env("HERDR_WORKSPACE_ID", "wD")
        .env(
            "PATH",
            format!("{}:{}", herdr.display(), std::env::var("PATH").unwrap()),
        );
    let result = run(cmd, "");
    assert_ne!(result.status, 0, "{}", result.stdout);
    let mut relay = Relay::connect(&url, Duration::from_secs(3))
        .unwrap()
        .with_prefix(prefix.clone());
    assert!(matches!(
        relay.agent_slot(&"other".parse().unwrap()).unwrap(),
        AgentSlot::Absent
    ));
}

#[test]
fn verified_codex_caller_ignores_a_newer_stale_record_for_the_same_pane() {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!("skipping: IX_RELAY_TEST_URL unset");
        return;
    };
    let prefix = format!("test:{}:verified-caller:", std::process::id());
    let mut relay = Relay::connect(&url, Duration::from_secs(3))
        .unwrap()
        .with_prefix(prefix.clone());
    for (name, session, last_seen_ms) in [("current-lead", "terminal", 1), ("old-lead", "old", 2)] {
        relay
            .register(&Registration {
                name: name.parse().unwrap(),
                machine: "draco".parse().unwrap(),
                kind: "codex".into(),
                session_id: Some(session.parse().unwrap()),
                cwd: None,
                herdr_pane: Some("wD:p1".parse().unwrap()),
                place: None,
                last_seen_ms,
            })
            .unwrap();
    }
    let scratch = Scratch::new("ix-relay-verified-caller-");
    let herdr = fake_herdr_dir(
        "[]",
        "[]",
        "[]",
        r#""wD:p1") echo '{"result":{"agent":{"agent_session":{"kind":"id","value":"terminal"}}}}' ;;"#,
        &scratch.join("calls"),
        false,
    );
    let mut cmd = base_cmd(&["whoami", "--json"], None);
    cmd.env("IX_RELAY_URL", &url)
        .env("IX_RELAY_PREFIX", &prefix)
        .env("IX_RELAY_MACHINE", "draco")
        .env("CODEX_THREAD_ID", "terminal")
        .env("HERDR_PANE_ID", "wD:p1")
        .env("HERDR_TAB_ID", "wD:t1")
        .env("HERDR_WORKSPACE_ID", "wD")
        .env(
            "PATH",
            format!("{}:{}", herdr.display(), std::env::var("PATH").unwrap()),
        );
    let result = run(cmd, "");
    assert_eq!(result.status, 0, "{}", result.stderr);
    let identity: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(identity["name"], "current-lead");
    assert_eq!(identity["source"], "registry");
}
