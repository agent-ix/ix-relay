//! Every way a hook can fail must leave exactly one record in the local
//! spool (`<config dir>/spool/hook.jsonl`) and still exit 0; the next healthy
//! hook ships the spool to `<prefix>health:hooks`. Run against the built
//! binary, each in its own config dir so nothing touches the real home.
//!
//! The cases the binary cannot reach without a fault-injecting bus (a failing
//! pop, a failing requeue, a spent budget at each stage) are covered in
//! `hook_run`'s own tests. The Redis-backed cases need `IX_RELAY_TEST_URL`
//! (`make test-redis` sets it) and skip with a message without it.
#![expect(
    clippy::unwrap_used,
    reason = "test code: a failed unwrap is the failure signal"
)]

mod support;

use std::{io::Write, num::NonZeroUsize, path::PathBuf, process::Stdio, time::Duration};

use relay_core::{Claim, Message, Popper, Registration, Relay, Target};
use serde_json::Value;
use support::{Run, base_cmd, run, spool_path};

const STOP: &str = r#"{"hook_event_name":"Stop","session_id":"s1","cwd":"/work"}"#;

/// A hook payload for `event`.
fn payload(event: &str) -> String {
    format!(r#"{{"hook_event_name":"{event}","session_id":"s1","cwd":"/work"}}"#)
}

/// A port nothing listens on: connecting is refused at once.
const DEAD_REDIS: (&str, &str) = ("redis://127.0.0.1:1", "test:");

/// One test's private config dir, removed when dropped.
struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn config(&self) -> PathBuf {
        self.dir.path().join("config.toml")
    }

    /// Runs `agent-msg <args>` with this env's config and, when given, a bus.
    fn run(
        &self,
        args: &[&str],
        name: Option<&str>,
        bus: Option<(&str, &str)>,
        stdin: &str,
    ) -> Run {
        let mut cmd = base_cmd(args, name);
        cmd.env("IX_RELAY_CONFIG", self.config());
        if let Some((url, prefix)) = bus {
            cmd.env("IX_RELAY_URL", url);
            cmd.env("IX_RELAY_PREFIX", prefix);
        }
        run(cmd, stdin)
    }

    fn hook(&self, name: Option<&str>, bus: Option<(&str, &str)>, stdin: &str) -> Run {
        self.run(&["hook", "claude"], name, bus, stdin)
    }

    fn records(&self) -> Vec<Value> {
        std::fs::read_to_string(spool_path(&self.config()))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

/// `(stage, error_kind)` of the only record, failing if there is not exactly one.
fn only_record(env: &Env) -> (String, String) {
    let records = env.records();
    assert_eq!(records.len(), 1, "records: {records:?}");
    let field = |name: &str| records[0][name].as_str().unwrap().to_owned();
    (field("stage"), field("error_kind"))
}

fn test_url() -> Option<String> {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
        return None;
    };
    Some(url)
}

#[test]
fn a_dead_redis_leaves_one_connect_record_and_exits_zero() {
    let env = Env::new();
    let r = env.hook(Some("hub:qsl:code"), Some(DEAD_REDIS), STOP);
    assert_eq!(r.status, 0);
    assert_eq!(r.stdout, "");
    assert_eq!(only_record(&env), ("connect".into(), "connection".into()));
}

/// Tracing: TC-030-01
/// ACs: FR-030-AC-1
///
/// (Named by bus name rather than derived, which the spec's "after it
/// identified a lead" covers too.)
#[test]
fn tc_030_01_three_failed_post_tool_use_hooks_leave_three_records() {
    let env = Env::new();
    for _ in 0..3 {
        let r = env.hook(
            Some("hub:qsl:code"),
            Some(DEAD_REDIS),
            &payload("PostToolUse"),
        );
        assert_eq!(r.status, 0);
        assert_eq!(r.stdout, "");
    }
    let records = env.records();
    assert_eq!(records.len(), 3, "{records:?}");
    assert!(records.iter().all(|r| r["event"] == "PostToolUse"));
}

/// Tracing: TC-017-05
/// ACs: FR-017-AC-5
#[test]
fn tc_017_05_a_closed_port_exits_zero_with_empty_stdout_for_every_event() {
    for event in ["SessionStart", "UserPromptSubmit", "PostToolUse", "Stop"] {
        let env = Env::new();
        let r = env.hook(Some("hub:qsl:code"), Some(DEAD_REDIS), &payload(event));
        assert_eq!(r.status, 0, "{event}");
        assert_eq!(r.stdout, "", "{event}");
    }
}

#[test]
fn unparseable_hook_input_leaves_one_decode_record() {
    let env = Env::new();
    let r = env.hook(Some("hub:qsl:code"), None, "this is not json");
    assert_eq!(r.status, 0);
    assert_eq!(only_record(&env), ("decode".into(), "malformed".into()));
}

/// Tracing: TC-017-03
/// ACs: FR-017-AC-3
#[test]
fn legacy_name_metadata_outside_herdr_does_not_register_or_fail() {
    let env = Env::new();
    let r = env.hook(Some("Bad Name"), Some(DEAD_REDIS), STOP);
    assert_eq!(r.status, 0);
    assert_eq!(r.stdout, "");
    assert_eq!(r.stderr, "");
    assert!(env.records().is_empty());
}

#[test]
fn a_malformed_config_leaves_one_config_record() {
    let env = Env::new();
    std::fs::write(env.config(), "this = [is not toml").unwrap();
    let r = env.hook(Some("hub:qsl:code"), None, STOP);
    assert_eq!(r.status, 0);
    assert_eq!(only_record(&env), ("config".into(), "config".into()));
}

/// Tracing: TC-017-02
/// ACs: FR-017-AC-2
#[test]
fn tc_017_02_an_unknown_flag_after_hook_exits_zero_and_leaves_one_args_record() {
    let env = Env::new();
    let r = env.run(
        &["hook", "claude", "--bogus"],
        Some("hub:qsl:code"),
        None,
        STOP,
    );
    assert_eq!(r.status, 0);
    assert_eq!(only_record(&env), ("args".into(), "args".into()));
}

#[test]
fn a_session_that_is_not_a_lead_and_a_subagent_leave_nothing() {
    let env = Env::new();
    // No name and no herdr pane: silent, before anything can fail.
    assert_eq!(env.hook(None, None, STOP).status, 0);
    let subagent = r#"{"hook_event_name":"Stop","session_id":"s1","agent_id":"a"}"#;
    assert_eq!(env.hook(Some("hub:qsl:code"), None, subagent).status, 0);
    assert!(env.records().is_empty());
}

/// Tracing: TC-030-05
/// ACs: FR-030-AC-5
///
/// The record has a fixed set of fields, none of them message text.
#[test]
fn tc_030_05_records_are_typed_and_carry_no_error_text() {
    let env = Env::new();
    env.hook(
        Some("hub:qsl:code"),
        Some(("redis://127.0.0.1:1", "test:")),
        STOP,
    );
    let record = &env.records()[0];
    let mut keys: Vec<&str> = record
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "at_ms",
            "error_kind",
            "event",
            "id",
            "lead",
            "machine",
            "session_id",
            "stage"
        ]
    );
    assert_eq!(
        record["id"].as_str().map(str::len),
        Some(32),
        "minted at append"
    );
    assert_eq!(record["event"], "Stop");
    assert_eq!(record["session_id"], "s1");
    assert_eq!(record["lead"], serde_json::Value::Null);
    assert!(!record["machine"].as_str().unwrap().is_empty());
    assert!(record["at_ms"].as_i64().unwrap() > 1_700_000_000_000);
}

