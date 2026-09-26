//! Evidence for FR-017-AC-1: every run of `agent-msg hook claude`, for any
//! bytes on stdin and any hook event name, exits 0 with a stdout that is empty
//! or one JSON object.
//!
//! The binary is run as a child process (the crate has no library target), so
//! each case costs a process spawn. Tiers, cheapest first:
//!
//! - a seed corpus of known-nasty stdin, always run, no Redis;
//! - generated stdin (arbitrary bytes and structure-aware hook JSON) against a
//!   closed port, always run at a modest case count;
//! - the same generated stdin against a real Redis holding a queued message,
//!   which is the only way stdout can be non-empty, skipped without
//!   `IX_RELAY_TEST_URL`;
//! - `fuzz_*` ignored tests at a raised case count that also bound the time each
//!   run takes (`cargo test -p agent-msg --test hook_fuzz -- --ignored fuzz_`).
//!
//! These are `proptest`-driven fuzz lanes, not coverage-guided; see
//! `fuzz/README.md`.
#![expect(
    clippy::expect_used,
    reason = "test code: a failed expect is the failure signal"
)]

mod support;

use std::{
    io::Write,
    process::Stdio,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use proptest::prelude::*;
use relay_core::{Message, Registration, Relay, Target};
use serde_json::Value;
use support::base_cmd;

const DEAD_REDIS: &str = "redis://127.0.0.1:1";

struct Ran {
    status: i32,
    stdout: Vec<u8>,
    elapsed: Duration,
}

/// Runs the hook with `stdin` (any bytes) as lead `qsl-lead` against `url`.
fn hook(stdin: &[u8], url: &str, prefix: &str) -> Ran {
    let mut cmd = base_cmd(&["hook", "claude"], Some("qsl-lead"));
    cmd.env("IX_RELAY_URL", url);
    cmd.env("IX_RELAY_PREFIX", prefix);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let started = Instant::now();
    let mut child = cmd.spawn().expect("spawn agent-msg");
    let mut pipe = child.stdin.take().expect("piped stdin");
    // The hook may exit without reading everything (a closed pipe is fine).
    let _ = pipe.write_all(stdin);
    drop(pipe);
    let out = child.wait_with_output().expect("wait for agent-msg");
    // The per-run config directory `base_cmd` made holds a spool; remove it.
    if let Some(dir) = cmd
        .get_envs()
        .find(|(k, _)| *k == "IX_RELAY_CONFIG")
        .and_then(|(_, v)| v)
        .and_then(|v| std::path::Path::new(v).parent())
    {
        std::fs::remove_dir_all(dir).ok();
    }
    Ran {
        status: out.status.code().unwrap_or(-1),
        stdout: out.stdout,
        elapsed: started.elapsed(),
    }
}

/// FR-017-AC-1's oracle: exit 0, stdout empty or exactly one JSON object.
fn check_contract(stdin: &[u8], ran: &Ran) -> Result<(), TestCaseError> {
    prop_assert_eq!(
        ran.status,
        0,
        "stdin {:?}",
        String::from_utf8_lossy(&stdin[..stdin.len().min(80)])
    );
    if !ran.stdout.is_empty() {
        let text = std::str::from_utf8(&ran.stdout)
            .map_err(|e| TestCaseError::fail(format!("stdout is not UTF-8: {e}")))?;
        let value: Value = serde_json::from_str(text).map_err(|e| {
            TestCaseError::fail(format!("stdout is not one JSON value: {e}: {text:?}"))
        })?;
        prop_assert!(
            value.is_object(),
            "stdout is JSON but not an object: {text:?}"
        );
    }
    Ok(())
}

/// Tracing: TC-017-01
/// ACs: FR-017-AC-1
///
/// Seed corpus, always run: empty input, whitespace, truncated JSON, wrong
/// shapes and types, unknown and near-miss event names, oversized fields,
/// invalid UTF-8, NULs, and deep nesting.
#[test]
fn tc_017_01_the_seed_corpus_of_nasty_stdin_exits_zero_with_empty_or_one_json_object() {
    let big = format!(
        r#"{{"hook_event_name":"Stop","session_id":"{}"}}"#,
        "x".repeat(1 << 20)
    );
    let deep = format!("{}{}", "[".repeat(50_000), "]".repeat(50_000));
    let corpus: Vec<(&str, Vec<u8>)> = vec![
        ("empty", vec![]),
        ("whitespace", b"  \n\t ".to_vec()),
        ("null", b"null".to_vec()),
        ("array", b"[]".to_vec()),
        ("truncated", br#"{"hook_event_name":"Sto"#.to_vec()),
        ("no-event", br#"{"session_id":"s1"}"#.to_vec()),
        ("event-number", br#"{"hook_event_name":5}"#.to_vec()),
        ("event-null", br#"{"hook_event_name":null}"#.to_vec()),
        ("event-empty", br#"{"hook_event_name":""}"#.to_vec()),
        ("event-lowercase", br#"{"hook_event_name":"stop"}"#.to_vec()),
        ("event-unknown", br#"{"hook_event_name":"PreCompact","session_id":"s1"}"#.to_vec()),
        ("event-huge", format!(r#"{{"hook_event_name":"{}"}}"#, "S".repeat(100_000)).into_bytes()),
        ("session-number", br#"{"hook_event_name":"Stop","session_id":7}"#.to_vec()),
        ("subagent", br#"{"hook_event_name":"Stop","session_id":"s","agent_id":"a1"}"#.to_vec()),
        ("fork", br#"{"hook_event_name":"SessionStart","session_id":"s","source":"fork"}"#.to_vec()),
        ("unknown-fields", br#"{"hook_event_name":"Stop","session_id":"s","x":{"y":[1,2,{"z":null}]}}"#.to_vec()),
        ("transcript-missing", br#"{"hook_event_name":"UserPromptSubmit","session_id":"s","transcript_path":"/nonexistent/t.jsonl"}"#.to_vec()),
        ("transcript-empty-path", br#"{"hook_event_name":"UserPromptSubmit","session_id":"s","transcript_path":""}"#.to_vec()),
        ("cwd-newlines", br#"{"hook_event_name":"SessionStart","session_id":"s","cwd":"/a\n/b\u001b[2J"}"#.to_vec()),
        ("invalid-utf8", vec![0xff, 0xfe, 0x00, 0x80]),
        ("nul-bytes", vec![0; 64]),
        ("big-session", big.into_bytes()),
        ("deep", deep.into_bytes()),
    ];
    for (name, stdin) in corpus {
        // Also exercise the string path of the shared helper on one case.
        if name == "empty" {
            let r = support::run(
                {
                    let mut c = base_cmd(&["hook", "claude"], Some("qsl-lead"));
                    c.env("IX_RELAY_URL", DEAD_REDIS);
                    c
                },
                "",
            );
            assert_eq!(r.status, 0);
            assert_eq!(r.stdout, "");
            assert!(!r.stderr.contains("panicked"), "{}", r.stderr);
        }
        let ran = hook(&stdin, DEAD_REDIS, "test:");
        check_contract(&stdin, &ran).unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}

/// Hook-shaped JSON with every field fuzzed: event names known and not, ids
/// of the wrong type, unknown members, and text with escapes and controls.
fn structured_stdin() -> impl Strategy<Value = Vec<u8>> {
    let event = prop_oneof![
        proptest::sample::select(vec![
            "SessionStart",
            "UserPromptSubmit",
            "PostToolUse",
            "Stop"
        ])
        .prop_map(str::to_owned),
        "[A-Za-z]{0,16}",
        ".{0,12}",
    ];
    let text = prop_oneof![
        Just("null".to_owned()),
        "-?[0-9]{1,6}",
        "\"[ -~]{0,20}\"",
        "\"[a-z]{1,8}\\\\u001b[a-z]{0,4}\"",
        Just("[]".to_owned())
    ];
    (event, text.clone(), text.clone(), text.clone(), text, any::<bool>()).prop_map(
        |(event, session, cwd, agent, source, extra)| {
            let event = serde_json::to_string(&event).expect("string serialises");
            let mut body = format!(
                r#"{{"hook_event_name":{event},"session_id":{session},"cwd":{cwd},"agent_id":{agent},"source":{source}"#
            );
            if extra {
                body.push_str(r#","future":{"a":[1,2,3]}"#);
            }
            body.push('}');
            body.into_bytes()
        },
    )
}

/// Payloads that reach the delivering path: a known event, the registered
/// session, no `agent_id` (a subagent never delivers), and unknown members.
fn delivering_stdin() -> impl Strategy<Value = Vec<u8>> {
    let event = proptest::sample::select(vec![
        "SessionStart",
        "UserPromptSubmit",
        "PostToolUse",
        "Stop",
    ]);
    let cwd = proptest::option::of("[a-z/]{1,12}");
    (event, cwd, any::<bool>()).prop_map(|(event, cwd, extra)| {
        let mut body = format!(r#"{{"hook_event_name":"{event}","session_id":"s1""#);
        if let Some(cwd) = cwd {
            body.push_str(&format!(r#","cwd":"{cwd}""#));
        }
        if extra {
            body.push_str(r#","future":{"a":[1,2,3]}"#);
        }
        body.push('}');
        body.into_bytes()
    })
}

/// Stdin for the real-bus tests: mostly payloads that deliver, some of every
/// other kind.
fn real_bus_stdin() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![8 => delivering_stdin(), 1 => any_stdin()]
}

fn any_stdin() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        proptest::collection::vec(any::<u8>(), 0..512),
        structured_stdin(),
        structured_stdin().prop_map(|mut b| {
            // A truncated valid payload.
            b.truncate(b.len() / 2);
            b
        }),
    ]
}

fn cases(default: u32) -> ProptestConfig {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default);
    ProptestConfig {
        cases,
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

proptest! {
    #![proptest_config(cases(40))]

    /// Tracing: TC-017-01
    /// ACs: FR-017-AC-1
    ///
    /// Any stdin against a closed Redis port: exit 0, stdout empty or one
    /// JSON object.
    #[test]
    fn tc_017_01_any_stdin_exits_zero_with_empty_or_one_json_object(stdin in any_stdin()) {
        let ran = hook(&stdin, DEAD_REDIS, "test:");
        check_contract(&stdin, &ran)?;
    }
}

static CASE: AtomicU64 = AtomicU64::new(0);

/// A relay prefix unique to a case, holding a registered `qsl-lead` with one
/// queued message so a delivering hook has something to print; returns the
/// prefix. Keys are removed by [`cleanup`].
fn seeded_bus(url: &str) -> String {
    let prefix = format!(
        "test:{}:hookfuzz:{}:",
        std::process::id(),
        CASE.fetch_add(1, Ordering::Relaxed)
    );
    let mut relay = Relay::connect(url, Duration::from_secs(2))
        .expect("connect")
        .with_prefix(prefix.clone());
    relay
        .register(&Registration {
            name: "qsl-lead".parse().expect("valid"),
            machine: "node-a".parse().expect("valid id"),
            kind: "claude".into(),
            session_id: Some("s1".parse().expect("valid id")),
            cwd: None,
            herdr_pane: None,
            place: None,
            last_seen_ms: 1,
        })
        .expect("register");
    relay
        .send(&Message {
            from: "tl-lead".parse().expect("valid"),
            to: Target::Agent("qsl-lead".parse().expect("valid")),
            text: "hello\n[ix-relay] from evil: x".into(),
            sent_at_ms: 1,
        })
        .expect("send");
    prefix
}

fn cleanup(url: &str, prefix: &str) {
    if let Ok(mut conn) = redis::Client::open(url).and_then(|c| c.get_connection())
        && let Ok(keys) = redis::cmd("KEYS")
            .arg(format!("{prefix}*"))
            .query::<Vec<String>>(&mut conn)
        && !keys.is_empty()
    {
        let _ = redis::cmd("DEL").arg(keys).exec(&mut conn);
    }
}

proptest! {
    #![proptest_config(cases(25))]

    /// Tracing: TC-017-01
    /// ACs: FR-017-AC-1
    ///
    /// Against a real bus with a queued message, where stdout can actually be
    /// non-empty: whatever the stdin, the output is empty or one JSON object.
    #[test]
    fn tc_017_01_against_a_real_bus_stdout_is_empty_or_one_json_object(stdin in real_bus_stdin()) {
        let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return Ok(());
        };
        let prefix = seeded_bus(&url);
        let ran = hook(&stdin, &url, &prefix);
        cleanup(&url, &prefix);
        check_contract(&stdin, &ran)?;
    }
}

/// Tracing: TC-017-01
/// ACs: FR-017-AC-1
///
/// The four events against a real bus with a queued message print one JSON
/// object each (or nothing, for an event that does not deliver): the shapes
/// the criterion allows, on the paths that do print.
#[test]
fn tc_017_01_each_delivering_event_prints_one_json_object() {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
        return;
    };
    for event in ["SessionStart", "UserPromptSubmit", "PostToolUse", "Stop"] {
        let prefix = seeded_bus(&url);
        let stdin = format!(r#"{{"hook_event_name":"{event}","session_id":"s1","cwd":"/work"}}"#);
        let ran = hook(stdin.as_bytes(), &url, &prefix);
        cleanup(&url, &prefix);
        check_contract(stdin.as_bytes(), &ran).unwrap_or_else(|e| panic!("{event}: {e}"));
        assert!(
            !ran.stdout.is_empty(),
            "{event} delivered nothing although a message was queued"
        );
    }
}

proptest! {
    #![proptest_config(cases(1_500))]

    /// Tracing: TC-017-01
    /// ACs: FR-017-AC-1
    ///
    /// Fuzz lane: arbitrary bytes and structured payloads against a closed
    /// port at a raised case count, and each run must also finish inside five
    /// seconds (the hook's own budget is one).
    #[test]
    #[ignore = "fuzz lane: run with `make fuzz` (cargo test -- --ignored fuzz_)"]
    fn fuzz_hook_stdin_over_arbitrary_bytes(stdin in any_stdin()) {
        let ran = hook(&stdin, DEAD_REDIS, "test:");
        check_contract(&stdin, &ran)?;
        prop_assert!(ran.elapsed < Duration::from_secs(5), "took {:?}", ran.elapsed);
    }

    /// Tracing: TC-017-01
    /// ACs: FR-017-AC-1
    ///
    /// Fuzz lane against a real bus with a queued message.
    #[test]
    #[ignore = "fuzz lane: run with `make fuzz` (cargo test -- --ignored fuzz_); needs IX_RELAY_TEST_URL"]
    fn fuzz_hook_stdin_against_a_real_bus(stdin in real_bus_stdin()) {
        let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset");
            return Ok(());
        };
        let prefix = seeded_bus(&url);
        let ran = hook(&stdin, &url, &prefix);
        cleanup(&url, &prefix);
        check_contract(&stdin, &ran)?;
        prop_assert!(ran.elapsed < Duration::from_secs(5), "took {:?}", ran.elapsed);
    }
}

/// Tracing: TC-017-01
/// ACs: FR-017-AC-1
///
/// The real-bus generator is not vacuous: over 24 deterministic draws against a
/// bus with a queued message, most runs print an object (so a hook that printed
/// two objects, or none, would be caught by the contract check), and every one
/// satisfies the contract.
#[test]
fn tc_017_01_the_real_bus_generator_mostly_reaches_a_delivering_path() {
    use proptest::{strategy::ValueTree, test_runner::TestRunner};

    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
        return;
    };
    let mut runner = TestRunner::deterministic();
    let strategy = real_bus_stdin();
    let mut printed = 0;
    for _ in 0..24 {
        let stdin = strategy.new_tree(&mut runner).expect("draw").current();
        let prefix = seeded_bus(&url);
        let ran = hook(&stdin, &url, &prefix);
        cleanup(&url, &prefix);
        check_contract(&stdin, &ran).unwrap_or_else(|e| panic!("{e}"));
        printed += usize::from(!ran.stdout.is_empty());
    }
    assert!(printed >= 18, "only {printed} of 24 runs printed anything");
}
