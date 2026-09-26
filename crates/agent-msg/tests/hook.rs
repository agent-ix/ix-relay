//! End-to-end tests for `agent-msg hook`, run against the built binary.
//!
//! Covers the property AGE-2007's review flagged as critical and untested: the
//! hook must exit 0 in every case, no matter what `AGENT_MSG_NAME` holds or
//! whether Redis answers. The Redis-backed cases below need `IX_RELAY_TEST_URL`
//! (`make test-redis` sets it) and skip with a clear message without it, same as
//! `relay-core`'s `tests/store.rs`.

mod support;

use support::{agent_msg, agent_msg_with_config, agent_msg_with_pane};

/// A real subagent call captured from Claude Code (AGE-2007 verify step): same
/// shape as a lead's call, but with `agent_id` set.
const SUBAGENT_PAYLOAD: &str = r#"{"session_id":"0202301c-34b9-4bd3-bf3c-320548aa5bc3","hook_event_name":"PostToolUse","tool_name":"Bash","tool_input":{"command":"echo sub"},"agent_id":"ac4ddbeeeea4eef75","agent_type":"general-purpose"}"#;

/// A real subagent call captured from Codex (AGE-2012 verify step, via a
/// `spawn_agent` subagent's own `PostToolUse` hook call): Codex marks it
/// exactly the same way Claude does, with a top-level `agent_id`, plus its
/// own `agent_type`.
const CODEX_SUBAGENT_PAYLOAD: &str = r#"{"session_id":"01a0d6ac-ec6a-7ba0-9ae9-dd543ce5a184","turn_id":"01a0d6ad-23a4-7fe1-996e-5c637a77c206","agent_id":"01a0d6ad-2383-7220-ae2e-d97441f51665","agent_type":"default","cwd":"/tmp/codex-probe-ws","hook_event_name":"PostToolUse","model":"gpt-5.6-hub","permission_mode":"bypassPermissions","tool_name":"Bash","tool_input":{"command":"echo subagent_ran"},"tool_response":"subagent_ran\n","tool_use_id":"exec-25327c05-c735-4b7f-9f3e-5b28d4ca7e48"}"#;

const SESSION_START: &str = r#"{"hook_event_name":"SessionStart","session_id":"s1","cwd":"/work"}"#;
const STOP: &str = r#"{"hook_event_name":"Stop","session_id":"s1","cwd":"/work"}"#;

fn hook(name: Option<&str>, url: Option<&str>, stdin: &str) -> support::Run {
    agent_msg(&["hook", "claude"], name, url, None, stdin)
}

fn hook_codex(name: Option<&str>, url: Option<&str>, stdin: &str) -> support::Run {
    agent_msg(&["hook", "codex"], name, url, None, stdin)
}

#[test]
fn unset_name_is_silent_and_exits_zero() {
    let r = hook(None, None, SESSION_START);
    assert_eq!(r.status, 0);
    assert_eq!(r.stdout, "");
}

#[test]
fn empty_name_is_silent_and_exits_zero() {
    let r = hook(Some(""), None, SESSION_START);
    assert_eq!(r.status, 0);
    assert_eq!(r.stdout, "");
}

#[test]
fn invalid_name_is_silent_and_exits_zero() {
    // Uppercase fails AgentName's herdr-style rule.
    let r = hook(Some("QSL-Lead"), None, SESSION_START);
    assert_eq!(r.status, 0);
    assert_eq!(r.stdout, "");
    assert!(
        r.stderr.contains("invalid agent name"),
        "stderr: {}",
        r.stderr
    );
}

#[test]
fn subagent_call_is_silent_and_exits_zero() {
    let r = hook(Some("qsl-lead"), None, SUBAGENT_PAYLOAD);
    assert_eq!(r.status, 0);
    assert_eq!(r.stdout, "");
}

#[test]
fn codex_subagent_call_is_silent_and_exits_zero() {
    let r = hook_codex(Some("qsl-lead"), None, CODEX_SUBAGENT_PAYLOAD);
    assert_eq!(r.status, 0);
    assert_eq!(r.stdout, "");
}

#[test]
fn dead_redis_still_exits_zero() {
    // Nothing listens on port 1: connection is refused immediately.
    let r = hook(Some("qsl-lead"), Some("redis://127.0.0.1:1"), SESSION_START);
    assert_eq!(r.status, 0);
    assert_eq!(r.stdout, "");
}