/// Tracing: TC-030-02
/// ACs: FR-030-AC-2
///
/// The next healthy hook ships every record to `health:hooks` and empties the
/// file; the one after that ships nothing more.
#[test]
fn tc_030_02_the_next_healthy_hook_ships_three_records_once() {
    let Some(url) = test_url() else { return };
    support::until_in_time(|attempt| {
        let prefix = format!("test:{}:health-{attempt}:", std::process::id());
        let env = Env::new();
        for _ in 0..3 {
            env.hook(
                Some("hub:qsl:code"),
                Some(("redis://127.0.0.1:1", &prefix)),
                &payload("PostToolUse"),
            );
        }
        assert_eq!(env.records().len(), 3);

        let healthy = env.hook(
            Some("hub:qsl:code"),
            Some((&url, &prefix)),
            &payload("PostToolUse"),
        );
        assert_eq!(healthy.status, 0, "stderr: {}", healthy.stderr);
        // A healthy hook with no time left ships nothing and leaves the spool as
        // it was, plus the record of why.
        if env.records().len() > 3 && support::ran_out_of_time(&env.records()[3..]) {
            assert_eq!(
                env.records().len(),
                4,
                "the three, and one for the healthy hook"
            );
            return false;
        }
        assert!(env.records().is_empty(), "the spool was emptied");

        let mut relay = Relay::connect(&url, Duration::from_secs(2))
            .unwrap()
            .with_prefix(prefix.clone());
        let shipped = relay.hook_health(100).unwrap().entries;
        assert_eq!(shipped.len(), 3, "{shipped:?}");
        for entry in &shipped {
            let record: Value = serde_json::from_str(&entry.record).unwrap();
            assert_eq!(
                entry.id.as_deref(),
                record["id"].as_str(),
                "the stream carries the id"
            );
            assert_eq!(record["stage"], "connect");
            assert_eq!(entry.lead, None);
            assert!(!entry.machine.as_str().is_empty());
            assert_eq!(record["machine"], entry.machine.as_str());
        }

        // Idempotent: nothing left to ship.
        env.hook(
            Some("hub:qsl:code"),
            Some((&url, &prefix)),
            &payload("PostToolUse"),
        );
        assert_eq!(relay.hook_health(100).unwrap().entries.len(), 3);
        true
    });
}

