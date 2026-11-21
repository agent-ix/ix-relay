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

use std::{fs, os::unix::fs::PermissionsExt, process::Stdio, time::Duration};

use ix_trace_rs::trace;
use relay_core::{
    Relay,
    named_focus::{FocusContent, FocusId, LivenessEvidence, Member, MemberKind, OwnerBinding},
};
use serde_json::Value;

// The codec the command runs, so that a definition seeded here is stored exactly as `create`
// stores it.
#[path = "../src/focuses/codec.rs"]
mod codec;

fn fake_herdr(dir: &support::Scratch) {
    let herdr = dir.join("herdr");
    fs::write(
        &herdr,
        "#!/bin/sh\ncase \"$1 $2\" in\n  'api snapshot') echo '{\"result\":{\"snapshot\":{\"panes\":[{\"pane_id\":\"w1:p1\",\"workspace_id\":\"w1\",\"tab_id\":\"w1:t1\",\"agent\":\"codex\",\"agent_status\":\"idle\",\"agent_session\":{\"agent\":\"codex\",\"kind\":\"id\",\"value\":\"session-1\"}}],\"workspaces\":[{\"workspace_id\":\"w1\",\"label\":\"misc\"}],\"tabs\":[{\"tab_id\":\"w1:t1\",\"label\":\"code\"}]}}}';;\n  'pane process-info') echo '{\"result\":{\"process_info\":{\"foreground_processes\":[{\"name\":\"codex\",\"pid\":123,\"argv\":[\"codex\"]}]}}}';;\n  *) exit 1;;\nesac\n",
    )
    .unwrap();
    fs::set_permissions(&herdr, fs::Permissions::from_mode(0o755)).unwrap();
}

/// The command in the isolated context: the fake Herdr first on `PATH`, the case's own Redis
/// prefix, and the calling pane `pane`.
fn build(
    args: &[&str],
    dir: &support::Scratch,
    url: Option<&str>,
    prefix: &str,
    pane: &str,
) -> std::process::Command {
    let mut cmd = support::base_cmd(args, None);
    if let Some(url) = url {
        cmd.env("IX_RELAY_URL", url);
    }
    cmd.env("IX_RELAY_PREFIX", prefix)
        .env("IX_RELAY_MACHINE", "draco")
        .env("HERDR_PANE_ID", pane)
        .env(
            "PATH",
            format!("{}:{}", dir.display(), std::env::var("PATH").unwrap()),
        );
    cmd
}

