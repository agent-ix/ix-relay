//! Hook wall time against a Redis that accepts and never answers (NFR-001).
//!
//! The thresholds are wall-clock, so these are `#[ignore]`d into a benchmark
//! lane (`cargo test -p agent-msg --test hook_budget -- --ignored`) and out of
//! the default run, where a loaded host could fail them for reasons unrelated
//! to the hook.
#![expect(
    clippy::unwrap_used,
    reason = "test code: a failed unwrap is the failure signal"
)]

mod support;

use std::{
    io::Read,
    net::TcpListener,
    thread,
    time::{Duration, Instant},
};

use support::{base_cmd, run};

/// A Redis that accepts every connection and reads without ever replying, as a
/// dead tunnel does. Threads end with the test process.
fn blackhole() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("redis://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            thread::spawn(move || {
                let mut stream = stream;
                let mut buf = [0u8; 4096];
                while stream.read(&mut buf).is_ok_and(|n| n > 0) {}
            });
        }
    });
    url
}

/// Hook deadline against a blackhole, with or without preserved name metadata.
fn hook_wall_time(event: &str, explicit: bool) -> Duration {
    // Warm executable validation for the freshly compiled debug binary. The
    // live fresh-startup acceptance covers cold startup separately; this lane
    // measures the hook's network deadline on an otherwise idle host.
    let warm = run(base_cmd(&["--version"], None), "");
    assert_eq!(warm.status, 0);
    let mut cmd = base_cmd(&["hook", "claude"], explicit.then_some("hub:EA:code"));
    // Both variants must actually execute a lead hook. Without HERDR_ENV the
    // derived variant exits at the non-lead gate and never tests Redis at all.
    cmd.env("HERDR_ENV", "1").env("CODEX_THREAD_ID", "s1");
    cmd.env("IX_RELAY_URL", blackhole());
    let stdin = format!(r#"{{"hook_event_name":"{event}","session_id":"s1","cwd":"/work"}}"#);
    let started = Instant::now();
    let r = run(cmd, &stdin);
    let elapsed = started.elapsed();
    assert_eq!(r.status, 0, "a hook never fails the turn: {}", r.stderr);
    assert!(r.stdout.is_empty(), "nothing to print: {}", r.stdout);
    elapsed
}

/// Tracing: TC-N001-M-01
/// ACs: NFR-001-M-1
#[test]
#[ignore = "benchmark lane: wall-clock threshold"]
fn tc_n001_m_01_user_prompt_submit_against_a_blackholed_redis_is_within_400ms() {
    let elapsed = hook_wall_time("UserPromptSubmit", false);
    assert!(elapsed < Duration::from_millis(400), "took {elapsed:?}");
}

/// Tracing: TC-N001-M-02
/// ACs: NFR-001-M-2
///
/// These events exist to deliver mail, so they keep the 900 ms budget hit or
/// miss (see `hook_deadline_budget`); a dead tunnel costs a tool call that.
#[test]
#[ignore = "benchmark lane: wall-clock threshold"]
fn tc_n001_m_02_post_tool_use_and_stop_against_a_blackholed_redis_are_within_1000ms() {
    for event in ["PostToolUse", "Stop"] {
        for explicit in [true, false] {
            let elapsed = hook_wall_time(event, explicit);
            assert!(
                elapsed < Duration::from_millis(1000),
                "{event} explicit={explicit} took {elapsed:?}"
            );
        }
    }
}

/// Tracing: TC-N001-M-06
/// ACs: NFR-001-M-6
///
/// A session-cache miss must ask herdr; with herdr hanging and Redis answering
/// (a real Redis, `IX_RELAY_TEST_URL`), `PostToolUse` and `Stop` still finish
/// inside their 900 ms budget (1000 ms threshold) and exit 0.
#[test]
#[ignore = "benchmark lane: wall-clock threshold; needs IX_RELAY_TEST_URL"]
fn tc_n001_m_06_a_cache_miss_with_herdr_hanging_stays_within_the_budget() {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
        return;
    };
    let herdr = support::Scratch::new("ix-relay-hanging-herdr-");
    let script = herdr.path().join("herdr");
    std::fs::write(&script, "#!/usr/bin/env bash\nexec sleep 30\n").unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    for event in ["PostToolUse", "Stop"] {
        let mut cmd = base_cmd(&["hook", "claude"], None);
        cmd.env("IX_RELAY_URL", &url);
        cmd.env(
            "IX_RELAY_PREFIX",
            format!("test:{}:m6:{event}:", std::process::id()),
        );
        cmd.env("HERDR_PANE_ID", "w1:p1");
        cmd.env("HERDR_TAB_ID", "w1:t1");
        cmd.env("HERDR_WORKSPACE_ID", "w1");
        let path = std::env::var("PATH").unwrap_or_default();
        cmd.env("PATH", format!("{}:{path}", herdr.path().display()));
        let stdin =
            format!(r#"{{"hook_event_name":"{event}","session_id":"m6-miss","cwd":"/work"}}"#);
        let started = Instant::now();
        let r = run(cmd, &stdin);
        let elapsed = started.elapsed();
        assert_eq!(r.status, 0, "{event}: {}", r.stderr);
        assert!(
            elapsed < Duration::from_millis(1000),
            "{event} took {elapsed:?}"
        );
    }
}
