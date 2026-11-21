// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! The real `agent-msg focuses` binary: standard output, standard error and exit status
//! (FR-061-AC-21), on the paths that need neither a credential nor Linear.
//!
//! The in-process cases in `src/focuses/tests.rs` cover resolution, the bounded read and the
//! writes with a fake source. Nothing here reads the process environment for a token, starts a
//! credential flow or reaches a network other than an isolated test Redis; a case that needs
//! Redis is skipped when `IX_RELAY_TEST_URL` is unset.
#![allow(clippy::unwrap_used, reason = "test fixtures")]

mod support;

use std::{fs, os::unix::fs::PermissionsExt};

use ix_trace_rs::trace;
use serde_json::Value;

fn fake_herdr(dir: &support::Scratch) {
    let herdr = dir.join("herdr");
    fs::write(
        &herdr,
        "#!/bin/sh\ncase \"$1 $2\" in\n  'api snapshot') echo '{\"result\":{\"snapshot\":{\"panes\":[{\"pane_id\":\"w1:p1\",\"workspace_id\":\"w1\",\"tab_id\":\"w1:t1\",\"agent\":\"codex\",\"agent_status\":\"idle\",\"agent_session\":{\"agent\":\"codex\",\"kind\":\"id\",\"value\":\"session-1\"}}],\"workspaces\":[{\"workspace_id\":\"w1\",\"label\":\"misc\"}],\"tabs\":[{\"tab_id\":\"w1:t1\",\"label\":\"code\"}]}}}';;\n  'pane process-info') echo '{\"result\":{\"process_info\":{\"foreground_processes\":[{\"name\":\"codex\",\"pid\":123,\"argv\":[\"codex\"]}]}}}';;\n  *) exit 1;;\nesac\n",
    )
    .unwrap();
    fs::set_permissions(&herdr, fs::Permissions::from_mode(0o755)).unwrap();
}

fn command(args: &[&str], dir: &support::Scratch, url: Option<&str>, prefix: &str) -> support::Run {
    let mut cmd = support::base_cmd(args, None);
    if let Some(url) = url {
        cmd.env("IX_RELAY_URL", url);
    }
    cmd.env("IX_RELAY_PREFIX", prefix)
        .env("IX_RELAY_MACHINE", "draco")
        .env("HERDR_PANE_ID", "w1:p1")
        .env(
            "PATH",
            format!("{}:{}", dir.display(), std::env::var("PATH").unwrap()),
        );
    support::run(cmd, "")
}

fn typed_error(run: &support::Run) -> Value {
    assert_eq!(
        run.status, 1,
        "stdout: {} stderr: {}",
        run.stdout, run.stderr
    );
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    assert!(
        run.stderr.ends_with('\n') && run.stderr.matches('\n').count() == 1,
        "{}",
        run.stderr
    );
    serde_json::from_str(&run.stderr).unwrap()
}

#[test]
#[trace("FR-061-AC-21")]
fn an_unreachable_relay_is_a_typed_error_on_standard_error_with_exit_status_1() {
    let dir = support::Scratch::new("focuses-cli-");
    fake_herdr(&dir);
    for args in [
        &["focuses", "--json", "list"][..],
        &["focuses", "--json", "show", "Anything"],
    ] {
        let run = command(args, &dir, None, "test:focuses-unreachable:");
        let error = typed_error(&run);
        assert_eq!(error["code"], "source_unavailable");
        assert_eq!(error["reason"], "failed");
    }
    let text = command(
        &["focuses", "list"],
        &dir,
        None,
        "test:focuses-unreachable:",
    );
    assert_eq!(text.status, 1);
    assert!(text.stdout.is_empty());
    assert!(
        text.stderr.starts_with("agent-msg: source_unavailable: "),
        "{}",
        text.stderr
    );
}

