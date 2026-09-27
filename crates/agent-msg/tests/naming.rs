//! End-to-end tests for AGE-2018's automatic bus names, run against the
//! built binary with a fake `herdr` stand-in first on `PATH` (see
//! `support::fake_herdr_dir`) — no real herdr session needed. Redis-backed
//! (need `IX_RELAY_TEST_URL`; `make test-redis` sets it) and skip with a
//! clear message without it, same as `tests/hook.rs`.
//!
//! Covers what the hook path must do without `AGENT_MSG_NAME`:
//! - derive and register at `SessionStart` (one herdr call per location
//!   question: workspace, tab, pane);
//! - look the name up from the session cache at a later event, with *no*
//!   herdr call at all;
//! - print nothing, still exit 0 and record the failure once, when herdr itself
//!   fails.

#![expect(
    clippy::unwrap_used,
    reason = "test code: a failed unwrap is the failure signal"
)]

mod support;

use support::{agent_msg, agent_msg_with_herdr, fake_herdr_dir};

const SESSION_START: &str =
    r#"{"hook_event_name":"SessionStart","session_id":"naming-s1","cwd":"/work"}"#;
const POST_TOOL_USE: &str =
    r#"{"hook_event_name":"PostToolUse","session_id":"naming-s1","cwd":"/work"}"#;

/// A workspace labelled `QSL`, one unlabelled first tab, one pane: derives
/// to `qsl` per AGE-2018's naming rule.
const WORKSPACES_JSON: &str = r#"[{"workspace_id":"wQ","label":"QSL","number":1}]"#;
const TABS_JSON: &str = r#"[{"tab_id":"wQ:t1","label":"1","number":1}]"#;
const PANES_JSON: &str = r#"[{"pane_id":"wQ:p1","tab_id":"wQ:t1"}]"#;

/// Skips with a clear message if `make test-redis` didn't set the URL.
fn test_url() -> Option<String> {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
        return None;
    };
    Some(url)
}

fn call_count(call_log: &std::path::Path) -> usize {
    std::fs::read_to_string(call_log)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.is_empty())
        .count()
}