/// Two messages queued for `receiver`, with `sender` registered, on `url`.
fn seed_two(url: &str, prefix: &str) -> Relay {
    let mut relay = Relay::connect(url, Duration::from_secs(2))
        .unwrap()
        .with_prefix(prefix);
    for lead in ["hub:EA:code", "hub:EA:sender"] {
        relay
            .register(&Registration {
                name: if lead == "hub:EA:code" {
                    relay_core::AgentName::for_session(
                        &"hub".parse().unwrap(),
                        &"s1".parse().unwrap(),
                    )
                } else {
                    lead.parse().unwrap()
                },
                machine: "hub".parse().unwrap(),
                kind: "claude".into(),
                session_id: None,
                cwd: None,
                herdr_pane: None,
                place: None,
                last_seen_ms: 1,
            })
            .unwrap();
    }
    for text in ["first", "second"] {
        relay
            .send(&Message {
                from: "hub:EA:sender".parse().unwrap(),
                to: Target::Agent(relay_core::AgentName::for_session(
                    &"hub".parse().unwrap(),
                    &"s1".parse().unwrap(),
                )),
                text: text.into(),
                sent_at_ms: 1,
            })
            .unwrap();
    }
    relay
        .commit_identity(&relay_core::IdentityCommit {
            session_name: Some((
                "s1".parse().unwrap(),
                relay_core::AgentName::for_session(&"hub".parse().unwrap(), &"s1".parse().unwrap()),
            )),
            ..Default::default()
        })
        .unwrap();
    relay
}