/// Skips with a clear message if `make test-redis` didn't set the URL.
fn test_url() -> Option<String> {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
        return None;
    };
    Some(url)
}

#[test]
fn session_start_registers_and_delivers_the_backlog() {
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:sessionstart:", std::process::id());

    // First call just registers; the inbox is empty, so nothing prints.
    let r = agent_msg(
        &["hook", "claude"],
        Some("receiver"),
        Some(&url),
        Some(&prefix),
        SESSION_START,
    );
    assert_eq!(r.status, 0);
    assert_eq!(r.stdout, "");

    let r = agent_msg(
        &["send", "receiver", "hi", "there"],
        Some("sender"),
        Some(&url),
        Some(&prefix),
        "",
    );
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);

    let r = agent_msg(
        &["hook", "claude"],
        Some("receiver"),
        Some(&url),
        Some(&prefix),
        SESSION_START,
    );
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);
    let out: serde_json::Value = serde_json::from_str(&r.stdout).expect("json stdout");
    assert_eq!(out["hookSpecificOutput"]["hookEventName"], "SessionStart");
    let ctx = out["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .expect("additionalContext string");
    assert!(ctx.contains("from sender"), "context: {ctx}");
    assert!(ctx.contains("hi there"), "context: {ctx}");
}

#[test]
fn stop_blocks_with_pending_messages_as_reason() {
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:stop:", std::process::id());
    agent_msg(
        &["hook", "claude"],
        Some("receiver"),
        Some(&url),
        Some(&prefix),
        SESSION_START,
    );
    let r = agent_msg(
        &["send", "receiver", "urgent"],
        Some("sender"),
        Some(&url),
        Some(&prefix),
        "",
    );
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);

    let r = agent_msg(
        &["hook", "claude"],
        Some("receiver"),
        Some(&url),
        Some(&prefix),
        STOP,
    );
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);
    let out: serde_json::Value = serde_json::from_str(&r.stdout).expect("json stdout");
    assert_eq!(out["decision"], "block");
    let reason = out["reason"].as_str().expect("reason string");
    assert!(reason.contains("urgent"), "reason: {reason}");
}

#[test]
fn subagent_call_leaves_the_inbox_untouched() {
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:subagent:", std::process::id());
    agent_msg(
        &["hook", "claude"],
        Some("receiver"),
        Some(&url),
        Some(&prefix),
        SESSION_START,
    );
    let r = agent_msg(
        &["send", "receiver", "keep"],
        Some("sender"),
        Some(&url),
        Some(&prefix),
        "",
    );
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);

    // A subagent call for the same lead must not pop the message.
    let r = agent_msg(
        &["hook", "claude"],
        Some("receiver"),
        Some(&url),
        Some(&prefix),
        SUBAGENT_PAYLOAD,
    );
    assert_eq!(r.status, 0);
    assert_eq!(r.stdout, "");

    // The lead still sees it on its next call.
    let r = agent_msg(
        &["hook", "claude"],
        Some("receiver"),
        Some(&url),
        Some(&prefix),
        STOP,
    );
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);
    let out: serde_json::Value = serde_json::from_str(&r.stdout).expect("json stdout");
    assert!(out["reason"].as_str().unwrap().contains("keep"));
}

#[test]
fn session_start_hook_records_the_herdr_pane_from_env() {
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:pane_hook:", std::process::id());
    let r = agent_msg_with_pane(
        &["hook", "claude"],
        Some("receiver"),
        Some(&url),
        Some(&prefix),
        Some("w1:p1"),
        SESSION_START,
    );
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);

    let r = agent_msg(&["who"], Some("receiver"), Some(&url), Some(&prefix), "");
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);
    assert!(r.stdout.contains("pane w1:p1"), "stdout: {}", r.stdout);
}

#[test]
fn register_command_records_the_herdr_pane_from_env() {
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:pane_register:", std::process::id());
    let r = agent_msg_with_pane(
        &["register"],
        Some("receiver"),
        Some(&url),
        Some(&prefix),
        Some("w2:p3"),
        "",
    );
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);

    let r = agent_msg(&["who"], Some("receiver"), Some(&url), Some(&prefix), "");
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);
    assert!(r.stdout.contains("pane w2:p3"), "stdout: {}", r.stdout);
}

