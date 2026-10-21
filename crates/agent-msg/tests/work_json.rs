//! Process-level JSON bytes for an explicit work report.
#![allow(clippy::unwrap_used, reason = "test fixtures")]

mod support;

use std::{fs, os::unix::fs::PermissionsExt};

/// Trace: FR-049-AC-1
#[test]
fn work_show_prints_the_existing_compact_report_and_one_newline() {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
        return;
    };
    let dir = support::Scratch::new("work-json-");
    let herdr = dir.join("herdr");
    fs::write(
        &herdr,
        "#!/bin/sh\ncase \"$1 $2\" in\n  'api snapshot') echo '{\"result\":{\"snapshot\":{\"panes\":[{\"pane_id\":\"w1:p1\",\"workspace_id\":\"w1\",\"tab_id\":\"w1:t1\",\"agent\":\"codex\",\"agent_status\":\"idle\",\"agent_session\":{\"agent\":\"codex\",\"kind\":\"id\",\"value\":\"session-1\"}}],\"workspaces\":[{\"workspace_id\":\"w1\",\"label\":\"misc\"}],\"tabs\":[{\"tab_id\":\"w1:t1\",\"label\":\"code\"}]}}}';;\n  'pane process-info') echo '{\"result\":{\"process_info\":{\"foreground_processes\":[{\"name\":\"codex\",\"pid\":123,\"argv\":[\"codex\"]}]}}}';;\n  *) exit 1;;\nesac\n",
    )
    .unwrap();
    fs::set_permissions(&herdr, fs::Permissions::from_mode(0o755)).unwrap();
    let prefix = format!("test:{}:work-json:", std::process::id());
    let run = |args: &[&str]| {
        let mut cmd = support::base_cmd(args, None);
        cmd.env("IX_RELAY_URL", &url)
            .env("IX_RELAY_PREFIX", &prefix)
            .env("IX_RELAY_MACHINE", "draco")
            .env("HERDR_PANE_ID", "w1:p1")
            .env(
                "PATH",
                format!("{}:{}", dir.display(), std::env::var("PATH").unwrap()),
            );
        support::run(cmd, "")
    };

    let empty = run(&["work", "show"]);
    assert_eq!(empty.status, 0, "{}", empty.stderr);
    assert_eq!(empty.stdout, "null\n");

    let set = run(&["work", "scope", "--all-team"]);
    assert_eq!(set.status, 0, "{}", set.stderr);
    let filled = run(&["work", "show"]);
    assert_eq!(filled.status, 0, "{}", filled.stderr);
    let report: serde_json::Value = serde_json::from_str(&filled.stdout).unwrap();
    let reported_at_ms = report["scope"]["reported_at_ms"].as_i64().unwrap();
    assert_eq!(
        filled.stdout,
        format!(
            "{{\"binding\":{{\"machine\":\"draco\",\"pane\":\"w1:p1\",\"session\":\"session-1\",\"process_id\":123,\"kind\":\"codex\",\"address\":\"draco:misc:code\"}},\"scope\":{{\"initiatives\":[],\"projects\":[],\"tickets\":[],\"reported_at_ms\":{reported_at_ms}}},\"focus\":null,\"agents\":[]}}\n"
        )
    );
}
