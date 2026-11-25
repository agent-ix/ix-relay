//! Optional diagnostic hook: records only allowlisted Codex `SessionStart`
//! metadata, one JSON line per event, and never interrupts the real hooks.

use std::{
    fs::{self, DirBuilder, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use serde_json::{Map, Value, json};

/// Environment variables copied into the record's `context`.
pub const CONTEXT_KEYS: [&str; 6] = [
    "HERDR_ENV",
    "HERDR_SOCKET_PATH",
    "HERDR_PANE_ID",
    "HERDR_TAB_ID",
    "HERDR_WORKSPACE_ID",
    "CODEX_THREAD_ID",
];

/// Most hook payload bytes read from stdin.
const MAX_PAYLOAD: u64 = 1024 * 1024;

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(false) => false,
        Value::Bool(true) => true,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// The record for `payload`, or `None` when it is not a top-level
/// `SessionStart` (subagent events carry an `agent_id`).
pub fn record(payload: &Value, env: impl Fn(&str) -> Option<String>) -> Option<Value> {
    let fields = payload.as_object()?;
    let get = |key: &str| fields.get(key).cloned().unwrap_or(Value::Null);
    if fields.get("hook_event_name") != Some(&json!("SessionStart"))
        || fields.get("agent_id").is_some_and(truthy)
    {
        return None;
    }
    let context: Map<String, Value> = CONTEXT_KEYS
        .iter()
        .map(|key| {
            (
                (*key).to_string(),
                env(key).map_or(Value::Null, Value::String),
            )
        })
        .collect();
    Some(json!({
        "hook_event_name": get("hook_event_name"),
        "session_id": get("session_id"),
        "transcript_path": get("transcript_path"),
        "context": context,
    }))
}

/// The capture file under a Codex home.
pub fn output_path(codex_home: &Path) -> PathBuf {
    codex_home.join("ix-relay").join("session-start.jsonl")
}

/// Appends `record` to `output` (mode 0600, no symlink, owned by this user).
/// A file that is not a regular file of this user is left untouched.
pub fn append(output: &Path, record: &Value) -> std::io::Result<()> {
    if let Some(parent) = output.parent() {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
    }
    let mut file = OpenOptions::new()
        .append(true)
        .create(true)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .mode(0o600)
        .open(output)?;
    let info = file.metadata()?;
    if !info.is_file() || info.uid() != nix::unistd::getuid().as_raw() {
        return Ok(());
    }
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.write_all(format!("{record}\n").as_bytes())
}

/// Runs the hook for the payload on `input`; every failure is swallowed.
pub fn run(input: impl Read, codex_home: &Path, env: impl Fn(&str) -> Option<String>) {
    let mut bytes = Vec::new();
    if input.take(MAX_PAYLOAD).read_to_end(&mut bytes).is_err() {
        return;
    }
    let Ok(payload) = serde_json::from_slice::<Value>(&bytes) else {
        return;
    };
    if let Some(record) = record(&payload, env) {
        let _ = append(&output_path(codex_home), &record);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(key: &str) -> Option<String> {
        (key == "HERDR_PANE_ID").then(|| "p1".to_string())
    }

    #[test]
    fn session_start_is_recorded_with_allowlisted_context_only() {
        let payload = json!({"hook_event_name": "SessionStart", "session_id": "s", "transcript_path": "/t", "secret": "x"});
        let rec = record(&payload, env).unwrap();
        assert_eq!(rec["session_id"], "s");
        assert_eq!(rec["context"]["HERDR_PANE_ID"], "p1");
        assert_eq!(rec["context"]["HERDR_ENV"], Value::Null);
        assert!(rec.get("secret").is_none());
    }

    #[test]
    fn other_events_and_subagents_are_skipped() {
        assert!(record(&json!({"hook_event_name": "Stop"}), env).is_none());
        let sub = json!({"hook_event_name": "SessionStart", "agent_id": "a"});
        assert!(record(&sub, env).is_none());
        assert!(record(&json!([1]), env).is_none());
        let falsy = json!({"hook_event_name": "SessionStart", "agent_id": ""});
        assert!(record(&falsy, env).is_some());
    }

    #[test]
    fn run_appends_a_private_line_and_swallows_bad_input() {
        let home = tempfile::tempdir().unwrap();
        let line = br#"{"hook_event_name":"SessionStart","session_id":"s"}"#;
        run(&line[..], home.path(), env);
        run(&b"not json"[..], home.path(), env);
        run(&line[..], home.path(), env);
        let out = output_path(home.path());
        let text = fs::read_to_string(&out).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert_eq!(
            fs::metadata(&out).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn a_symlinked_capture_file_is_not_followed() {
        let home = tempfile::tempdir().unwrap();
        let out = output_path(home.path());
        fs::create_dir_all(out.parent().unwrap()).unwrap();
        let target = home.path().join("elsewhere");
        fs::write(&target, "").unwrap();
        std::os::unix::fs::symlink(&target, &out).unwrap();
        run(
            &br#"{"hook_event_name":"SessionStart"}"#[..],
            home.path(),
            env,
        );
        assert_eq!(fs::read_to_string(&target).unwrap(), "");
    }
}