#[test]
#[trace("FR-061-AC-21")]
fn reads_print_one_compact_json_value_and_a_missing_definition_is_not_found() {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
        return;
    };
    let dir = support::Scratch::new("focuses-cli-");
    fake_herdr(&dir);
    let prefix = format!("test:{}:focuses-cli-read:", std::process::id());

    let listed = command(&["focuses", "--json", "list"], &dir, Some(&url), &prefix);
    assert_eq!(listed.status, 0, "{}", listed.stderr);
    assert!(listed.stderr.is_empty());
    assert_eq!(listed.stdout, "{\"definitions\":[]}\n");

    let text = command(&["focuses", "list"], &dir, Some(&url), &prefix);
    assert_eq!(text.status, 0, "{}", text.stderr);
    assert_eq!(text.stdout, "no named focus definitions\n");

    let missing = command(
        &["focuses", "--json", "show", "No such name"],
        &dir,
        Some(&url),
        &prefix,
    );
    let error = typed_error(&missing);
    assert_eq!(error["code"], "not_found");
    assert_eq!(error["focus_id"], "No such name");

    let by_id = command(
        &[
            "focuses",
            "--json",
            "show",
            "foc-00000000000000000000000000000001",
        ],
        &dir,
        Some(&url),
        &prefix,
    );
    assert_eq!(typed_error(&by_id)["code"], "not_found");
}

#[test]
#[trace("FR-061-AC-5", "FR-061-AC-21")]
fn an_invalid_definition_file_is_refused_before_any_credential_or_snapshot_is_used() {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
        return;
    };
    let dir = support::Scratch::new("focuses-cli-");
    fake_herdr(&dir);
    let prefix = format!("test:{}:focuses-cli-file:", std::process::id());
    let cases = [
        (
            "a focus reference",
            r#"{"team":"SWM","name":"A","include":{},"focus":"foc-1"}"#,
        ),
        (
            "two teams",
            r#"{"team":["SWM","OTH"],"name":"A","include":{}}"#,
        ),
        (
            "an unknown member",
            r#"{"team":"SWM","name":"A","include":{"everything":[]}}"#,
        ),
        ("not JSON", "team: SWM"),
    ];
    for (what, body) in cases {
        let file = dir.join("definition.json");
        fs::write(&file, body).unwrap();
        let run = command(
            &[
                "focuses",
                "--json",
                "create",
                "--file",
                file.to_str().unwrap(),
            ],
            &dir,
            Some(&url),
            &prefix,
        );
        let error = typed_error(&run);
        assert_eq!(error["code"], "invalid_argument", "{what}");
        let update = command(
            &[
                "focuses",
                "--json",
                "update",
                "foc-00000000000000000000000000000001",
                "--file",
                file.to_str().unwrap(),
                "--expected-revision",
                "1",
            ],
            &dir,
            Some(&url),
            &prefix,
        );
        assert_eq!(typed_error(&update)["code"], "invalid_argument", "{what}");
    }
}

#[test]
#[trace("FR-061-AC-20")]
fn the_read_timeout_outside_one_to_six_hundred_is_a_usage_error() {
    let dir = support::Scratch::new("focuses-cli-");
    for bad in ["0", "601"] {
        let run = command(
            &[
                "focuses",
                "create",
                "--file",
                "/nonexistent.json",
                "--read-timeout",
                bad,
            ],
            &dir,
            None,
            "test:focuses-usage:",
        );
        assert_eq!(run.status, 2, "{bad}: {}", run.stderr);
        assert!(run.stdout.is_empty());
        assert!(run.stderr.contains("--read-timeout"), "{}", run.stderr);
    }
}

#[test]
#[trace("FR-061-AC-22")]
fn focuses_is_a_separate_family_and_the_singular_focus_command_is_untouched() {
    let dir = support::Scratch::new("focuses-cli-");
    // The singular command takes free text; the plural family has subcommands only.
    let help = command(&["--help"], &dir, None, "test:focuses-help:");
    assert_eq!(help.status, 0);
    assert!(help.stdout.contains("focuses"), "{}", help.stdout);
    assert!(help.stdout.contains("\n  focus "), "{}", help.stdout);
    let plural = command(&["focuses", "phase 2"], &dir, None, "test:focuses-help:");
    assert_eq!(plural.status, 2, "{}", plural.stderr);
}
