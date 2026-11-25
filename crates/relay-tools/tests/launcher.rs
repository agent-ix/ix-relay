//! The launcher binary against a stub `codex` on a private `PATH`.
#![expect(
    clippy::unwrap_used,
    reason = "test code: a failed unwrap is the failure signal"
)]

use ix_trace_rs::trace;
use std::{os::unix::fs::PermissionsExt, process::Command};

fn run(herdr: Option<&str>, args: &[&str]) -> (String, bool) {
    let dir = tempfile::tempdir().unwrap();
    let stub = dir.path().join("codex");
    std::fs::write(
        &stub,
        "#!/bin/sh\necho \"args:$*\"\necho \"name:$AGENT_MSG_NAME\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_codex-herdr-launcher"));
    cmd.args(args)
        .env("PATH", format!("/usr/bin:/bin:{}", dir.path().display()))
        .env("AGENT_MSG_NAME", "leave-this-alone")
        .env_remove("HERDR_ENV");
    if let Some(value) = herdr {
        cmd.env("HERDR_ENV", value);
    }
    let out = cmd.output().unwrap();
    (String::from_utf8(out.stdout).unwrap(), out.status.success())
}

#[test]
#[trace("FR-006-AC-3")]
fn interactive_herdr_start_runs_the_stub_with_no_daemon_and_the_same_environment() {
    let (out, ok) = run(Some("1"), &["resume", "session-id"]);
    assert!(ok);
    assert_eq!(
        out,
        "args:--no-daemon resume session-id\nname:leave-this-alone\n"
    );
}

#[test]
#[trace("FR-006-AC-3")]
fn outside_herdr_the_arguments_are_untouched() {
    let (out, ok) = run(None, &["resume", "session-id"]);
    assert!(ok);
    assert_eq!(out, "args:resume session-id\nname:leave-this-alone\n");
}

#[test]
#[trace("FR-006-AC-3")]
fn missing_codex_is_a_failure() {
    let out = Command::new(env!("CARGO_BIN_EXE_codex-herdr-launcher"))
        .env("PATH", "/nonexistent")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("could not find the underlying Codex"));
}
