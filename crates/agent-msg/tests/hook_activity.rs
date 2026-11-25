//! The activity backoff against a real refusing Redis and a real hung one
//! (FR-068, NFR-001): the first call fails on the network and starts the
//! backoff; the next call of an activity-only event never touches the network.
//! The wall-clock thresholds live in `hook_budget.rs`.
#![expect(
    clippy::unwrap_used,
    reason = "test code: a failed unwrap is the failure signal"
)]

mod support;

use std::{
    io::Read,
    net::TcpListener,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use ix_trace_rs::trace;
use support::{Scratch, base_cmd, run};

/// A Redis that accepts every connection and never answers, and how many
/// connections it has accepted. Threads end with the test process.
struct Blackhole {
    url: String,
    accepted: Arc<AtomicUsize>,
}

fn blackhole() -> Blackhole {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("redis://{}", listener.local_addr().unwrap());
    let accepted = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&accepted);
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            seen.fetch_add(1, Ordering::SeqCst);
            thread::spawn(move || {
                let mut stream = stream;
                let mut buf = [0u8; 4096];
                while stream.read(&mut buf).is_ok_and(|n| n > 0) {}
            });
        }
    });
    Blackhole { url, accepted }
}

/// Nothing listens on this port: a refused connection.
const REFUSED: &str = "redis://127.0.0.1:1";

const EVENTS: [(&str, &str); 3] = [
    ("PreToolUse", r#","tool_name":"Bash""#),
    (
        "Notification",
        r#","notification_type":"permission_prompt""#,
    ),
    ("SessionEnd", ""),
];

/// One hook call of `event` as a lead in a herdr pane, against `url`, with its
/// state in `config_dir`. Returns the wall time.
fn call(url: &str, config_dir: &Path, event: &str, extra: &str) -> Duration {
    let mut cmd = base_cmd(&["hook", "claude"], Some("hub:EA:code"));
    cmd.env("IX_RELAY_URL", url);
    cmd.env("IX_RELAY_CONFIG", config_dir.join("config.toml"));
    let stdin =
        format!(r#"{{"hook_event_name":"{event}","session_id":"s1","cwd":"/work"{extra}}}"#);
    let started = Instant::now();
    let r = run(cmd, &stdin);
    let elapsed = started.elapsed();
    assert_eq!(r.status, 0, "{event}: a hook never fails the turn");
    assert!(
        r.stdout.is_empty(),
        "{event}: nothing on stdout: {}",
        r.stdout
    );
    elapsed
}

fn counts(config_dir: &Path) -> serde_json::Value {
    let text = std::fs::read_to_string(config_dir.join("spool/activity.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn spool_records(config_dir: &Path) -> usize {
    std::fs::read_to_string(config_dir.join("spool/hook.jsonl"))
        .map_or(0, |text| text.lines().count())
}

#[trace("FR-068-AC-11")]
#[test]
fn a_refusing_redis_starts_the_backoff_and_the_next_call_skips_the_network() {
    for (event, extra) in EVENTS {
        let dir = Scratch::new("ix-relay-activity-refused-");
        let first = call(REFUSED, dir.path(), event, extra);
        let c = counts(dir.path());
        assert_eq!(
            (c["dropped"].as_u64(), c["backoff_ms"].as_u64()),
            (Some(1), Some(5000))
        );
        assert_eq!(spool_records(dir.path()), 1, "one connect record");
        let second = call(REFUSED, dir.path(), event, extra);
        let c = counts(dir.path());
        assert_eq!(
            (c["skipped"].as_u64(), c["dropped"].as_u64()),
            (Some(1), Some(1))
        );
        assert_eq!(spool_records(dir.path()), 1, "{event}: no second record");
        eprintln!("refused {event}: first {first:?}, second {second:?}");
    }
}

#[trace("FR-068-AC-8", "FR-068-AC-11")]
#[test]
fn a_hung_redis_starts_the_backoff_and_the_next_call_opens_no_connection() {
    for (event, extra) in EVENTS {
        let hole = blackhole();
        let dir = Scratch::new("ix-relay-activity-hung-");
        let first = call(&hole.url, dir.path(), event, extra);
        let c = counts(dir.path());
        assert_eq!(
            (c["dropped"].as_u64(), c["backoff_ms"].as_u64()),
            (Some(1), Some(5000))
        );
        let records = spool_records(dir.path());
        assert_eq!(records, 1, "one record for the failed call");
        let connections = hole.accepted.load(Ordering::SeqCst);
        assert!(connections >= 1);
        let second = call(&hole.url, dir.path(), event, extra);
        assert_eq!(
            hole.accepted.load(Ordering::SeqCst),
            connections,
            "{event}: no network"
        );
        let c = counts(dir.path());
        assert_eq!(
            (c["skipped"].as_u64(), c["dropped"].as_u64()),
            (Some(1), Some(1))
        );
        assert_eq!(spool_records(dir.path()), records);
        eprintln!("hung {event}: first {first:?}, second {second:?}");
    }
}

/// Real hook processes that find the state lock held drop their update and each counts it in
/// `activity.lockskips`; a process that cannot write the state files counts under
/// `activity.writefails`. The two reasons never share a file.
#[trace("FR-068-AC-14", "FR-068-AC-15")]
#[test]
fn separate_hook_processes_count_lock_skips_and_write_failures_apart() {
    let skips = |dir: &Path, name: &str| {
        std::fs::metadata(dir.join("spool").join(name)).map_or(0, |m| m.len())
    };

    let dir = Scratch::new("ix-relay-activity-locked-");
    std::fs::create_dir_all(dir.path().join("spool")).unwrap();
    let holder = std::fs::File::create(dir.path().join("spool/activity.lock")).unwrap();
    holder.lock().unwrap();
    for _ in 0..3 {
        let waited = call(REFUSED, dir.path(), "PreToolUse", r#","tool_name":"Bash""#);
        eprintln!("held lock, one hook: {waited:?}");
    }
    assert!(
        skips(dir.path(), "activity.lockskips") >= 3,
        "each of three processes counted its dropped update"
    );
    assert_eq!(skips(dir.path(), "activity.writefails"), 0);
    assert!(
        !dir.path().join("spool/activity.json").exists(),
        "no update was applied while the lock was held"
    );
    drop(holder);

    let dir = Scratch::new("ix-relay-activity-unwritable-");
    std::fs::create_dir_all(dir.path().join("spool")).unwrap();
    let elsewhere = dir.path().join("elsewhere.json");
    std::fs::write(&elsewhere, "{}").unwrap();
    std::os::unix::fs::symlink(&elsewhere, dir.path().join("spool/activity.json")).unwrap();
    call(REFUSED, dir.path(), "PreToolUse", r#","tool_name":"Bash""#);
    assert!(skips(dir.path(), "activity.writefails") >= 1);
    assert_eq!(skips(dir.path(), "activity.lockskips"), 0);
    assert_eq!(std::fs::read_to_string(&elsewhere).unwrap(), "{}");
}
