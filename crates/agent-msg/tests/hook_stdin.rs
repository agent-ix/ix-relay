//! The hook reads stdin inside its deadline (NFR-001-M-5): a harness that never
//! closes or stalls the pipe must not hold the hook, and the hook still exits 0
//! with exactly one spool record and touches nothing on the bus.
//!
//! Against the built binary. The default lane asserts outcomes (exit code,
//! records, no connection); the wall-clock bounds are `#[ignore]`d into the
//! benchmark lane (`cargo test -p agent-msg --test hook_stdin -- --ignored`).
#![expect(
    clippy::unwrap_used,
    reason = "test code: a failed unwrap is the failure signal"
)]

mod support;

use std::{
    io::{Read, Write},
    net::TcpListener,
    process::{Child, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use serde_json::Value;
use support::{base_cmd, nonexistent_config_path, spool_path};

/// The spool record the hook wrote, as `(stage, error_kind)`.
type Record = (String, String);

/// A watchdog well past any budget: a hook still running after this is wedged.
const WEDGED: Duration = Duration::from_secs(20);

/// A Redis that counts connections and never answers.
struct CountingRedis {
    url: String,
    connections: Arc<AtomicUsize>,
    /// The peer port of every connection accepted, in order.
    peers: Arc<std::sync::Mutex<Vec<u16>>>,
}

impl CountingRedis {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("redis://{}", listener.local_addr().unwrap());
        let connections = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&connections);
        let peers: Arc<std::sync::Mutex<Vec<u16>>> = Arc::default();
        let seen_peers = Arc::clone(&peers);
        // Ends with the test process.
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                if let Ok(peer) = stream.peer_addr() {
                    seen_peers.lock().unwrap().push(peer.port());
                }
                seen.fetch_add(1, Ordering::SeqCst);
                thread::spawn(move || {
                    let mut stream = stream;
                    let mut buf = [0u8; 4096];
                    while stream.read(&mut buf).is_ok_and(|n| n > 0) {}
                });
            }
        });
        Self {
            url,
            connections,
            peers,
        }
    }

    /// Asserts at most `most` connections were made before this call. The accept
    /// queue is FIFO and the loop records each peer port in order, so once a probe
    /// connection of our own shows up in that record, every connection made before
    /// it (the hook has exited) is in the record ahead of it: the bound is checked
    /// against a positive signal, not after a sleep.
    fn assert_at_most(&self, most: usize) {
        let probe = std::net::TcpStream::connect(self.url.trim_start_matches("redis://")).unwrap();
        let port = probe.local_addr().unwrap().port();
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            let peers = self.peers.lock().unwrap();
            if let Some(before) = peers.iter().position(|p| *p == port) {
                assert!(before <= most, "{before} connections before the probe");
                return;
            }
            drop(peers);
            assert!(
                Instant::now() < until,
                "the accept loop never saw the probe"
            );
            thread::sleep(Duration::from_millis(2));
        }
    }

    /// Asserts exactly `expected` connections were made. The count is taken
    /// after the hook exited, so a connection still in the accept queue is given
    /// a moment to be counted before an exact count is compared.
    fn assert_connections(&self, expected: usize) {
        let until = Instant::now() + Duration::from_secs(2);
        while self.connections.load(Ordering::SeqCst) < expected && Instant::now() < until {
            thread::sleep(Duration::from_millis(5));
        }
        if expected == 0 {
            // Nothing to wait for; give a stray connect the same moment to show.
            thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(self.connections.load(Ordering::SeqCst), expected);
    }
}

/// A running hook whose stdin the test still holds.
struct Hook {
    child: Child,
    stdin: Option<std::process::ChildStdin>,
    config: std::path::PathBuf,
    started: Instant,
}

