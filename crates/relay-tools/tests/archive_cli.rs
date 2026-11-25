//! The `archive-relay-namespace` binary resolves its Redis URL like
//! `agent-msg`: `IX_RELAY_URL`, then `url` in `IX_RELAY_CONFIG`'s file.
#![expect(
    clippy::unwrap_used,
    reason = "test code: a failed unwrap is the failure signal"
)]

use std::process::{Command, Output};

use ix_trace_rs::trace;

fn run(env: &[(&str, &str)], output: &std::path::Path) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_archive-relay-namespace"));
    cmd.args(["--prefix", "test:cli:", "--output"])
        .arg(output)
        .env_remove("HOME")
        .env_remove("IX_RELAY_URL")
        .env_remove("IX_RELAY_CONFIG");
    for (key, value) in env {
        cmd.env(key, value);
    }
    cmd.output().unwrap()
}

fn config(dir: &tempfile::TempDir, url: &str) -> String {
    let path = dir.path().join("config.toml");
    std::fs::write(&path, format!("url = \"{url}\"\n")).unwrap();
    path.to_str().unwrap().to_string()
}

#[test]
#[trace("FR-073-AC-3")]
fn the_url_comes_from_the_file_named_by_ix_relay_config_without_home() {
    let dir = tempfile::tempdir().unwrap();
    // A rediss:// URL is refused by name, so the message shows which tier won.
    let file = config(&dir, "rediss://from-the-file");
    let out = run(&[("IX_RELAY_CONFIG", &file)], &dir.path().join("a.jsonl"));
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("rediss:// is not supported"));
}

#[test]
#[trace("FR-073-AC-3")]
fn ix_relay_url_wins_over_the_file_and_needs_no_home() {
    let dir = tempfile::tempdir().unwrap();
    let file = config(&dir, "redis://from-the-file");
    let out = run(
        &[
            ("IX_RELAY_CONFIG", &file),
            ("IX_RELAY_URL", "rediss://from-the-environment"),
        ],
        &dir.path().join("a.jsonl"),
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("rediss:// is not supported"));
}

#[test]
#[trace("FR-073-AC-3")]
fn a_malformed_config_file_fails_before_any_connection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "url = \"redis://x\"\nunknown_key = 1\n").unwrap();
    let out = run(
        &[("IX_RELAY_CONFIG", path.to_str().unwrap())],
        &dir.path().join("a.jsonl"),
    );
    assert!(!out.status.success());
    assert!(!dir.path().join("a.jsonl").exists());
}

#[test]
#[trace("FR-073-AC-3")]
fn the_config_file_url_reaches_the_redis_it_names() {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!("skipping: IX_RELAY_TEST_URL unset");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let file = config(&dir, &url);
    let output = dir.path().join("a.jsonl");
    let out = run(&[("IX_RELAY_CONFIG", &file)], &output);
    assert!(out.status.success(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("archived "));
}