#[test]
fn session_start_hook_without_herdr_pane_env_records_no_pane() {
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:no_pane:", std::process::id());
    let r = agent_msg_with_pane(
        &["hook", "claude"],
        Some("receiver"),
        Some(&url),
        Some(&prefix),
        None,
        SESSION_START,
    );
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);

    let r = agent_msg(&["who"], Some("receiver"), Some(&url), Some(&prefix), "");
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);
    assert!(r.stdout.contains("pane -"), "stdout: {}", r.stdout);
}

#[test]
fn codex_subagent_call_leaves_the_inbox_untouched() {
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:codexsubagent:", std::process::id());
    agent_msg(
        &["hook", "codex"],
        Some("receiver"),
        Some(&url),
        Some(&prefix),
        SESSION_START,
    );
    let r = agent_msg(
        &["send", "receiver", "keep"],
        Some("sender"),
        Some(&url),
        Some(&prefix),
        "",
    );
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);

    // A Codex subagent call (real captured payload, agent_id set) for the same
    // lead must not pop the message.
    let r = agent_msg(
        &["hook", "codex"],
        Some("receiver"),
        Some(&url),
        Some(&prefix),
        CODEX_SUBAGENT_PAYLOAD,
    );
    assert_eq!(r.status, 0);
    assert_eq!(r.stdout, "");

    // The lead still sees it on its next call.
    let r = agent_msg(
        &["hook", "codex"],
        Some("receiver"),
        Some(&url),
        Some(&prefix),
        STOP,
    );
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);
    let out: serde_json::Value = serde_json::from_str(&r.stdout).expect("json stdout");
    assert!(out["reason"].as_str().unwrap().contains("keep"));
}

#[test]
fn send_to_an_unknown_lead_exits_one() {
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:unknown:", std::process::id());
    let r = agent_msg(
        &["send", "nope-lead", "hi"],
        Some("sender"),
        Some(&url),
        Some(&prefix),
        "",
    );
    assert_eq!(r.status, 1);
    assert!(
        r.stderr.contains("no agent matches \"nope-lead\""),
        "stderr: {}",
        r.stderr
    );
}

/// AGE-2017: `~/.ix/relay/config.toml` (overridden here through
/// `IX_RELAY_CONFIG`, never the real path — see `relay_core::config`).
fn scratch_config_dir(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "ix-relay-agent-msg-config-{name}-{}",
        std::process::id()
    ))
}

#[test]
fn hook_stays_silent_and_exits_zero_on_a_malformed_config_file() {
    let dir = scratch_config_dir("malformed-hook");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    // Unterminated string: an unambiguous TOML syntax error (see
    // relay-core's `config::tests::malformed_file_names_path_line_and_column`).
    std::fs::write(&path, "url = \"redis://x\"\nmachine = \"unterminated\n").unwrap();

    let r = agent_msg_with_config(&["hook", "claude"], Some("qsl-lead"), &path, SESSION_START);

    assert_eq!(r.status, 0, "stderr: {}", r.stderr);
    assert_eq!(r.stdout, "");
    assert!(
        r.stderr.contains("agent-msg hook:") && r.stderr.contains(path.to_str().unwrap()),
        "stderr: {}",
        r.stderr
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_non_hook_command_exits_one_on_a_malformed_config_file() {
    let dir = scratch_config_dir("malformed-cmd");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    std::fs::write(&path, "url = \"redis://x\"\nmachine = \"unterminated\n").unwrap();

    let r = agent_msg_with_config(&["who"], Some("qsl-lead"), &path, "");

    assert_eq!(r.status, 1);
    assert!(
        r.stderr.contains(path.to_str().unwrap()),
        "stderr: {}",
        r.stderr
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_url_in_the_config_file_is_used_when_no_flag_or_env_overrides_it() {
    let dir = scratch_config_dir("url-from-file");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    // Not a valid Redis URL at all: `redis::Client::open` refuses it before
    // ever touching the network, which only happens if the CLI actually read
    // this string from the file rather than falling back to the compiled-in
    // default (a well-formed URL, which would fail differently — a refused
    // connection, not a parse error).
    std::fs::write(&path, "url = \"agent-msg-canary-not-a-redis-url\"\n").unwrap();

    let r = agent_msg_with_config(&["who"], Some("qsl-lead"), &path, "");

    assert_eq!(r.status, 1);
    assert!(
        r.stderr.contains("did not parse"),
        "expected a URL parse failure naming the config file's bogus url: {}",
        r.stderr
    );
    std::fs::remove_dir_all(&dir).ok();
}