impl Hook {
    fn spawn(redis: &CountingRedis) -> Self {
        let config = nonexistent_config_path();
        let mut cmd = base_cmd(&["hook", "claude"], Some("hub:EA:code"));
        cmd.env("IX_RELAY_CONFIG", &config);
        cmd.env("IX_RELAY_URL", &redis.url);
        cmd.env("IX_RELAY_PREFIX", "test:");
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        let mut child = cmd.spawn().unwrap();
        let stdin = child.stdin.take();
        Self {
            child,
            stdin,
            config,
            started: Instant::now(),
        }
    }

    /// Writes `bytes`, ignoring a hook that has already stopped reading.
    fn write(&mut self, bytes: &[u8]) {
        if let Some(stdin) = self.stdin.as_mut() {
            let _ = stdin.write_all(bytes);
            let _ = stdin.flush();
        }
    }

    fn close_stdin(&mut self) {
        self.stdin = None;
    }

    /// Waits for exit (killing a wedged hook), asserts exit 0 and empty stdout,
    /// and returns the wall time and the spool records. Stdin stays open until
    /// here unless the test closed it.
    fn finish(mut self) -> (Duration, Vec<Record>) {
        let deadline = self.started + WEDGED;
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() > deadline {
                self.child.kill().unwrap();
                self.child.wait().unwrap();
                panic!("the hook was still running after {WEDGED:?}: stdin wedged it");
            }
            thread::sleep(Duration::from_millis(10));
        };
        let elapsed = self.started.elapsed();
        assert_eq!(status.code(), Some(0), "a hook never fails the turn");
        let mut stdout = String::new();
        self.child
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut stdout)
            .unwrap();
        assert_eq!(stdout, "", "nothing to print");
        let records = std::fs::read_to_string(spool_path(&self.config))
            .unwrap_or_default()
            .lines()
            .map(|line| {
                let v: Value = serde_json::from_str(line).unwrap();
                let field = |name: &str| v[name].as_str().unwrap().to_owned();
                (field("stage"), field("error_kind"))
            })
            .collect();
        std::fs::remove_dir_all(self.config.parent().unwrap()).ok();
        (elapsed, records)
    }
}