/// Tracing: TC-020-01
/// ACs: FR-020-AC-1
///
/// The hook's stdout is a pipe whose reader has already gone, so the write
/// fails with a broken pipe after the pop succeeded.
#[test]
fn tc_020_01_a_closed_stdout_leaves_the_messages_in_the_inbox_in_order() {
    let Some(url) = test_url() else { return };
    support::until_in_time(|attempt| {
        let prefix = format!("test:{}:closed-stdout-{attempt}:", std::process::id());
        let mut relay = seed_two(&url, &prefix);
        let env = Env::new();

        let mut cmd = base_cmd(&["hook", "claude"], Some("hub:EA:code"));
        cmd.env("IX_RELAY_CONFIG", env.config());
        cmd.env("IX_RELAY_URL", &url);
        cmd.env("IX_RELAY_PREFIX", &prefix);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        let mut child = cmd.spawn().unwrap();
        // Close the read end before the child, still starting up, can write.
        drop(child.stdout.take());
        let mut stdin = child.stdin.take().unwrap();
        // The child may already have exited on a read error: a broken pipe is fine.
        let _ = stdin.write_all(STOP.as_bytes());
        drop(stdin);
        let out = child.wait_with_output().unwrap();

        assert_eq!(out.status.code(), Some(0));
        // A hook with no time to put the messages back leaves them in flight.
        if support::ran_out_of_time(&env.records()) {
            // Whatever the hook did with the mail, none of it is lost: it is in
            // the inbox, or in flight until its lease returns it (FR-020).
            let receiver: relay_core::AgentName =
                relay_core::AgentName::for_session(&"hub".parse().unwrap(), &"s1".parse().unwrap());
            let safe = relay.inbox_len(&receiver).unwrap() + relay.inflight_len(&receiver).unwrap();
            assert_eq!(safe, 2, "the mail of a run that ran out of time is kept");
            return false;
        }
        let popped = relay
            .pop_inbox(
                &relay_core::AgentName::for_session(
                    &"hub".parse().unwrap(),
                    &"s1".parse().unwrap(),
                ),
                NonZeroUsize::new(10).unwrap(),
            )
            .unwrap();
        let texts: Vec<&str> = popped.messages.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(texts, ["first", "second"]);
        assert_eq!(only_record(&env), ("output".into(), "io".into()));
        assert_eq!(
            relay
                .inflight_len(&relay_core::AgentName::for_session(
                    &"hub".parse().unwrap(),
                    &"s1".parse().unwrap()
                ))
                .unwrap(),
            0,
            "the batch went back to the inbox, it is not also still in flight"
        );
        true
    });
}

/// Tracing: TC-020-04
/// ACs: FR-020-AC-3, FR-020-AC-4
///
/// A hook killed after its pop and before it wrote anything: simulated by a
/// pop that nobody acknowledges, with a lease that has already run out (the
/// pop takes the messages out of the inbox and that is all that happened). The
/// next hook, the real binary, returns them in order, marked as redelivered,
/// and acknowledges them: nothing is left in the inbox or in flight.
#[test]
fn tc_020_04_a_hook_killed_after_its_pop_has_its_messages_delivered_by_the_next_hook() {
    let Some(url) = test_url() else { return };
    support::until_in_time(|attempt| {
        let prefix = format!("test:{}:killed-hook-{attempt}:", std::process::id());
        let mut relay = seed_two(&url, &prefix);
        let receiver: relay_core::AgentName =
            relay_core::AgentName::for_session(&"hub".parse().unwrap(), &"s1".parse().unwrap());
        let killed = relay
            .pop_inbox_budget(
                &receiver,
                NonZeroUsize::new(50).unwrap(),
                6144,
                Claim::new(Popper::Hook).with_lease(Duration::ZERO),
            )
            .unwrap();
        assert_eq!(killed.messages.len(), 2);
        assert_eq!(relay.inbox_len(&receiver).unwrap(), 0, "out of the inbox");
        let env = Env::new();

        let r = env.hook(Some("hub:EA:code"), Some((&url, &prefix)), STOP);

        assert_eq!(r.status, 0, "stderr: {}", r.stderr);
        if support::ran_out_of_time(&env.records()) {
            return false;
        }
        let out: Value = serde_json::from_str(&r.stdout).expect("json stdout");
        assert_eq!(
            out["reason"],
            "[ix-relay] from hub:EA:sender (redelivered): first\n[ix-relay] from hub:EA:sender (redelivered): second"
        );
        assert!(env.records().is_empty(), "not a failure of the hook");
        assert_eq!(relay.inbox_len(&receiver).unwrap(), 0);
        assert_eq!(relay.inflight_len(&receiver).unwrap(), 0, "acknowledged");
        true
    });
}