#[test]
fn session_start_derives_and_registers_with_no_agent_msg_name() {
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:naming-session-start:", std::process::id());
    let scratch =
        std::env::temp_dir().join(format!("ix-relay-naming-test-{}-a", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let call_log = scratch.join("calls.log");
    let herdr_dir = fake_herdr_dir(WORKSPACES_JSON, TABS_JSON, PANES_JSON, "", &call_log, false);

    let r = agent_msg_with_herdr(
        &["hook", "claude"],
        Some(&url),
        Some(&prefix),
        &herdr_dir,
        "wQ:p1",
        "wQ:t1",
        "wQ",
        SESSION_START,
    );
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);

    // Registered under the derived name "qsl" — visible via `who`.
    let who = agent_msg(&["who"], Some("qsl"), Some(&url), Some(&prefix), "");
    assert_eq!(who.status, 0, "stderr: {}", who.stderr);
    assert!(who.stdout.contains("qsl"), "stdout: {}", who.stdout);

    // Exactly one call each to workspace/tab/pane list, plus one `agent
    // get` (tier 3 of the name-precedence chain, which reports no override
    // here — see WORKSPACES_JSON/TABS_JSON/PANES_JSON's own doc comment).
    assert_eq!(
        call_count(&call_log),
        4,
        "expected one workspace/tab/pane list call each, plus one agent get; log:\n{}",
        std::fs::read_to_string(&call_log).unwrap_or_default()
    );

    // A message sent to "qsl" is delivered on the derived name's own next
    // hook call, proving delivery works end to end under the derived name.
    let sent = agent_msg(
        &["send", "qsl", "derived-name-delivery"],
        Some("sender"),
        Some(&url),
        Some(&prefix),
        "",
    );
    assert_eq!(sent.status, 0, "stderr: {}", sent.stderr);

    std::fs::remove_dir_all(&scratch).ok();
    std::fs::remove_dir_all(&herdr_dir).ok();
}

#[test]
fn post_tool_use_uses_the_cached_name_with_no_further_herdr_call() {
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:naming-cache:", std::process::id());
    let scratch =
        std::env::temp_dir().join(format!("ix-relay-naming-test-{}-b", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let call_log = scratch.join("calls.log");
    let herdr_dir = fake_herdr_dir(WORKSPACES_JSON, TABS_JSON, PANES_JSON, "", &call_log, false);

    // SessionStart derives and caches "qsl" against this session id.
    let r = agent_msg_with_herdr(
        &["hook", "claude"],
        Some(&url),
        Some(&prefix),
        &herdr_dir,
        "wQ:p1",
        "wQ:t1",
        "wQ",
        SESSION_START,
    );
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);
    let after_session_start = call_count(&call_log);
    assert_eq!(after_session_start, 4);

    // A message queued for "qsl" before the next hook call.
    let sent = agent_msg(
        &["send", "qsl", "postissue-check"],
        Some("sender"),
        Some(&url),
        Some(&prefix),
        "",
    );
    assert_eq!(sent.status, 0, "stderr: {}", sent.stderr);

    // PostToolUse, same session id, no AGENT_MSG_NAME: must deliver via the
    // cached name, with NO new herdr call — the fake herdr's call log must
    // not grow past the 3 calls SessionStart already made.
    let r = agent_msg_with_herdr(
        &["hook", "claude"],
        Some(&url),
        Some(&prefix),
        &herdr_dir,
        "wQ:p1",
        "wQ:t1",
        "wQ",
        POST_TOOL_USE,
    );
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);
    let out: serde_json::Value = serde_json::from_str(&r.stdout).expect("json stdout");
    let ctx = out["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .expect("additionalContext string");
    assert!(ctx.contains("postissue-check"), "context: {ctx}");
    assert_eq!(
        call_count(&call_log),
        after_session_start,
        "PostToolUse must use the session->name cache, not call herdr again"
    );

    std::fs::remove_dir_all(&scratch).ok();
    std::fs::remove_dir_all(&herdr_dir).ok();
}

/// Runs one hook as a lead in pane `wQ:p1` with `herdr_dir` first on `PATH`, its
/// spool in `config_dir`.
fn hook_in_pane(
    url: &str,
    prefix: &str,
    herdr_dir: &std::path::Path,
    config_dir: &std::path::Path,
    stdin: &str,
) -> support::Run {
    let mut cmd = support::base_cmd(&["hook", "claude"], None);
    cmd.env("IX_RELAY_URL", url)
        .env("IX_RELAY_PREFIX", prefix)
        .env("IX_RELAY_CONFIG", config_dir.join("config.toml"))
        .env("HERDR_PANE_ID", "wQ:p1")
        .env("HERDR_TAB_ID", "wQ:t1")
        .env("HERDR_WORKSPACE_ID", "wQ")
        .env(
            "PATH",
            format!(
                "{}:{}",
                herdr_dir.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        );
    support::run(cmd, stdin)
}

/// The `(stage, error_kind)` of every record in the spool of `config_dir`.
fn spooled(config_dir: &std::path::Path) -> Vec<(String, String)> {
    std::fs::read_to_string(support::spool_path(&config_dir.join("config.toml")))
        .unwrap_or_default()
        .lines()
        .map(|line| {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            let field = |name: &str| v[name].as_str().unwrap().to_owned();
            (field("stage"), field("error_kind"))
        })
        .collect()
}

/// A failure of herdr as the hook records it: `herdr_failed`, or `budget` when a
/// herdr call was refused for want of time on a machine too slow for the hook's
/// budget (AGE-2059).
fn is_herdr_failure(record: &(String, String)) -> bool {
    matches!(
        (record.0.as_str(), record.1.as_str()),
        ("derive", "herdr_failed") | ("budget", "budget_exhausted")
    )
}

/// Tracing: TC-030-10
/// ACs: FR-030-AC-10
///
/// A hook in a herdr pane whose herdr fails every call is not a non-lead: it
/// prints nothing, registers nothing, exits 0 and leaves one record (AGE-2059).
#[test]
fn tc_030_10_herdr_failure_at_session_start_records_one_failure_and_registers_nothing() {
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:naming-herdr-fail:", std::process::id());
    let scratch =
        std::env::temp_dir().join(format!("ix-relay-naming-test-{}-c", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let call_log = scratch.join("calls.log");
    let herdr_dir = fake_herdr_dir(WORKSPACES_JSON, TABS_JSON, PANES_JSON, "", &call_log, true);
    let config = tempfile::tempdir().unwrap();

    let r = hook_in_pane(
        &url,
        &prefix,
        &herdr_dir,
        config.path(),
        r#"{"hook_event_name":"SessionStart","session_id":"naming-fail-s1","cwd":"/work"}"#,
    );
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);
    assert_eq!(r.stdout, "", "a failed derivation must print nothing");
    let records = spooled(config.path());
    assert!(
        records.len() == 1 && is_herdr_failure(&records[0]),
        "one herdr failure record: {records:?}"
    );

    // Nothing was registered.
    let who = agent_msg(&["who"], Some("qsl"), Some(&url), Some(&prefix), "");
    assert_eq!(who.status, 0, "stderr: {}", who.stderr);
    assert!(
        !who.stdout.contains("qsl "),
        "qsl must not have been registered: {}",
        who.stdout
    );

    std::fs::remove_dir_all(&scratch).ok();
    std::fs::remove_dir_all(&herdr_dir).ok();
}

/// Tracing: TC-030-11
/// ACs: FR-030-AC-11
///
/// Uncached `PostToolUse` calls while herdr fails leave one record between them
/// and each asks herdr again; the first call after herdr answers derives and
/// caches the lead's name (a `unresolved:` mark would have held it off).
#[test]
fn tc_030_11_uncached_post_tool_use_calls_record_a_herdr_outage_once_and_retry() {
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:naming-herdr-outage:", std::process::id());
    let scratch =
        std::env::temp_dir().join(format!("ix-relay-naming-test-{}-o", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let call_log = scratch.join("calls.log");
    let broken = fake_herdr_dir(WORKSPACES_JSON, TABS_JSON, PANES_JSON, "", &call_log, true);
    let config = tempfile::tempdir().unwrap();

    let mut calls = 0;
    for call in 1..=3 {
        let r = hook_in_pane(&url, &prefix, &broken, config.path(), POST_TOOL_USE);
        assert_eq!(r.status, 0, "stderr: {}", r.stderr);
        assert_eq!(r.stdout, "");
        let now = call_count(&call_log);
        assert!(
            now > calls,
            "call {call} asked herdr again ({now} <= {calls})"
        );
        calls = now;
    }
    let records = spooled(config.path());
    assert!(
        records.len() == 1 && is_herdr_failure(&records[0]),
        "one record for the whole outage: {records:?}"
    );

    // herdr answers again.
    let healthy = fake_herdr_dir(WORKSPACES_JSON, TABS_JSON, PANES_JSON, "", &call_log, false);
    let r = hook_in_pane(&url, &prefix, &healthy, config.path(), POST_TOOL_USE);
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);
    let mut relay = relay_core::Relay::connect(&url, std::time::Duration::from_secs(5))
        .unwrap()
        .with_prefix(&prefix);
    let probe = relay.probe_session(&"naming-s1".parse().unwrap()).unwrap();
    assert_eq!(
        probe.name,
        Some("qsl".parse().unwrap()),
        "the first call after the outage derived the lead"
    );

    std::fs::remove_dir_all(&scratch).ok();
    std::fs::remove_dir_all(&broken).ok();
    std::fs::remove_dir_all(&healthy).ok();
}

#[test]
fn herdr_agent_name_outranks_the_location_name_and_the_location_becomes_an_alias() {
    // The operator's 2026-09-25 correction to AGE-2018: a herdr agent name (tier 3)
    // outranks the plain herdr-location name (tier 4). The location name
    // ("qsl") must still be registered, as an alias of the winner, so a
    // message sent to it still arrives.
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:naming-herdr-agent-name:", std::process::id());
    let scratch =
        std::env::temp_dir().join(format!("ix-relay-naming-test-{}-e", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let call_log = scratch.join("calls.log");
    let agent_get_cases = r#""wQ:p1") echo '{"result":{"agent":{"name":"custom-herdr-name"}}}' ;;"#;
    let herdr_dir = fake_herdr_dir(
        WORKSPACES_JSON,
        TABS_JSON,
        PANES_JSON,
        agent_get_cases,
        &call_log,
        false,
    );

    let r = agent_msg_with_herdr(
        &["hook", "claude"],
        Some(&url),
        Some(&prefix),
        &herdr_dir,
        "wQ:p1",
        "wQ:t1",
        "wQ",
        SESSION_START,
    );
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);

    // The herdr agent name won: registered under "custom-herdr-name".
    let who = agent_msg(
        &["who"],
        Some("custom-herdr-name"),
        Some(&url),
        Some(&prefix),
        "",
    );
    assert_eq!(who.status, 0, "stderr: {}", who.stderr);
    assert!(
        who.stdout.contains("custom-herdr-name"),
        "stdout: {}",
        who.stdout
    );
    assert!(
        who.stdout.contains("aka qsl"),
        "the location name must be listed as an alias: {}",
        who.stdout
    );

    // A message sent to the *old* (location) name still arrives, via the
    // alias.
    let sent = agent_msg(
        &["send", "qsl", "via-the-old-location-alias"],
        Some("sender"),
        Some(&url),
        Some(&prefix),
        "",
    );
    assert_eq!(sent.status, 0, "stderr: {}", sent.stderr);
    let inbox = agent_msg(
        &["inbox"],
        Some("custom-herdr-name"),
        Some(&url),
        Some(&prefix),
        "",
    );
    assert_eq!(inbox.status, 0, "stderr: {}", inbox.stderr);
    assert!(
        inbox.stdout.contains("via-the-old-location-alias"),
        "stdout: {}",
        inbox.stdout
    );

    std::fs::remove_dir_all(&scratch).ok();
    std::fs::remove_dir_all(&herdr_dir).ok();
}

#[test]
fn user_prompt_submit_re_derives_so_a_mid_session_rename_takes_effect() {
    // The operator's 2026-09-25 correction, finding 6: UserPromptSubmit re-derives
    // (so a `/rename` takes effect on the very next prompt) but makes *no
    // herdr calls at all* — only the transcript is re-scanned. This test's
    // "rename" is therefore a `custom-title` line landing in the session's
    // transcript between SessionStart and the next UserPromptSubmit, not a
    // herdr-side change (the fake herdr's call log is asserted flat across
    // both hook calls below, proving no extra herdr call happened).
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:naming-rename-mid-session:", std::process::id());
    let scratch =
        std::env::temp_dir().join(format!("ix-relay-naming-test-{}-f", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let call_log = scratch.join("calls.log");
    let herdr_dir = fake_herdr_dir(WORKSPACES_JSON, TABS_JSON, PANES_JSON, "", &call_log, false);

    let r = agent_msg_with_herdr(
        &["hook", "claude"],
        Some(&url),
        Some(&prefix),
        &herdr_dir,
        "wQ:p1",
        "wQ:t1",
        "wQ",
        SESSION_START,
    );
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);
    let who = agent_msg(&["who"], Some("qsl"), Some(&url), Some(&prefix), "");
    assert!(who.stdout.contains("qsl"), "stdout: {}", who.stdout);
    let calls_after_session_start = call_count(&call_log);
    assert!(
        calls_after_session_start > 0,
        "SessionStart must call herdr at least once"
    );

    // A `/rename` mid-session: a real transcript file with a `custom-title`
    // line, referenced by `transcript_path` in the next hook call — never a
    // herdr call.
    let transcript = scratch.join("transcript.jsonl");
    std::fs::write(
        &transcript,
        r#"{"type":"custom-title","customTitle":"renamed-mid-session","sessionId":"naming-s1"}
"#,
    )
    .unwrap();

    let r = agent_msg_with_herdr(
        &["hook", "claude"],
        Some(&url),
        Some(&prefix),
        &herdr_dir,
        "wQ:p1",
        "wQ:t1",
        "wQ",
        &format!(
            r#"{{"hook_event_name":"UserPromptSubmit","session_id":"naming-s1","cwd":"/work","transcript_path":{:?}}}"#,
            transcript.to_str().unwrap()
        ),
    );
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);
    assert_eq!(
        call_count(&call_log),
        calls_after_session_start,
        "UserPromptSubmit must make no herdr calls at all"
    );

    let who = agent_msg(
        &["who"],
        Some("renamed-mid-session"),
        Some(&url),
        Some(&prefix),
        "",
    );
    assert_eq!(who.status, 0, "stderr: {}", who.stderr);
    assert!(
        who.stdout.contains("renamed-mid-session"),
        "the new name must be registered on the bus: {}",
        who.stdout
    );
    assert!(
        who.stdout.contains("aka qsl"),
        "the old location name must still work, as an alias: {}",
        who.stdout
    );

    // A message to the *old* name still arrives, via the alias.
    let sent = agent_msg(
        &["send", "qsl", "still-reachable-by-the-old-name"],
        Some("sender"),
        Some(&url),
        Some(&prefix),
        "",
    );
    assert_eq!(sent.status, 0, "stderr: {}", sent.stderr);
    let inbox = agent_msg(
        &["inbox"],
        Some("renamed-mid-session"),
        Some(&url),
        Some(&prefix),
        "",
    );
    assert!(
        inbox.stdout.contains("still-reachable-by-the-old-name"),
        "stdout: {}",
        inbox.stdout
    );

    std::fs::remove_dir_all(&scratch).ok();
    std::fs::remove_dir_all(&herdr_dir).ok();
}

#[test]
fn a_rename_onto_a_name_another_lead_holds_is_widened_not_stolen() {
    // AGE-2026: `/rename` used to skip collision resolution (only SessionStart
    // ran it), so renaming onto a live lead's name overwrote that lead's
    // registration. Registry reads only: no herdr call at UserPromptSubmit.
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:naming-rename-collision:", std::process::id());
    let scratch =
        std::env::temp_dir().join(format!("ix-relay-naming-test-{}-rc", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let call_log = scratch.join("calls.log");
    let herdr_dir = fake_herdr_dir(WORKSPACES_JSON, TABS_JSON, PANES_JSON, "", &call_log, false);

    let mut relay = relay_core::Relay::connect(&url, std::time::Duration::from_secs(2))
        .unwrap()
        .with_prefix(prefix.clone());
    relay
        .register(&relay_core::Registration {
            name: "taken-name".parse().unwrap(),
            machine: "some-other-machine".parse().unwrap(),
            kind: "claude".into(),
            session_id: Some("other-sid".parse().unwrap()),
            cwd: None,
            herdr_pane: Some("wZ:p1".parse().unwrap()),
            place: None,
            last_seen_ms: 1,
        })
        .unwrap();

    let r = agent_msg_with_herdr(
        &["hook", "claude"],
        Some(&url),
        Some(&prefix),
        &herdr_dir,
        "wQ:p1",
        "wQ:t1",
        "wQ",
        SESSION_START,
    );
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);
    let calls_after_session_start = call_count(&call_log);

    let transcript = scratch.join("transcript.jsonl");
    std::fs::write(
        &transcript,
        "{\"type\":\"custom-title\",\"customTitle\":\"taken-name\",\"sessionId\":\"naming-s1\"}\n",
    )
    .unwrap();
    let r = agent_msg_with_herdr(
        &["hook", "claude"],
        Some(&url),
        Some(&prefix),
        &herdr_dir,
        "wQ:p1",
        "wQ:t1",
        "wQ",
        &format!(
            r#"{{"hook_event_name":"UserPromptSubmit","session_id":"naming-s1","cwd":"/work","transcript_path":{:?}}}"#,
            transcript.to_str().unwrap()
        ),
    );
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);
    assert_eq!(call_count(&call_log), calls_after_session_start);

    let other = relay
        .agent(&"taken-name".parse().unwrap())
        .unwrap()
        .expect("the other lead is still registered");
    assert_eq!(
        other.session_id.as_ref().map(|v| v.as_str()),
        Some("other-sid"),
        "not overwritten"
    );
    let mine = relay
        .agents()
        .unwrap()
        .registrations
        .into_iter()
        .find(|r| r.session_id.as_ref().map(|v| v.as_str()) == Some("naming-s1"))
        .expect("this session is registered");
    assert!(
        mine.name.as_str().starts_with("taken-name-"),
        "widened by machine, got {}",
        mine.name
    );

    std::fs::remove_dir_all(&scratch).ok();
    std::fs::remove_dir_all(&herdr_dir).ok();
}

#[test]
fn a_rename_onto_a_session_less_registration_does_not_overwrite_it() {
    // AGE-2026 review: lead A registered by `agent-msg register` has no
    // session id; a `/rename` to its name used to be treated as "stale, take
    // it over", leaving two leads on one inbox.
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:naming-rename-sessionless:", std::process::id());
    let scratch =
        std::env::temp_dir().join(format!("ix-relay-naming-test-{}-rs", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let call_log = scratch.join("calls.log");
    let herdr_dir = fake_herdr_dir(WORKSPACES_JSON, TABS_JSON, PANES_JSON, "", &call_log, false);

    let a = agent_msg(
        &["register"],
        Some("shared-name"),
        Some(&url),
        Some(&prefix),
        "",
    );
    assert_eq!(a.status, 0, "stderr: {}", a.stderr);

    let r = agent_msg_with_herdr(
        &["hook", "claude"],
        Some(&url),
        Some(&prefix),
        &herdr_dir,
        "wQ:p1",
        "wQ:t1",
        "wQ",
        SESSION_START,
    );
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);
    let transcript = scratch.join("transcript.jsonl");
    std::fs::write(
        &transcript,
        "{\"type\":\"custom-title\",\"customTitle\":\"shared-name\",\"sessionId\":\"naming-s1\"}\n",
    )
    .unwrap();
    let r = agent_msg_with_herdr(
        &["hook", "claude"],
        Some(&url),
        Some(&prefix),
        &herdr_dir,
        "wQ:p1",
        "wQ:t1",
        "wQ",
        &format!(
            r#"{{"hook_event_name":"UserPromptSubmit","session_id":"naming-s1","cwd":"/work","transcript_path":{:?}}}"#,
            transcript.to_str().unwrap()
        ),
    );
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);

    let mut relay = relay_core::Relay::connect(&url, std::time::Duration::from_secs(2))
        .unwrap()
        .with_prefix(prefix.clone());
    let a_reg = relay
        .agent(&"shared-name".parse().unwrap())
        .unwrap()
        .expect("A's registration survives");
    assert_eq!(a_reg.session_id, None, "still A's, not overwritten by B");
    let b_reg = relay
        .agents()
        .unwrap()
        .registrations
        .into_iter()
        .find(|r| r.session_id.as_ref().map(|v| v.as_str()) == Some("naming-s1"))
        .expect("B is registered");
    assert_ne!(b_reg.name.as_str(), "shared-name");
    assert!(
        b_reg.name.as_str().starts_with("shared-name-"),
        "{}",
        b_reg.name
    );

    std::fs::remove_dir_all(&scratch).ok();
    std::fs::remove_dir_all(&herdr_dir).ok();
}

#[test]
fn unresolvable_location_is_silent_not_an_error() {
    // The pane/tab/workspace ids the process reports don't appear in what
    // herdr lists (a mismatch between env and reality) — must fail closed,
    // same as any other herdr failure, never guess.
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:naming-unresolvable:", std::process::id());
    let scratch =
        std::env::temp_dir().join(format!("ix-relay-naming-test-{}-d", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let call_log = scratch.join("calls.log");
    let herdr_dir = fake_herdr_dir(WORKSPACES_JSON, TABS_JSON, PANES_JSON, "", &call_log, false);

    let r = agent_msg_with_herdr(
        &["hook", "claude"],
        Some(&url),
        Some(&prefix),
        &herdr_dir,
        "wZ:p9",
        "wZ:t9",
        "wZ",
        r#"{"hook_event_name":"SessionStart","session_id":"naming-unresolvable-s1","cwd":"/work"}"#,
    );
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);
    assert_eq!(r.stdout, "");

    std::fs::remove_dir_all(&scratch).ok();
    std::fs::remove_dir_all(&herdr_dir).ok();
}

// ---- forks and escalation investigations never join the bus (finding 1) --

/// A tab labelled `esc-*`: an escalation investigation fork. Must never
/// register, under any precedence tier, and must remember that so later
/// hook calls in the same session stay silent without re-deriving.
#[test]
fn an_esc_prefixed_tab_never_joins_the_bus_and_stays_silent_on_later_hooks() {
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:naming-esc-tab:", std::process::id());
    let scratch = std::env::temp_dir().join(format!(
        "ix-relay-naming-test-{}-esc-tab",
        std::process::id()
    ));
    std::fs::create_dir_all(&scratch).unwrap();
    let call_log = scratch.join("calls.log");
    let esc_tabs_json = r#"[{"tab_id":"wQ:t1","label":"esc-e7-1","number":1}]"#;
    let herdr_dir = fake_herdr_dir(
        WORKSPACES_JSON,
        esc_tabs_json,
        PANES_JSON,
        "",
        &call_log,
        false,
    );

    let r = agent_msg_with_herdr(
        &["hook", "claude"],
        Some(&url),
        Some(&prefix),
        &herdr_dir,
        "wQ:p1",
        "wQ:t1",
        "wQ",
        r#"{"hook_event_name":"SessionStart","session_id":"naming-esc-s1","cwd":"/work"}"#,
    );
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);
    assert_eq!(
        r.stdout, "",
        "an escalation fork must produce no hook output"
    );
    let who = agent_msg(
        &["who"],
        Some("someone-else"),
        Some(&url),
        Some(&prefix),
        "",
    );
    assert!(
        !who.stdout.contains("esc"),
        "nothing must be registered: {}",
        who.stdout
    );

    // A later hook in the same session, even with herdr now failing
    // outright, must stay silent purely from the remembered suppression —
    // never attempt to re-derive.
    let failing_herdr_dir = fake_herdr_dir(
        WORKSPACES_JSON,
        esc_tabs_json,
        PANES_JSON,
        "",
        &call_log,
        true,
    );
    let r = agent_msg_with_herdr(
        &["hook", "claude"],
        Some(&url),
        Some(&prefix),
        &failing_herdr_dir,
        "wQ:p1",
        "wQ:t1",
        "wQ",
        POST_TOOL_USE.replace("naming-s1", "naming-esc-s1").as_str(),
    );
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);
    assert_eq!(r.stdout, "");

    std::fs::remove_dir_all(&scratch).ok();
    std::fs::remove_dir_all(&herdr_dir).ok();
    std::fs::remove_dir_all(&failing_herdr_dir).ok();
}

/// A herdr agent name starting with `esc-` (rather than the tab label) also
/// suppresses the whole session.
#[test]
fn an_esc_prefixed_herdr_agent_name_never_joins_the_bus() {
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:naming-esc-agent-name:", std::process::id());
    let scratch = std::env::temp_dir().join(format!(
        "ix-relay-naming-test-{}-esc-agent-name",
        std::process::id()
    ));
    std::fs::create_dir_all(&scratch).unwrap();
    let call_log = scratch.join("calls.log");
    let agent_get_cases = r#""wQ:p1") echo '{"result":{"agent":{"name":"esc-e9-1"}}}' ;;"#;
    let herdr_dir = fake_herdr_dir(
        WORKSPACES_JSON,
        TABS_JSON,
        PANES_JSON,
        agent_get_cases,
        &call_log,
        false,
    );

    let r = agent_msg_with_herdr(
        &["hook", "claude"],
        Some(&url),
        Some(&prefix),
        &herdr_dir,
        "wQ:p1",
        "wQ:t1",
        "wQ",
        r#"{"hook_event_name":"SessionStart","session_id":"naming-esc-agent-s1","cwd":"/work"}"#,
    );
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);
    assert_eq!(r.stdout, "");
    let who = agent_msg(
        &["who"],
        Some("someone-else"),
        Some(&url),
        Some(&prefix),
        "",
    );
    assert!(!who.stdout.contains("esc-e9-1"), "stdout: {}", who.stdout);
    assert!(
        !who.stdout.contains("qsl"),
        "the plain location name must not have registered either: {}",
        who.stdout
    );

    std::fs::remove_dir_all(&scratch).ok();
    std::fs::remove_dir_all(&herdr_dir).ok();
}

/// A Claude `--fork-session` fork (`"source":"fork"` in `SessionStart`'s own
/// payload) never joins the bus, even with an otherwise perfectly ordinary
/// location.
#[test]
fn a_fork_session_never_joins_the_bus() {
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:naming-fork:", std::process::id());
    let scratch =
        std::env::temp_dir().join(format!("ix-relay-naming-test-{}-fork", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let call_log = scratch.join("calls.log");
    let herdr_dir = fake_herdr_dir(WORKSPACES_JSON, TABS_JSON, PANES_JSON, "", &call_log, false);

    let r = agent_msg_with_herdr(
        &["hook", "claude"],
        Some(&url),
        Some(&prefix),
        &herdr_dir,
        "wQ:p1",
        "wQ:t1",
        "wQ",
        r#"{"hook_event_name":"SessionStart","session_id":"naming-fork-s1","cwd":"/work","source":"fork"}"#,
    );
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);
    assert_eq!(r.stdout, "");
    let who = agent_msg(
        &["who"],
        Some("someone-else"),
        Some(&url),
        Some(&prefix),
        "",
    );
    assert!(!who.stdout.contains("qsl"), "stdout: {}", who.stdout);

    std::fs::remove_dir_all(&scratch).ok();
    std::fs::remove_dir_all(&herdr_dir).ok();
}

// ---- interactive commands resolve by pane from the registry (finding 4) --

/// `agent-msg whoami`/`send` run directly in the agent's own shell (not
/// through a hook) must see a `/rename` the hook already published — and
/// must make no herdr call to do it (proven here by giving them a herdr
/// that always fails: if the interactive path fell back to re-deriving
/// through herdr, this would surface as an error or "not a lead", not the
/// renamed name).
#[test]
fn interactive_commands_resolve_a_rename_from_the_registry_with_no_herdr_call() {
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:naming-interactive-registry:", std::process::id());
    let scratch = std::env::temp_dir().join(format!(
        "ix-relay-naming-test-{}-interactive",
        std::process::id()
    ));
    std::fs::create_dir_all(&scratch).unwrap();
    let call_log = scratch.join("calls.log");
    let herdr_dir = fake_herdr_dir(WORKSPACES_JSON, TABS_JSON, PANES_JSON, "", &call_log, false);

    // SessionStart derives "qsl" and registers it.
    let r = agent_msg_with_herdr(
        &["hook", "claude"],
        Some(&url),
        Some(&prefix),
        &herdr_dir,
        "wQ:p1",
        "wQ:t1",
        "wQ",
        r#"{"hook_event_name":"SessionStart","session_id":"naming-interactive-s1","cwd":"/work"}"#,
    );
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);

    // A /rename, via UserPromptSubmit + transcript, same as the mid-session
    // rename test above.
    let transcript = scratch.join("transcript.jsonl");
    std::fs::write(
        &transcript,
        r#"{"type":"custom-title","customTitle":"interactive-renamed","sessionId":"naming-interactive-s1"}
"#,
    )
    .unwrap();
    let r = agent_msg_with_herdr(
        &["hook", "claude"],
        Some(&url),
        Some(&prefix),
        &herdr_dir,
        "wQ:p1",
        "wQ:t1",
        "wQ",
        &format!(
            r#"{{"hook_event_name":"UserPromptSubmit","session_id":"naming-interactive-s1","cwd":"/work","transcript_path":{:?}}}"#,
            transcript.to_str().unwrap()
        ),
    );
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);

    // Now `whoami`, run directly (no AGENT_MSG_NAME, no hook JSON), with a
    // herdr that always fails: must still report "interactive-renamed",
    // resolved from the registry by HERDR_PANE_ID, not "qsl" (which is all
    // a fresh herdr re-derive could ever produce, and would anyway fail
    // outright against this herdr).
    let failing_herdr_dir =
        fake_herdr_dir(WORKSPACES_JSON, TABS_JSON, PANES_JSON, "", &call_log, true);
    let whoami = agent_msg_with_herdr(
        &["whoami"],
        Some(&url),
        Some(&prefix),
        &failing_herdr_dir,
        "wQ:p1",
        "wQ:t1",
        "wQ",
        "",
    );
    assert_eq!(whoami.status, 0, "stderr: {}", whoami.stderr);
    assert!(
        whoami.stdout.contains("interactive-renamed"),
        "stdout: {}",
        whoami.stdout
    );
    assert!(
        whoami.stdout.contains("registry"),
        "must report the registry source: {}",
        whoami.stdout
    );

    // `send`, same story, and its target resolves through the registry too.
    let send = agent_msg_with_herdr(
        &["send", "interactive-renamed", "hi"],
        Some(&url),
        Some(&prefix),
        &failing_herdr_dir,
        "wQ:p1",
        "wQ:t1",
        "wQ",
        "",
    );
    assert_eq!(send.status, 0, "stderr: {}", send.stderr);
    assert!(send.stdout.contains("queued"), "stdout: {}", send.stdout);

    std::fs::remove_dir_all(&scratch).ok();
    std::fs::remove_dir_all(&herdr_dir).ok();
    std::fs::remove_dir_all(&failing_herdr_dir).ok();
}

// ---- a second rename keeps every earlier name reachable (finding 5) ------

/// a → b → c: sending to a, b or c must all reach c's inbox — `a` never
/// dangles pointed at the now-demoted `b`.
#[test]
fn a_second_rename_keeps_every_earlier_name_working_as_a_one_hop_alias() {
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:naming-chain-rename:", std::process::id());
    let scratch =
        std::env::temp_dir().join(format!("ix-relay-naming-test-{}-chain", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let call_log = scratch.join("calls.log");
    let herdr_dir = fake_herdr_dir(WORKSPACES_JSON, TABS_JSON, PANES_JSON, "", &call_log, false);

    // SessionStart: canonical is "qsl" (the location name, "a").
    let r = agent_msg_with_herdr(
        &["hook", "claude"],
        Some(&url),
        Some(&prefix),
        &herdr_dir,
        "wQ:p1",
        "wQ:t1",
        "wQ",
        r#"{"hook_event_name":"SessionStart","session_id":"naming-chain-s1","cwd":"/work"}"#,
    );
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);

    // First rename: qsl -> b.
    let transcript = scratch.join("transcript.jsonl");
    std::fs::write(
        &transcript,
        "{\"type\":\"custom-title\",\"customTitle\":\"b\",\"sessionId\":\"naming-chain-s1\"}\n",
    )
    .unwrap();
    let r = agent_msg_with_herdr(
        &["hook", "claude"],
        Some(&url),
        Some(&prefix),
        &herdr_dir,
        "wQ:p1",
        "wQ:t1",
        "wQ",
        &format!(
            r#"{{"hook_event_name":"UserPromptSubmit","session_id":"naming-chain-s1","cwd":"/work","transcript_path":{:?}}}"#,
            transcript.to_str().unwrap()
        ),
    );
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);

    // Second rename: b -> c.
    std::fs::write(
        &transcript,
        "{\"type\":\"custom-title\",\"customTitle\":\"b\",\"sessionId\":\"naming-chain-s1\"}\n\
         {\"type\":\"custom-title\",\"customTitle\":\"c\",\"sessionId\":\"naming-chain-s1\"}\n",
    )
    .unwrap();
    let r = agent_msg_with_herdr(
        &["hook", "claude"],
        Some(&url),
        Some(&prefix),
        &herdr_dir,
        "wQ:p1",
        "wQ:t1",
        "wQ",
        &format!(
            r#"{{"hook_event_name":"UserPromptSubmit","session_id":"naming-chain-s1","cwd":"/work","transcript_path":{:?}}}"#,
            transcript.to_str().unwrap()
        ),
    );
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);

    let who = agent_msg(
        &["who"],
        Some("someone-else"),
        Some(&url),
        Some(&prefix),
        "",
    );
    // These hooks did not observe a session-bound Herdr identity, so the
    // canonical lead remains explicitly unplaced until the watcher verifies it.
    assert!(
        who.stdout.contains("-  c  draco") && who.stdout.contains("location unverified"),
        "canonical must be \"c\" and its inherited pane claim must stay unverified: {}",
        who.stdout
    );
    assert!(
        who.stdout.contains("aka b, qsl"),
        "both earlier names must be listed as aliases: {}",
        who.stdout
    );

    for old_name in ["qsl", "b"] {
        let sent = agent_msg(
            &["send", old_name, &format!("via-{old_name}")],
            Some("sender"),
            Some(&url),
            Some(&prefix),
            "",
        );
        assert_eq!(
            sent.status, 0,
            "sending to {old_name} must still work: stderr: {}",
            sent.stderr
        );
    }
    let inbox = agent_msg(&["inbox"], Some("c"), Some(&url), Some(&prefix), "");
    assert!(inbox.stdout.contains("via-qsl"), "stdout: {}", inbox.stdout);
    assert!(inbox.stdout.contains("via-b"), "stdout: {}", inbox.stdout);

    std::fs::remove_dir_all(&scratch).ok();
    std::fs::remove_dir_all(&herdr_dir).ok();
}
