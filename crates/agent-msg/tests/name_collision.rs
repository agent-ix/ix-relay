//! A hook that cannot find a free bus name must leave the holder's registration
//! alone and register nothing, and one that cannot check whether the holder is
//! live must treat it as live and take the next free name (FR-022-AC-6,
//! FR-022-AC-7, FR-022-AC-8, AGE-2034).
//!
//! Redis-backed (`IX_RELAY_TEST_URL`, set by `make test-redis`; each test skips
//! with a message without it). The end-to-end cases run the built binary with a
//! fake `herdr` first on `PATH`; the property test drives the real name
//! resolver against a real registry and models the hook's commit.
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: a failed unwrap/expect is the failure signal"
)]

mod support;

use std::{cell::Cell, path::Path, time::Duration};

use proptest::prelude::*;
use relay_core::{PaneId, Registration, Relay, SessionId};
use relay_ops::naming::{CollisionError, PaneLiveness, ProcessError, resolve_collision};
use serde_json::Value;
use support::{Scratch, base_cmd, fake_herdr_dir, run, spool_path};

/// This machine's bus name for the spawned hook.
const MACHINE: &str = "node-a";
/// The session the hook under test belongs to.
const MY_SESSION: &str = "me-sid";
const SESSION_START: &str =
    r#"{"hook_event_name":"SessionStart","session_id":"me-sid","cwd":"/work"}"#;
const POST_TOOL_USE: &str =
    r#"{"hook_event_name":"PostToolUse","session_id":"me-sid","cwd":"/work"}"#;

/// One workspace labelled `QSL` with one lone pane: derives to `qsl`.
const WORKSPACES: &str = r#"[{"workspace_id":"wQ","label":"QSL","number":1}]"#;
const TABS: &str = r#"[{"tab_id":"wQ:t1","label":"1","number":1}]"#;
const PANES: &str = r#"[{"pane_id":"wQ:p1","tab_id":"wQ:t1"}]"#;

/// `agent get` answers for the holder's pane `wH:p1`: still hosting
/// `holder-sid`.
const HOLDER_LIVE: &str = r#""wH:p1") echo '{"result":{"agent":{"agent_session":{"kind":"id","value":"holder-sid"}}}}' ;;"#;
/// `agent get` for the holder's pane fails with an error that is not
/// `agent_not_found`, like a herdr that is down.
const HOLDER_ERROR: &str =
    r#""wH:p1") echo '{"error":{"code":"boom","message":"herdr is down"}}' >&2; exit 1 ;;"#;

fn test_url() -> Option<String> {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
        return None;
    };
    Some(url)
}

fn relay(url: &str, prefix: &str) -> Relay {
    Relay::connect(url, Duration::from_secs(5))
        .unwrap()
        .with_prefix(prefix)
}

fn registration(name: &str, machine: &str, session: &str, pane: &str) -> Registration {
    Registration {
        name: name.parse().unwrap(),
        machine: machine.parse().unwrap(),
        kind: "claude".into(),
        session_id: Some(session.parse().unwrap()),
        cwd: None,
        herdr_pane: Some(pane.parse().unwrap()),
        place: None,
        last_seen_ms: 1,
    }
}