/// Tracing: TC-020-04
/// ACs: FR-020-AC-3
///
/// A batch whose lease has not run out is not offered to a second hook: the
/// messages are with whoever popped them.
#[test]
fn tc_020_04_a_batch_still_leased_is_not_delivered_to_a_second_hook() {
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:leased-hook:", std::process::id());
    let mut relay = seed_two(&url, &prefix);
    let receiver: relay_core::AgentName =
        relay_core::AgentName::for_session(&"hub".parse().unwrap(), &"s1".parse().unwrap());
    relay
        .pop_inbox_budget(
            &receiver,
            NonZeroUsize::new(50).unwrap(),
            6144,
            Claim::new(Popper::Hook),
        )
        .unwrap();
    let env = Env::new();

    let r = env.hook(Some("hub:EA:code"), Some((&url, &prefix)), STOP);

    assert_eq!(r.status, 0, "stderr: {}", r.stderr);
    assert_eq!(r.stdout.trim(), "", "nothing to deliver while leased");
    assert_eq!(relay.inflight_len(&receiver).unwrap(), 2, "still in flight");
}

/// Tracing: TC-020-02
/// ACs: FR-020-AC-2
///
/// The registry key holds the wrong type, so reading addresses fails after
/// the pop; the hook prints both messages signed with bus names.
#[test]
fn tc_020_02_a_registry_read_failure_after_the_pop_still_delivers_both_messages() {
    let Some(url) = test_url() else { return };
    support::until_in_time(|attempt| {
        let prefix = format!("test:{}:bad-registry-{attempt}:", std::process::id());
        let _relay = seed_two(&url, &prefix);
        let mut raw = redis::Client::open(url.as_str())
            .unwrap()
            .get_connection()
            .unwrap();
        let () = redis::cmd("SET")
            .arg(format!("{prefix}agents"))
            .arg("not a hash")
            .query(&mut raw)
            .unwrap();
        let env = Env::new();

        let r = env.hook(Some("hub:EA:code"), Some((&url, &prefix)), STOP);

        assert_eq!(r.status, 0, "stderr: {}", r.stderr);
        if support::ran_out_of_time(&env.records()) {
            return false;
        }
        let out: Value = serde_json::from_str(&r.stdout).expect("json stdout");
        assert_eq!(out["decision"], "block");
        assert_eq!(
            out["reason"],
            "[ix-relay] from hub:EA:sender: first\n[ix-relay] from hub:EA:sender: second"
        );
        assert!(env.records().is_empty(), "not a failure of the hook");
        true
    });
}

/// (args, name, config contents, stdin, expected stage)
type ClosedStderrCase = (
    &'static [&'static str],
    Option<&'static str>,
    Option<&'static str>,
    &'static str,
    &'static str,
);

/// Runs `cmd` with its stderr a pipe whose reader has already gone, so any
/// write to stderr fails with a broken pipe (and `eprintln!` would panic).
fn run_with_closed_stderr(mut cmd: std::process::Command, stdin: &str) -> i32 {
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    let mut child = cmd.spawn().unwrap();
    drop(child.stderr.take());
    let mut pipe = child.stdin.take().unwrap();
    let _ = pipe.write_all(stdin.as_bytes());
    drop(pipe);
    child
        .wait_with_output()
        .unwrap()
        .status
        .code()
        .unwrap_or(-1)
}