fn command(args: &[&str], dir: &support::Scratch, url: Option<&str>, prefix: &str) -> support::Run {
    support::run(build(args, dir, url, prefix, "w1:p1"), "")
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

// --- The production caller, through the real binary and an isolated fake Herdr ------------------

/// The Redis of the case, or `None` (with a note) when no test Redis is configured.
fn test_url() -> Option<String> {
    let url = std::env::var("IX_RELAY_TEST_URL").ok();
    if url.is_none() {
        eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
    }
    url
}

/// A definition stored the way `create` stores one, owned by `<draco>/<session>`: seeded
/// through the library because `create` itself needs Linear.
fn seed(url: &str, prefix: &str, session: &str) -> FocusId {
    let mut relay = Relay::connect(url, Duration::from_secs(5))
        .unwrap()
        .with_prefix(prefix);
    let owner = OwnerBinding::new(
        &"draco".parse().unwrap(),
        &session.parse().unwrap(),
        Some(1),
        &format!("draco:misc:{session}"),
        1,
    );
    let content = FocusContent {
        team: "SWM".into(),
        name: format!("Seeded by {session}"),
        description: String::new(),
        include: vec![Member::new(MemberKind::Project, "p-1", "p-1")],
        exclude: Vec::new(),
    };
    relay
        .focus_create(
            &codec::IxBoardCodec,
            content,
            &owner,
            1,
            &LivenessEvidence::complete(Vec::new()),
        )
        .unwrap()
        .meta
        .focus_id
}

fn shown(dir: &support::Scratch, url: &str, prefix: &str, id: &FocusId) -> Value {
    let run = command(
        &["focuses", "--json", "show", id.as_str()],
        dir,
        Some(url),
        prefix,
    );
    assert_eq!(run.status, 0, "{}", run.stderr);
    serde_json::from_str(&run.stdout).unwrap()
}

#[test]
#[trace("FR-061-AC-6", "FR-061-AC-11", "FR-061-AC-21")]
fn the_calling_pane_verified_through_herdr_is_the_owner_that_archives_and_its_binding_is_refreshed()
{
    let Some(url) = test_url() else {
        return;
    };
    let dir = support::Scratch::new("focuses-cli-");
    fake_herdr(&dir);
    let prefix = format!("test:{}:focuses-cli-owner:", std::process::id());
    let id = seed(&url, &prefix, "session-1");
    assert_eq!(
        shown(&dir, &url, &prefix, &id)["owner"]["binding"]["verified_at"],
        1
    );

    let archived = command(
        &[
            "focuses",
            "--json",
            "archive",
            id.as_str(),
            "--expected-revision",
            "1",
        ],
        &dir,
        Some(&url),
        &prefix,
    );
    assert_eq!(archived.status, 0, "{}", archived.stderr);
    assert!(archived.stderr.is_empty());
    assert_eq!(archived.stdout.matches('\n').count(), 1);
    let printed: Value = serde_json::from_str(&archived.stdout).unwrap();
    assert_eq!(printed["state"], "archived");

    let after = shown(&dir, &url, &prefix, &id);
    let binding = &after["owner"]["binding"];
    assert_eq!(after["owner"]["owner_id"], "draco/session-1");
    assert_eq!(
        after["owner"]["epoch"], 1,
        "the same owner: the epoch does not move"
    );
    assert_eq!(
        binding["process_id"], 123,
        "the process measured through Herdr"
    );
    assert_eq!(
        binding["address"], "draco:misc:code",
        "the address Herdr reports for the pane, replacing the seeded one"
    );
    assert!(
        binding["verified_at"].as_i64().unwrap() > 1_700_000_000_000,
        "the binding is stamped when it was verified, in milliseconds: {binding}"
    );
}

#[test]
#[trace("FR-061-AC-6", "FR-061-AC-21")]
fn a_caller_that_cannot_be_verified_or_does_not_own_the_definition_changes_nothing() {
    let Some(url) = test_url() else {
        return;
    };
    let dir = support::Scratch::new("focuses-cli-");
    fake_herdr(&dir);
    let prefix = format!("test:{}:focuses-cli-refused:", std::process::id());
    let mine = seed(&url, &prefix, "session-1");
    let theirs = seed(&url, &prefix, "someone-else");
    let before = (
        shown(&dir, &url, &prefix, &mine),
        shown(&dir, &url, &prefix, &theirs),
    );
    let archive = |id: &FocusId| {
        [
            "focuses",
            "--json",
            "archive",
            id.as_str(),
            "--expected-revision",
            "1",
        ]
        .map(str::to_owned)
    };
    let run = |args: Vec<String>, pane: &str| {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        support::run(build(&args, &dir, Some(&url), &prefix, pane), "")
    };

    // The pane hosts session-1, not the session `--session` names.
    let mut args = archive(&mine).to_vec();
    args.splice(1..1, ["--session".to_owned(), "other-session".to_owned()]);
    let error = typed_error(&run(args, "w1:p1"));
    assert_eq!(error["code"], "invalid_argument");
    assert_eq!(error["field"], "session");

    // A pane Herdr does not list is not a verified caller.
    let error = typed_error(&run(archive(&mine).to_vec(), "w9:p9"));
    assert_eq!(error["code"], "invalid_argument");
    assert_eq!(error["field"], "lead");

    // A verified caller that is not the owner is refused, and the error names the owner.
    let error = typed_error(&run(archive(&theirs).to_vec(), "w1:p1"));
    assert_eq!(error["code"], "not_owner");
    assert_eq!(error["current_owner_id"], "draco/someone-else");
    assert_eq!(error["current_owner_epoch"], 1);

    assert_eq!(
        (
            shown(&dir, &url, &prefix, &mine),
            shown(&dir, &url, &prefix, &theirs)
        ),
        before,
        "no refusal changed a definition"
    );
}

#[test]
#[trace("FR-061-AC-7", "FR-061-AC-21")]
fn the_calling_pane_claims_a_definition_whose_owner_session_is_absent() {
    let Some(url) = test_url() else {
        return;
    };
    let dir = support::Scratch::new("focuses-cli-");
    fake_herdr(&dir);
    let prefix = format!("test:{}:focuses-cli-claim:", std::process::id());
    let id = seed(&url, &prefix, "gone-session");
    let claimed = command(
        &[
            "focuses",
            "--json",
            "claim",
            id.as_str(),
            "--expected-owner-epoch",
            "1",
        ],
        &dir,
        Some(&url),
        &prefix,
    );
    assert_eq!(claimed.status, 0, "{}", claimed.stderr);
    let printed: Value = serde_json::from_str(&claimed.stdout).unwrap();
    assert_eq!(printed["owner_id"], "draco/session-1");
    assert_eq!(printed["owner_epoch"], 2);
    assert_eq!(printed["change"]["kind"], "claim");
    assert_eq!(printed["change"]["previous_owner_id"], "draco/gone-session");
    let after = shown(&dir, &url, &prefix, &id);
    assert_eq!(after["owner"]["binding"]["process_id"], 123);
    assert!(
        after["owner"]["change"]["at"].as_i64().unwrap() > 1_700_000_000_000,
        "{after}"
    );
}

#[test]
#[trace("FR-061-AC-21")]
fn a_closed_standard_output_after_the_write_committed_is_not_reported_as_a_refusal() {
    let Some(url) = test_url() else {
        return;
    };
    let dir = support::Scratch::new("focuses-cli-");
    fake_herdr(&dir);
    let prefix = format!("test:{}:focuses-cli-closed:", std::process::id());
    let id = seed(&url, &prefix, "session-1");

    // Standard output is a pipe whose reader has already gone, as in `agent-msg focuses ... | true`.
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let mut cmd = build(
        &[
            "focuses",
            "--json",
            "archive",
            id.as_str(),
            "--expected-revision",
            "1",
        ],
        &dir,
        Some(&url),
        &prefix,
        "w1:p1",
    );
    let out = cmd
        .stdin(Stdio::null())
        .stdout(writer)
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "the reader stopped, the command did not fail: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stderr.is_empty(),
        "no `invalid_argument` and no typed error: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        shown(&dir, &url, &prefix, &id)["state"],
        "archived",
        "the write that was committed stands"
    );
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