/// Runs one hook (`payload`) for [`MY_SESSION`] in pane `wQ:p1`, its spool in
/// `config_dir`, and returns what it printed.
fn hook(
    url: &str,
    prefix: &str,
    herdr: &Scratch,
    config_dir: &Path,
    payload: &str,
) -> support::Run {
    let mut cmd = base_cmd(&["hook", "claude"], None);
    cmd.env("IX_RELAY_URL", url)
        .env("IX_RELAY_PREFIX", prefix)
        .env("IX_RELAY_MACHINE", MACHINE)
        .env("IX_RELAY_CONFIG", config_dir.join("config.toml"))
        .env("HERDR_PANE_ID", "wQ:p1")
        .env("HERDR_TAB_ID", "wQ:t1")
        .env("HERDR_WORKSPACE_ID", "wQ")
        .env(
            "PATH",
            format!(
                "{}:{}",
                herdr.path().display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        );
    run(cmd, payload)
}

/// The spool records the hooks run in `config_dir` left.
fn spool_records(config_dir: &Path) -> Vec<Value> {
    read_records(&spool_path(&config_dir.join("config.toml")))
}

/// The records a healthy hook shipped from the spool to `<prefix>health:hooks`.
fn shipped_records(url: &str, prefix: &str) -> Vec<Value> {
    let mut conn = redis::Client::open(url).unwrap().get_connection().unwrap();
    let reply: redis::streams::StreamRangeReply = redis::cmd("XRANGE")
        .arg(format!("{prefix}health:hooks"))
        .arg("-")
        .arg("+")
        .query(&mut conn)
        .unwrap();
    reply
        .ids
        .iter()
        .filter_map(|entry| entry.get::<String>("rec"))
        .map(|json| serde_json::from_str(&json).unwrap())
        .collect()
}

/// Registers `qsl` (held by `holder_machine`), `qsl-<machine>` and `qsl-2` to
/// `qsl-1000` to other sessions: every name a lead deriving `qsl` could take.
fn take_every_variant(relay: &mut Relay, holder_machine: &str) -> Registration {
    let holder = registration("qsl", holder_machine, "holder-sid", "wH:p1");
    relay.register(&holder).unwrap();
    relay
        .register(&registration("qsl-node-a", "hub", "x", "wX:p1"))
        .unwrap();
    for n in 2..=relay_core::naming::MAX_VARIANT_NUMBER {
        relay
            .register(&registration(&format!("qsl-{n}"), "hub", "x", "wX:p1"))
            .unwrap();
    }
    holder
}

fn read_records(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// Asserts the hook registered nothing for its session, cached no name, left the
/// holder as it was, printed nothing and exited 0. Holds for every outcome of a
/// hook that found no name, whether it ran out of resolver variants or of time.
fn assert_nothing_registered(relay: &mut Relay, holder_before: &Registration, run: &support::Run) {
    assert_eq!(
        run.status, 0,
        "the hook never fails the turn: {}",
        run.stderr
    );
    assert_eq!(run.stdout, "");
    let holder = relay
        .agent(&holder_before.name)
        .unwrap()
        .expect("the holder is still registered");
    assert_eq!(holder.session_id, holder_before.session_id);
    assert_eq!(holder.machine, holder_before.machine);
    assert_eq!(holder.herdr_pane, holder_before.herdr_pane);
    assert_eq!(holder.last_seen_ms, holder_before.last_seen_ms);
    let mine = relay
        .agents()
        .unwrap()
        .registrations
        .into_iter()
        .filter(|r| r.session_id.as_ref().map(|v| v.as_str()) == Some(MY_SESSION))
        .count();
    assert_eq!(mine, 0, "this session must stay unregistered");
    assert_eq!(
        relay
            .probe_session(&MY_SESSION.parse().expect("valid id"))
            .unwrap()
            .name,
        None
    );
}

/// The bus name cached for [`MY_SESSION`].
fn session_name(relay: &mut Relay) -> Option<relay_core::AgentName> {
    relay
        .probe_session(&MY_SESSION.parse().expect("valid id"))
        .unwrap()
        .name
}

/// Whether `record` is what a hook leaves when the machine was too slow for it
/// (AGE-2059): its time ran out before herdr could name the session
/// (`budget`/`budget_exhausted`), herdr itself timed out or failed
/// (`derive`/`herdr_failed`), or a Redis read timed out while the resolver walked
/// the variants (`derive`/`timeout`). Any of them is one honest record of a hook
/// that did not get to finish; none is `no_free_name`.
fn is_out_of_time(record: &Value) -> bool {
    matches!(
        (record["stage"].as_str(), record["error_kind"].as_str()),
        (Some("budget"), Some("budget_exhausted"))
            | (Some("derive"), Some("herdr_failed" | "timeout"))
    )
}

/// Runs `payload` as a hook, each time in a fresh config dir, until one finds
/// that every variant is taken, and returns that run with its config dir.
///
/// The resolver walks 1,000 registered names, so how long the hook takes depends
/// on the machine, and the hook's budget (which the test cannot widen) can run
/// out first. Every run, whichever way it ends, must leave the holder untouched,
/// this session unregistered and exactly one record: a `no_free_name` record
/// naming the candidate, or one of the out-of-time records ([`is_out_of_time`]).
/// That an out-of-time hook records exactly once is what the hook's own tests pin
/// deterministically (`hook_run`'s `tc_030_08_a_herdr_failure...` and the
/// `budget` cases); a run cut short is retried, and only a run that reached the
/// end of the variants proves `no_free_name`.
fn hook_until_no_free_name(
    (url, prefix): (&str, &str),
    relay: &mut Relay,
    holder: &Registration,
    herdr: &Scratch,
    payload: &str,
) -> (support::Run, tempfile::TempDir) {
    for attempt in 1..=support::OUT_OF_TIME_ATTEMPTS {
        let config = tempfile::tempdir().unwrap();
        let run = hook(url, prefix, herdr, config.path(), payload);
        assert_nothing_registered(relay, holder, &run);
        let records = spool_records(config.path());
        assert_eq!(records.len(), 1, "{records:?}");
        if is_out_of_time(&records[0]) {
            eprintln!("attempt {attempt}: the hook ran out of time, trying again: {records:?}");
            continue;
        }
        assert_eq!(records[0]["stage"], "derive");
        assert_eq!(records[0]["error_kind"], "no_free_name");
        assert_eq!(
            records[0]["candidate"], "qsl",
            "the record names the collided name"
        );
        return (run, config);
    }
    panic!(
        "{} hooks in a row ran out of time: this machine is too loaded",
        support::OUT_OF_TIME_ATTEMPTS
    );
}

/// The holder is as it was registered.
fn assert_holder_unchanged(relay: &mut Relay, before: &Registration) {
    let holder = relay
        .agent(&before.name)
        .unwrap()
        .expect("the holder is still registered");
    assert_eq!(holder.session_id, before.session_id);
    assert_eq!(holder.machine, before.machine);
    assert_eq!(holder.herdr_pane, before.herdr_pane);
    assert_eq!(holder.last_seen_ms, before.last_seen_ms);
}

/// Tracing: TC-022-06
/// ACs: FR-022-AC-6
///
/// With the candidate and every variant taken, whichever branch (same-machine
/// live holder, or a holder on another machine widening with `-<machine>`), the
/// hook registers nothing and the holder is untouched.
#[test]
fn tc_022_06_every_variant_taken_leaves_the_holder_untouched() {
    let Some(url) = test_url() else { return };
    for (case, holder_machine) in [("same", MACHINE), ("other", "hub")] {
        let prefix = format!("test:{}:collision-nofree-{case}:", std::process::id());
        let mut relay = relay(&url, &prefix);
        let holder = take_every_variant(&mut relay, holder_machine);
        let scratch = Scratch::new("ix-relay-collision-");
        let log = scratch.path().join("calls.log");
        let herdr = fake_herdr_dir(WORKSPACES, TABS, PANES, HOLDER_LIVE, &log, false);
        hook_until_no_free_name((&url, &prefix), &mut relay, &holder, &herdr, SESSION_START);
        assert_holder_unchanged(&mut relay, &holder);
    }
}

/// Tracing: TC-022-08
/// ACs: FR-022-AC-8
///
/// A session that finds no free name and then makes many tool calls leaves one
/// record, not one per call, and the calls after the first make no herdr call.
#[test]
fn tc_022_08_many_post_tool_use_hooks_leave_one_record_and_stop_asking_herdr() {
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:collision-negcache:", std::process::id());
    let mut relay = relay(&url, &prefix);
    let holder = take_every_variant(&mut relay, MACHINE);
    let scratch = Scratch::new("ix-relay-collision-");
    let log = scratch.path().join("calls.log");
    let herdr = fake_herdr_dir(WORKSPACES, TABS, PANES, HOLDER_LIVE, &log, false);
    let herdr_calls = || {
        std::fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .count()
    };
    // The first hook derives, finds no free name and marks the session. The mark
    // is best effort: a hook whose time ran out just after the resolver finished
    // records the failure without setting it. A run the machine was too slow for,
    // in either way, is repeated (see `hook_until_no_free_name`).
    let mut marked = None;
    for _ in 0..support::OUT_OF_TIME_ATTEMPTS {
        let (_, config) =
            hook_until_no_free_name((&url, &prefix), &mut relay, &holder, &herdr, POST_TOOL_USE);
        let probe = relay
            .probe_session(&MY_SESSION.parse().expect("valid id"))
            .unwrap();
        if probe.unresolved {
            marked = Some(config);
            break;
        }
    }
    let config = marked.expect("no hook set the mark: this machine is too loaded");
    let calls_after_first = herdr_calls();
    assert!(
        calls_after_first > 0,
        "the first hook derived through herdr"
    );
    for _ in 0..4 {
        let run = hook(&url, &prefix, &herdr, config.path(), POST_TOOL_USE);
        assert_eq!(run.status, 0, "{}", run.stderr);
        assert_eq!(run.stdout, "");
    }
    assert_eq!(
        herdr_calls(),
        calls_after_first,
        "later hooks skipped the derivation"
    );
    let records = spool_records(config.path());
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(records[0]["error_kind"], "no_free_name");
    assert_holder_unchanged(&mut relay, &holder);
    assert_eq!(
        relay
            .probe_session(&MY_SESSION.parse().expect("valid id"))
            .unwrap()
            .name,
        None
    );
}

/// Tracing: TC-022-07
/// ACs: FR-022-AC-7
///
/// A same-machine holder whose liveness herdr cannot tell is treated as live:
/// the hook registers under the next free number, leaves the holder untouched
/// and records one non-fatal note.
#[test]
fn tc_022_07_a_herdr_liveness_error_takes_the_next_free_name_and_notes_it() {
    let Some(url) = test_url() else { return };
    let scratch = Scratch::new("ix-relay-collision-");
    let log = scratch.path().join("calls.log");
    let herdr = fake_herdr_dir(WORKSPACES, TABS, PANES, HOLDER_ERROR, &log, false);
    // A hook the machine was too slow for records why and registers nothing
    // (`is_out_of_time`); it is run again on a fresh bus and spool.
    for attempt in 1..=support::OUT_OF_TIME_ATTEMPTS {
        let prefix = format!("test:{}:collision-liveness-{attempt}:", std::process::id());
        let mut relay = relay(&url, &prefix);
        let holder = registration("qsl", MACHINE, "holder-sid", "wH:p1");
        relay.register(&holder).unwrap();
        let config = tempfile::tempdir().unwrap();
        let run = hook(&url, &prefix, &herdr, config.path(), SESSION_START);
        assert_eq!(run.status, 0, "{}", run.stderr);
        assert_holder_unchanged(&mut relay, &holder);
        let Some(mine) = relay.agent(&"qsl-2".parse().unwrap()).unwrap() else {
            let records = spool_records(config.path());
            assert!(
                !records.is_empty() && records.iter().all(is_out_of_time),
                "no name taken, and not for want of time: {records:?}"
            );
            eprintln!("attempt {attempt}: the hook ran out of time, trying again: {records:?}");
            continue;
        };
        assert_eq!(
            mine.session_id.as_ref().map(|v| v.as_str()),
            Some(MY_SESSION)
        );
        // The hook delivered, so it also shipped its spool.
        let records = shipped_records(&url, &prefix);
        assert_eq!(records.len(), 1, "{records:?}");
        assert_eq!(records[0]["stage"], "derive");
        assert_eq!(records[0]["error_kind"], "liveness_unknown");
        assert_eq!(records[0]["candidate"], "qsl");
        return;
    }
    panic!(
        "{} hooks in a row ran out of time: this machine is too loaded",
        support::OUT_OF_TIME_ATTEMPTS
    );
}

/// Tracing: TC-022-06
/// ACs: FR-022-AC-6
///
/// A `/rename` onto a name with no free variant keeps the session's own name:
/// the `UserPromptSubmit` still exits 0, the holder is untouched and the
/// session stays registered as it was.
#[test]
fn tc_022_06_a_rename_with_no_free_variant_keeps_the_cached_name() {
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:collision-rename:", std::process::id());
    let mut relay = relay(&url, &prefix);
    let scratch = Scratch::new("ix-relay-collision-");
    let log = scratch.path().join("calls.log");
    let herdr = fake_herdr_dir(WORKSPACES, TABS, PANES, "", &log, false);
    let config = tempfile::tempdir().unwrap();
    // The session registers as `qsl` at its `SessionStart`; a hook the machine
    // was too slow for records why and is run again.
    let mut started = false;
    for _ in 0..support::OUT_OF_TIME_ATTEMPTS {
        let start = hook(&url, &prefix, &herdr, config.path(), SESSION_START);
        assert_eq!(start.status, 0, "{}", start.stderr);
        if session_name(&mut relay) == Some("qsl".parse().unwrap()) {
            started = true;
            break;
        }
        let records = spool_records(config.path());
        assert!(
            !records.is_empty() && records.iter().all(is_out_of_time),
            "not registered, and not for want of time: {records:?}"
        );
    }
    assert!(
        started,
        "the session never started: this machine is too loaded"
    );

    let holder = registration("taken-name", MACHINE, "holder-sid", "wH:p1");
    relay.register(&holder).unwrap();
    for n in 2..=relay_core::naming::MAX_VARIANT_NUMBER {
        relay
            .register(&registration(
                &format!("taken-name-{n}"),
                MACHINE,
                "x",
                "wX:p1",
            ))
            .unwrap();
    }
    let transcript = config.path().join("transcript.jsonl");
    std::fs::write(
        &transcript,
        format!("{{\"type\":\"custom-title\",\"customTitle\":\"taken-name\",\"sessionId\":\"{MY_SESSION}\"}}\n"),
    )
    .unwrap();
    let prompt = format!(
        r#"{{"hook_event_name":"UserPromptSubmit","session_id":"{MY_SESSION}","cwd":"/work","transcript_path":{:?}}}"#,
        transcript.to_str().unwrap()
    );
    let run = hook(&url, &prefix, &herdr, config.path(), &prompt);
    assert_eq!(run.status, 0, "{}", run.stderr);
    assert_holder_unchanged(&mut relay, &holder);
    assert_eq!(
        relay
            .probe_session(&MY_SESSION.parse().expect("valid id"))
            .unwrap()
            .name,
        Some("qsl".parse().unwrap())
    );
    let mine = relay.agent(&"qsl".parse().unwrap()).unwrap().unwrap();
    assert_eq!(
        mine.session_id.as_ref().map(|v| v.as_str()),
        Some(MY_SESSION)
    );
    // Resolving 1000 taken variants can outlast a `UserPromptSubmit`'s budget on
    // a loaded machine, which records its own `budget_exhausted`: what matters
    // is that the failed rename is not recorded as a failure to find a name.
    let refused = spool_records(config.path())
        .iter()
        .filter(|r| r["error_kind"] == "no_free_name")
        .count();
    assert_eq!(refused, 0, "the prompt kept its name instead of failing");
}

/// How herdr answers the liveness question for the holder of the candidate.
#[derive(Debug, Clone, Copy)]
enum Liveness {
    /// The holder's pane hosts its session.
    Live,
    /// It does not: the registration is stale.
    Gone,
    /// herdr cannot be asked.
    Error,
}

struct ScriptedLiveness(Liveness);

impl PaneLiveness for ScriptedLiveness {
    fn hosts(&self, _pane: &PaneId, _session_id: &SessionId) -> Result<bool, ProcessError> {
        match self.0 {
            Liveness::Live => Ok(true),
            Liveness::Gone => Ok(false),
            Liveness::Error => Err(ProcessError::Reported {
                code: "boom".into(),
                message: "herdr is down".into(),
            }),
        }
    }
}

/// A registry entry the property test starts from.
#[derive(Debug, Clone)]
struct Held {
    /// `None` is the candidate itself, `Some(n)` is `qsl-<n>`.
    variant: Option<u32>,
    same_machine: bool,
    has_session: bool,
    has_pane: bool,
}

fn held() -> impl Strategy<Value = Held> {
    (
        prop_oneof![Just(None), (2u32..=6).prop_map(Some)],
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(|(variant, same_machine, has_session, has_pane)| Held {
            variant,
            same_machine,
            has_session,
            has_pane,
        })
}

fn liveness() -> impl Strategy<Value = Liveness> {
    prop_oneof![
        Just(Liveness::Live),
        Just(Liveness::Gone),
        Just(Liveness::Error)
    ]
}

thread_local! {
    static CASE: Cell<u64> = const { Cell::new(0) };
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Tracing: TC-022-06
    /// ACs: FR-022-AC-6, FR-022-AC-7
    ///
    /// Model: the registry is a map from name to registration; name derivation
    /// answers a name or a typed error; the hook commits (`register`) only on a
    /// name. For any taken names and any liveness outcome, an entry whose
    /// session differs from the caller's is unchanged unless it is a
    /// same-machine holder the caller could show gone (known session and pane,
    /// liveness `Gone`) and the commit was for exactly its name.
    #[test]
    fn tc_022_06_a_hook_never_overwrites_a_registration_of_another_session(
        entries in proptest::collection::vec(held(), 0..8),
        outcome in liveness(),
    ) {
        let Some(url) = test_url() else { return Ok(()); };
        let n = CASE.with(|c| { c.set(c.get() + 1); c.get() });
        let prefix = format!("test:{}:collision-prop-{n}:", std::process::id());
        let mut relay = relay(&url, &prefix);
        let mut model = std::collections::BTreeMap::new();
        for e in &entries {
            let name = match e.variant {
                None => "qsl".to_owned(),
                Some(n) => format!("qsl-{n}"),
            };
            let mut reg = registration(
                &name,
                if e.same_machine { MACHINE } else { "hub" },
                "other-sid",
                "wH:p1",
            );
            if !e.has_session { reg.session_id = None; }
            if !e.has_pane { reg.herdr_pane = None; }
            relay.register(&reg).unwrap();
            model.insert(name, reg);
        }
        let before = model.clone();

        let candidate = "qsl".parse().unwrap();
        let derived = resolve_collision(
            &mut relay,
            &ScriptedLiveness(outcome),
            candidate,
            &MACHINE.parse().unwrap(),
            Some(&MY_SESSION.parse().unwrap()),
        )
        .map(|resolved| resolved.name);
        let committed = match derived {
            Ok(name) => {
                let reg = registration(name.as_str(), MACHINE, MY_SESSION, "wQ:p1");
                relay.register(&reg).unwrap();
                model.insert(name.as_str().to_owned(), reg);
                Some(name)
            }
            Err(CollisionError::NoFreeName { .. }) => None,
            Err(CollisionError::Relay(e)) => panic!("registry read failed: {e}"),
        };

        for (name, was) in &before {
            let now = relay.agent(&name.parse().unwrap()).unwrap().unwrap();
            let overwritten = now.session_id.as_ref().map(|v| v.as_str()) == Some(MY_SESSION);
            // The only takeover is of a holder shown gone.
            let provably_gone = was.machine.as_str() == MACHINE
                && was.session_id.is_some()
                && was.herdr_pane.is_some()
                && matches!(outcome, Liveness::Gone);
            if overwritten {
                prop_assert!(provably_gone, "{name} was overwritten: {was:?}, {outcome:?}");
                prop_assert_eq!(committed.as_ref().map(|n| n.as_str()), Some(name.as_str()));
            } else {
                prop_assert_eq!(&now.session_id, &was.session_id);
                prop_assert_eq!(&now.machine, &was.machine);
                prop_assert_eq!(&now.herdr_pane, &was.herdr_pane);
            }
        }
        // Whatever it committed, the caller holds exactly one name.
        let mine = relay
            .agents()
            .unwrap()
            .registrations
            .into_iter()
            .filter(|r| r.session_id.as_ref().map(|v| v.as_str()) == Some(MY_SESSION))
            .count();
        prop_assert_eq!(mine, usize::from(committed.is_some()));
    }
}