/// Tracing: TC-030-08
/// ACs: FR-030-AC-8
///
/// A closed stderr must not turn a recorded failure into a panic (exit 101)
/// or lose its record: for every stage the binary can reach without a Redis.
#[test]
fn tc_030_08_a_closed_stderr_still_exits_zero_and_leaves_one_record_at_every_reachable_stage() {
    // (args, name, config contents, stdin, expected stage)
    let cases: [ClosedStderrCase; 4] = [
        (
            &["hook", "claude", "--bogus"],
            Some("hub:qsl:code"),
            None,
            STOP,
            "args",
        ),
        (
            &["hook", "claude"],
            Some("hub:qsl:code"),
            Some("this = [is not toml"),
            STOP,
            "config",
        ),
        (
            &["hook", "claude"],
            Some("hub:qsl:code"),
            None,
            "not json",
            "decode",
        ),
        (
            &["hook", "claude"],
            Some("hub:qsl:code"),
            None,
            STOP,
            "connect",
        ),
    ];
    for (args, name, config, stdin, stage) in cases {
        let env = Env::new();
        if let Some(config) = config {
            std::fs::write(env.config(), config).unwrap();
        }
        let mut cmd = base_cmd(args, name);
        cmd.env("IX_RELAY_CONFIG", env.config());
        cmd.env("IX_RELAY_URL", DEAD_REDIS.0);
        cmd.env("IX_RELAY_PREFIX", DEAD_REDIS.1);
        assert_eq!(run_with_closed_stderr(cmd, stdin), 0, "{stage}");
        let records = env.records();
        assert_eq!(records.len(), 1, "{stage}: {records:?}");
        assert_eq!(records[0]["stage"], stage);
    }
    // The stdin faults reach the same recorder: too large is a `decode` record.
    let env = Env::new();
    let mut cmd = base_cmd(&["hook", "claude"], Some("hub:qsl:code"));
    cmd.env("IX_RELAY_CONFIG", env.config());
    cmd.env("IX_RELAY_URL", DEAD_REDIS.0);
    cmd.env("IX_RELAY_PREFIX", DEAD_REDIS.1);
    let big = " ".repeat((8 << 20) + 1);
    assert_eq!(run_with_closed_stderr(cmd, &big), 0, "too_large");
    let records = env.records();
    assert_eq!(records.len(), 1, "too_large: {records:?}");
    // Reading 8 MiB inside the 300 ms the hook allows itself for stdin depends
    // on the machine: a loaded one records the timeout instead (the sizes are
    // pinned by `stdin::tests`). Either way it is one `decode` record.
    assert_eq!(records[0]["stage"], "decode");
    assert!(
        matches!(
            records[0]["error_kind"].as_str(),
            Some("too_large" | "stdin_timeout")
        ),
        "{records:?}"
    );
}

/// Tracing: TC-030-09
/// ACs: FR-030-AC-9
///
/// A relative `IX_RELAY_CONFIG` would put `spool/` in the working directory
/// (a lead's git repo): the spool refuses it and the hook still exits 0.
#[test]
fn tc_030_09_a_relative_config_path_never_writes_a_spool_into_the_working_directory() {
    let cwd = tempfile::tempdir().unwrap();
    let mut cmd = base_cmd(&["hook", "claude"], Some("hub:qsl:code"));
    cmd.env("IX_RELAY_CONFIG", "config.toml");
    cmd.env("IX_RELAY_URL", DEAD_REDIS.0);
    cmd.current_dir(cwd.path());
    let r = run(cmd, STOP);
    assert_eq!(r.status, 0);
    assert!(std::fs::read_dir(cwd.path()).unwrap().next().is_none());
}

/// Stale metadata cannot opt a one-off process into lead enrollment.
#[test]
fn legacy_name_metadata_does_not_touch_a_healthy_bus() {
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:ignored-name:", std::process::id());
    let env = Env::new();
    let r = env.hook(Some("Bad Name"), Some((&url, &prefix)), STOP);
    assert_eq!(r.status, 0);
    assert!(env.records().is_empty());
    let mut relay = Relay::connect(&url, Duration::from_secs(2))
        .unwrap()
        .with_prefix(prefix);
    assert!(relay.agents().unwrap().registrations.is_empty());
    assert!(relay.hook_health(10).unwrap().entries.is_empty());
}