fn payload(event: &str) -> String {
    format!(r#"{{"hook_event_name":"{event}","session_id":"s1","cwd":"/work"}}"#)
}

fn rec(stage: &str, kind: &str) -> Vec<Record> {
    vec![(stage.to_owned(), kind.to_owned())]
}

/// Tracing: TC-N001-M-05
/// ACs: NFR-001-M-5
#[test]
fn tc_n001_m_05_a_stdin_that_is_never_closed_or_written_ends_in_one_timeout_record() {
    let redis = CountingRedis::start();
    let hook = Hook::spawn(&redis);
    // Stdin is held open by `hook` and never written to.
    let (_, records) = hook.finish();
    assert_eq!(records, rec("decode", "stdin_timeout"));
    // Nothing was registered or popped: the hook never reached the bus.
    redis.assert_connections(0);
}

#[test]
fn a_stdin_closed_empty_leaves_one_malformed_record() {
    let redis = CountingRedis::start();
    let mut hook = Hook::spawn(&redis);
    hook.close_stdin();
    let (_, records) = hook.finish();
    assert_eq!(records, rec("decode", "malformed"));
    // A persistent input fault makes exactly one attempt to ship the spool (see
    // `flush_after_failure`).
    redis.assert_connections(1);
}

#[test]
fn a_whole_payload_is_served_without_the_pipe_being_closed() {
    let redis = CountingRedis::start();
    let mut hook = Hook::spawn(&redis);
    hook.write(payload("Stop").as_bytes());
    // Not closed: the hook must go on to the bus, and fail there (the blackhole
    // never answers) rather than wait for end of file.
    let (_, records) = hook.finish();
    redis.assert_connections(1);
    assert_eq!(records.len(), 1, "{records:?}");
    assert_ne!(records[0].0, "decode", "{records:?}");
}

/// A payload that arrives in two writes, the pipe then closed, is joined. There
/// is no sleep between the writes, so no wall-clock ordering is involved; the
/// arrival-over-time cases run on the injected clock in the unit tests
/// (`stdin::tests`, `hook_run::tests::tc_n001_m_05_the_event_budget_*`).
#[test]
fn a_payload_written_in_two_pieces_is_served() {
    let redis = CountingRedis::start();
    let mut hook = Hook::spawn(&redis);
    let whole = payload("SessionStart");
    let (head, tail) = whole.split_at(20);
    hook.write(head.as_bytes());
    hook.write(tail.as_bytes());
    hook.close_stdin();
    let (_, records) = hook.finish();
    redis.assert_connections(1);
    assert_eq!(records.len(), 1, "{records:?}");
    assert_ne!(records[0].0, "decode", "{records:?}");
}

/// The same with a real gap between the pieces (wall-clock, so the benchmark lane).
#[test]
#[ignore = "benchmark lane: depends on wall-clock arrival"]
fn a_payload_that_arrives_with_a_gap_within_the_budget_is_served() {
    let redis = CountingRedis::start();
    let mut hook = Hook::spawn(&redis);
    let whole = payload("SessionStart");
    let (head, tail) = whole.split_at(20);
    hook.write(head.as_bytes());
    thread::sleep(Duration::from_millis(50));
    hook.write(tail.as_bytes());
    hook.close_stdin();
    let (_, records) = hook.finish();
    redis.assert_connections(1);
    assert_ne!(records[0].0, "decode", "{records:?}");
}

/// Tracing: TC-030-01
/// ACs: FR-030-AC-1
#[test]
fn an_oversized_payload_is_a_recorded_too_large_not_an_allocation() {
    let redis = CountingRedis::start();
    let mut hook = Hook::spawn(&redis);
    // Past the cap; the hook stops reading, so the write may end early.
    hook.write(&vec![b' '; (8 << 20) + (64 << 10)]);
    let (_, records) = hook.finish();
    // Reading 8 MiB inside the 300 ms the hook allows itself for stdin depends on
    // the machine (the byte cap itself is pinned by `stdin::tests`): a loaded one
    // gets the timeout first. `too_large` is a persistent input fault, so the hook
    // makes one attempt to ship the spool if it has time left; a timeout never
    // reaches the bus.
    match records.as_slice() {
        [(stage, kind)] if (stage.as_str(), kind.as_str()) == ("decode", "too_large") => {
            // The hook made at most the one attempt to ship the spool (none when the
            // read took all the time). Its child has exited, so the connections it
            // made are already accepted or refused: the count is final once the
            // accept loop has drained, which `assert_settled` waits for.
            redis.assert_at_most(1);
        }
        [(stage, kind)] if (stage.as_str(), kind.as_str()) == ("decode", "stdin_timeout") => {
            redis.assert_connections(0);
        }
        other => panic!("expected one decode record, got {other:?}"),
    }
}

/// Complete JSON that is not a hook payload, on a pipe that stays open, is
/// `malformed` at once: the read ends at the object's closing brace and the
/// decode refuses it, rather than waiting out the deadline for a `stdin_timeout`.
#[test]
fn a_complete_non_hook_object_on_an_open_pipe_is_malformed_not_a_timeout() {
    let redis = CountingRedis::start();
    let mut hook = Hook::spawn(&redis);
    hook.write(b"{}");
    let (_, records) = hook.finish();
    assert_eq!(records, rec("decode", "malformed"));
    redis.assert_connections(1);
}

/// Bytes after the first object (a second value, garbage) do not matter.
#[test]
fn bytes_after_the_first_object_are_ignored() {
    let redis = CountingRedis::start();
    let mut hook = Hook::spawn(&redis);
    hook.write(format!("{}\n{}\0junk", payload("Stop"), payload("Stop")).as_bytes());
    let (_, records) = hook.finish();
    redis.assert_connections(1);
    assert_ne!(records[0].0, "decode", "{records:?}");
}

/// Tracing: TC-N001-M-05
/// ACs: NFR-001-M-5
///
/// With no payload the event is unknown, so a silent pipe is bounded by the
/// smallest budget (300 ms): the record is written well before the harness's
/// 2 s kill. Threshold 400 ms (NFR-001-M-5).
#[test]
#[ignore = "benchmark lane: wall-clock threshold"]
fn tc_n001_m_05_a_silent_stdin_records_within_400ms() {
    let redis = CountingRedis::start();
    let (elapsed, records) = Hook::spawn(&redis).finish();
    assert_eq!(records, rec("decode", "stdin_timeout"));
    assert!(elapsed < Duration::from_millis(400), "took {elapsed:?}");
}

/// Tracing: TC-N001-M-07
/// ACs: NFR-001-M-5
///
/// A byte every 200 ms never leaves the 300 ms idle window, so only the overall
/// stdin cap ends the read: one `stdin_timeout` record at about 900 ms, before
/// the harness's 2 s kill. Threshold 1000 ms.
#[test]
#[ignore = "benchmark lane: wall-clock threshold"]
fn tc_n001_m_07_a_drip_of_bytes_records_at_the_900ms_cap() {
    let redis = CountingRedis::start();
    let mut hook = Hook::spawn(&redis);
    let mut next_byte = Instant::now();
    while hook.child.try_wait().unwrap().is_none() {
        assert!(hook.started.elapsed() < WEDGED, "stdin wedged the hook");
        if Instant::now() >= next_byte {
            hook.write(b" ");
            next_byte += Duration::from_millis(200);
        }
        thread::sleep(Duration::from_millis(5));
    }
    let (elapsed, records) = hook.finish();
    assert_eq!(records, rec("decode", "stdin_timeout"));
    assert!(
        elapsed >= Duration::from_millis(850),
        "cut early: {elapsed:?}"
    );
    assert!(elapsed < Duration::from_millis(1000), "took {elapsed:?}");
}

/// Reading and decoding the largest accepted payload must fit the smallest
/// budget. `agent_id` makes the hook return after the decode (a subagent), so
/// the time is the read and decode alone.
#[test]
#[ignore = "benchmark lane: wall-clock threshold"]
fn tc_n001_m_05_a_payload_at_the_cap_is_read_within_the_smallest_budget() {
    let redis = CountingRedis::start();
    let mut hook = Hook::spawn(&redis);
    let head = r#"{"hook_event_name":"PostToolUse","agent_id":"a","tool_response":""#;
    let tail = r#""}"#;
    let fill = (8 << 20) - head.len() - tail.len();
    hook.write(head.as_bytes());
    hook.write(&vec![b'x'; fill]);
    hook.write(tail.as_bytes());
    let (elapsed, records) = hook.finish();
    assert!(records.is_empty(), "read and decoded in time: {records:?}");
    assert!(elapsed < Duration::from_millis(300), "took {elapsed:?}");
}

/// Tracing: TC-N001-M-05
/// ACs: NFR-001-M-5
///
/// A whole payload on a pipe that stays open: each event's own budget applies,
/// counted from the start of the call.
#[test]
#[ignore = "benchmark lane: wall-clock threshold"]
fn tc_n001_m_05_each_events_budget_applies_with_stdin_held_open() {
    for (event, budget_ms) in [
        ("UserPromptSubmit", 300),
        ("PostToolUse", 900),
        ("Stop", 900),
        ("SessionStart", 3300),
    ] {
        let redis = CountingRedis::start();
        let mut hook = Hook::spawn(&redis);
        hook.write(payload(event).as_bytes());
        let (elapsed, _) = hook.finish();
        assert!(
            elapsed < Duration::from_millis(budget_ms + 400),
            "{event} took {elapsed:?}"
        );
    }
}
