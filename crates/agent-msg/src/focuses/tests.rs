// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! `agent-msg focuses` in process (FR-061): the real command code, a real Redis (isolated per
//! case and skipped when `IX_RELAY_TEST_URL` is unset), the real ix-board `ReadService` with a
//! fake workspace source and a controlled clock, and a fake verified caller.
//!
//! No credential store, Linear, Herdr or shared Redis is touched. Each case reports what the
//! command printed on standard output and standard error and the exit status `main` would
//! return, and the "no writes" cases compare every key of the case's prefix byte for byte.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test fixtures")]

use std::{
    collections::BTreeMap,
    fs,
    sync::{
        Arc, Mutex,
        atomic::{AtomicI64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use clap::Parser;
use ix_board::{
    CatalogEvidence, Clock, Deadline, Error, ReadService, WorkspaceSnapshot, WorkspaceSource,
};
use ix_trace_rs::trace;
use relay_core::{
    AgentName, HerdrPlaceIds, MachineName, Place, Registration, SessionId,
    named_focus::{FocusError, FocusResult, OwnerBinding},
};
use relay_test_support::Bus;
use serde_json::{Value, json};

use super::{
    CommandError, Env, FocusesArgs,
    caller::CallerSource,
    codec::IxBoardCodec,
    file, output, report,
    resolve::{self, ServiceSnapshots, SnapshotSource},
    run,
};

const START: i64 = 1_800_000_000;

// --- fakes ----------------------------------------------------------------------------------

/// A clock that only moves when a test says so, for the cache's wall time and for the
/// bounded read's monotonic deadline.
#[derive(Clone)]
struct ManualClock {
    epoch: Arc<AtomicI64>,
    base: Instant,
    elapsed: Arc<Mutex<Duration>>,
    step: Arc<Mutex<Duration>>,
}

impl ManualClock {
    fn new(epoch: i64) -> Self {
        Self {
            epoch: Arc::new(AtomicI64::new(epoch)),
            base: Instant::now(),
            elapsed: Arc::new(Mutex::new(Duration::ZERO)),
            step: Arc::new(Mutex::new(Duration::ZERO)),
        }
    }

    /// Makes every reading of the monotonic time cost `step`, so work that checks the
    /// deadline as it goes (the resolution scans) sees time pass without any waiting.
    fn each_reading_costs(&self, step: Duration) {
        *self.step.lock().unwrap() = step;
    }

    fn epoch(&self) -> i64 {
        self.epoch.load(Ordering::SeqCst)
    }

    fn pass(&self, seconds: i64) {
        self.epoch.fetch_add(seconds, Ordering::SeqCst);
    }

    fn advance(&self, by: Duration) {
        *self.elapsed.lock().unwrap() += by;
    }

    fn elapsed(&self) -> Duration {
        *self.elapsed.lock().unwrap()
    }
}

impl Clock for ManualClock {
    fn now_epoch_seconds(&self) -> i64 {
        self.epoch()
    }

    fn monotonic_now(&self) -> Instant {
        let step = *self.step.lock().unwrap();
        let mut elapsed = self.elapsed.lock().unwrap();
        let now = self.base + *elapsed;
        *elapsed += step;
        now
    }
}

#[derive(Clone, Copy)]
enum Behavior {
    Succeed,
    /// The credentials are missing.
    NoCredential,
    /// The workspace data is incomplete.
    Incomplete,
    /// Any other failure of the source.
    Broken,
    /// The request never completes: it ends at its timeout.
    NeverComplete,
    /// The read succeeds, after this many seconds of wall time.
    SucceedAfter(i64),
}

/// A workspace source that honors the bounded read, records what it was asked and can fail.
#[derive(Clone)]
struct FakeSource {
    snapshot: WorkspaceSnapshot,
    clock: ManualClock,
    behavior: Arc<Mutex<Behavior>>,
    reads: Arc<AtomicUsize>,
    timeouts: Arc<Mutex<Vec<Duration>>>,
}

impl FakeSource {
    fn behave(&self, behavior: Behavior) {
        *self.behavior.lock().unwrap() = behavior;
    }

    fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

impl WorkspaceSource for FakeSource {
    fn fetch_workspace(&self) -> Result<WorkspaceSnapshot, Error> {
        Err(Error::BoundUnsupported)
    }

    fn supports_bounded_read(&self) -> bool {
        true
    }

    fn catalog_evidence(&self) -> CatalogEvidence {
        CatalogEvidence::default()
    }

    fn fetch_workspace_bounded(&self, deadline: &Deadline<'_>) -> Result<WorkspaceSnapshot, Error> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        let behavior = *self.behavior.lock().unwrap();
        match behavior {
            Behavior::Succeed => Ok(self.snapshot.clone()),
            Behavior::SucceedAfter(seconds) => {
                self.clock.pass(seconds);
                Ok(self.snapshot.clone())
            }
            Behavior::NoCredential => Err(Error::MissingApiKey),
            Behavior::Incomplete => Err(Error::MissingWorkspaceData),
            Behavior::Broken => Err(Error::BoundUnsupported),
            Behavior::NeverComplete => {
                let timeout = deadline.request_timeout(Duration::from_secs(30))?;
                self.timeouts.lock().unwrap().push(timeout);
                self.clock.advance(timeout);
                Err(Error::ReadTimeout {
                    deadline: deadline.limit(),
                })
            }
        }
    }
}

/// The verified caller: a fixed session, or a refusal.
struct FakeCaller {
    session: &'static str,
    pid: u32,
}

impl CallerSource for FakeCaller {
    fn verify(
        &self,
        _lead: Option<&str>,
        session: Option<&SessionId>,
        now_ms: i64,
    ) -> FocusResult<OwnerBinding> {
        if session.is_some_and(|wanted| wanted.as_str() != self.session) {
            return Err(FocusError::invalid(
                "session",
                "the pane hosts a different native session than --session names",
            ));
        }
        Ok(OwnerBinding::new(
            &machine(),
            &self.session.parse().unwrap(),
            Some(self.pid),
            &format!("terra:QSL:{}", self.session),
            now_ms,
        ))
    }
}

fn machine() -> MachineName {
    "terra".parse().unwrap()
}

/// A registration whose placement is `Verified`: the session is live.
fn live(session_id: &str) -> Registration {
    let machine = machine();
    let session: SessionId = session_id.parse().unwrap();
    Registration {
        name: AgentName::for_session(&machine, &session),
        machine,
        kind: "codex".into(),
        session_id: Some(session.clone()),
        cwd: None,
        herdr_pane: Some("w1:p1".parse().unwrap()),
        place: Some(Place {
            workspace: "QSL".into(),
            tab: session_id.into(),
            herdr_ids: Some(HerdrPlaceIds {
                workspace: "w1".parse().unwrap(),
                tab: "w1:t1".parse().unwrap(),
                pane: "w1:p1".parse().unwrap(),
                session_id: session,
                verified_at_ms: 1,
            }),
        }),
        last_seen_ms: 1,
    }
}

// --- the workspace ---------------------------------------------------------------------------

fn project(id: &str, name: &str, team: &str, milestones: Value, initiatives: Value) -> Value {
    json!({
        "id": id, "identifier": null, "name": name, "updated_at": null, "description": "",
        "content": null, "status": "Started", "priority": 0, "sort_order": 0.0,
        "target_date": null, "team_keys": [team], "milestones": milestones,
        "initiatives": initiatives,
    })
}

fn issue(id: &str, identifier: &str, team: &str, project_id: Option<&str>) -> Value {
    json!({
        "id": id, "identifier": identifier, "title": "t", "description": null, "state": "Todo",
        "state_name": "Todo", "state_type": "unstarted", "parent_id": null, "team_key": team,
        "assignee_id": null, "project_id": project_id, "project_milestone_id": null,
        "priority": 0.0, "sort_order": 0.0, "blocked_by": [], "created_at": null,
        "updated_at": null, "attachment_urls": [], "attachments_complete": true,
    })
}

fn workspace() -> WorkspaceSnapshot {
    let beta = json!([{"id": "m-beta", "name": "Beta", "description": null,
                       "sort_order": 0.0, "target_date": null}]);
    let completion = json!([{"id": "i-1", "identifier": "I-1", "name": "Completion"}]);
    serde_json::from_value(json!({
        "projects": [
            project("p-alpha", "Alpha", "SWM", beta, completion),
            project("p-gamma", "Gamma", "SWM", json!([]), json!([])),
            project("p-twin-1", "Twin", "SWM", json!([]), json!([])),
            project("p-twin-2", "Twin", "SWM", json!([]), json!([])),
        ],
        "issues": [
            issue("t-1", "SWM-1", "SWM", Some("p-alpha")),
            issue("t-2", "SWM-2", "SWM", Some("p-gamma")),
            issue("t-other", "OTH-1", "OTH", None),
        ],
        "fetched_at": 0,
    }))
    .unwrap()
}

// --- the harness -----------------------------------------------------------------------------

struct Outcome {
    stdout: String,
    stderr: String,
    status: u8,
}

impl Outcome {
    fn out(&self) -> Value {
        assert_eq!(self.status, 0, "stderr: {}", self.stderr);
        assert!(self.stderr.is_empty(), "{}", self.stderr);
        assert!(self.stdout.ends_with('\n') && self.stdout.matches('\n').count() == 1);
        serde_json::from_str(&self.stdout).unwrap()
    }

    /// The typed error object; exit status 1 and nothing on standard output.
    fn error(&self) -> Value {
        assert_eq!(self.status, 1, "stdout: {}", self.stdout);
        assert!(self.stdout.is_empty(), "{}", self.stdout);
        assert!(self.stderr.ends_with('\n') && self.stderr.matches('\n').count() == 1);
        serde_json::from_str(&self.stderr).unwrap()
    }

    fn code(&self) -> String {
        self.error()["code"].as_str().unwrap().to_owned()
    }
}

#[derive(Parser)]
struct Wrapper {
    #[command(flatten)]
    focuses: FocusesArgs,
}

struct Harness {
    bus: Bus,
    dir: tempfile::TempDir,
    clock: ManualClock,
    source: FakeSource,
    service: ServiceSnapshots<FakeSource, ManualClock>,
    caller: FakeCaller,
}

impl Harness {
    fn new(case: &str) -> Option<Self> {
        Self::with_workspace(case, workspace())
    }

    fn with_workspace(case: &str, snapshot: WorkspaceSnapshot) -> Option<Self> {
        let bus = Bus::connect(case)?;
        let dir = tempfile::tempdir().unwrap();
        let clock = ManualClock::new(START);
        let source = FakeSource {
            snapshot,
            clock: clock.clone(),
            behavior: Arc::new(Mutex::new(Behavior::Succeed)),
            reads: Arc::new(AtomicUsize::new(0)),
            timeouts: Arc::new(Mutex::new(Vec::new())),
        };
        let service = ServiceSnapshots(
            ReadService::with_clock(
                source.clone(),
                &dir.path().join("cache.sqlite3"),
                clock.clone(),
            )
            .unwrap(),
        );
        Some(Self {
            bus,
            dir,
            clock,
            source,
            service,
            caller: FakeCaller {
                session: "sess-a",
                pid: 11,
            },
        })
    }

    fn as_session(&mut self, session: &'static str) {
        self.caller = FakeCaller { session, pid: 12 };
    }

    fn file(&self, name: &str, definition: &Value) -> String {
        let path = self.dir.path().join(name);
        fs::write(&path, serde_json::to_vec(definition).unwrap()).unwrap();
        path.to_str().unwrap().to_owned()
    }

    fn run(&mut self, args: &[&str]) -> Outcome {
        let clock = self.clock.clone();
        execute(
            &mut self.bus,
            &self.caller,
            &mut self.service,
            &move || clock.epoch() * 1_000,
            args,
        )
    }

    /// Runs `args` writing the result to `out`, and gives the command's own result.
    fn run_to<W: std::io::Write>(
        &mut self,
        args: &[&str],
        out: &mut W,
    ) -> Result<(), CommandError> {
        let clock = self.clock.clone();
        let parsed = parse(args).expect("arguments parse");
        let machine = machine();
        let mut env = Env {
            relay: &mut self.bus.relay,
            codec: &IxBoardCodec,
            snapshots: &mut self.service,
            caller: &self.caller,
            machine: &machine,
            clock: &move || clock.epoch() * 1_000,
        };
        run(&parsed, &mut env, out)
    }

    /// Every key of the case, byte for byte.
    fn keys(&mut self) -> BTreeMap<String, Vec<u8>> {
        dump(&mut self.bus)
    }

    fn focus_keys(&mut self) -> BTreeMap<String, Vec<u8>> {
        let own = self.bus.key("focus:");
        self.keys()
            .into_iter()
            .filter(|(key, _)| key.starts_with(&own))
            .collect()
    }
}

fn dump(bus: &mut Bus) -> BTreeMap<String, Vec<u8>> {
    bus.keys()
        .into_iter()
        .map(|key| {
            let bytes: Vec<u8> = redis::cmd("DUMP").arg(&key).query(&mut bus.raw).unwrap();
            (key, bytes)
        })
        .collect()
}

/// The arguments of `agent-msg focuses ...`, or the usage error.
fn parse(args: &[&str]) -> Result<FocusesArgs, clap::Error> {
    let mut argv = vec!["focuses"];
    argv.extend_from_slice(args);
    Wrapper::try_parse_from(&argv).map(|parsed| parsed.focuses)
}

/// What `main` does after connecting: run the command, report a failure as a typed error.
fn execute(
    bus: &mut Bus,
    caller: &dyn CallerSource,
    snapshots: &mut dyn SnapshotSource,
    clock: &dyn Fn() -> i64,
    args: &[&str],
) -> Outcome {
    let parsed = match parse(args) {
        Ok(parsed) => parsed,
        Err(error) => {
            return Outcome {
                stdout: String::new(),
                stderr: error.to_string(),
                status: 2,
            };
        }
    };
    let machine = machine();
    let mut env = Env {
        relay: &mut bus.relay,
        codec: &IxBoardCodec,
        snapshots,
        caller,
        machine: &machine,
        clock,
    };
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let status = match run(&parsed, &mut env, &mut stdout) {
        Ok(()) => 0,
        Err(error) => report(&error, parsed.json, &mut stderr),
    };
    Outcome {
        stdout: String::from_utf8(stdout).unwrap(),
        stderr: String::from_utf8(stderr).unwrap(),
        status,
    }
}

fn definition(name: &str) -> Value {
    json!({
        "team": "SWM",
        "name": name,
        "description": "the plan",
        "include": {
            "initiatives": ["Completion"],
            "projects": ["Gamma"],
            "milestones": [{"ref": "Beta", "project": "Alpha"}],
            "tickets": ["SWM-2"],
        },
        "exclude": {"tickets": ["SWM-1"]},
    })
}

fn created(h: &mut Harness, name: &str) -> String {
    let file = h.file(&format!("{name}.json"), &definition(name));
    let out = h.run(&["--json", "create", "--file", &file]);
    out.out()["focus_id"].as_str().unwrap().to_owned()
}

// --- AC-1, AC-21: create and show -----------------------------------------------------------

#[test]
#[trace("FR-061-AC-1", "FR-061-AC-21")]
fn create_resolves_names_to_canonical_ids_and_show_reproduces_the_file_shape() {
    let Some(mut h) = Harness::new("cli-create") else {
        return;
    };
    let file = h.file("plan.json", &definition("Plan"));
    let made = h.run(&["--json", "create", "--file", &file]).out();
    let id = made["focus_id"].as_str().unwrap().to_owned();
    assert!(id.starts_with("foc-"));
    assert_eq!(made["revision"], 1);
    assert_eq!(made["owner_id"], "terra/sess-a");
    assert_eq!(made["owner_epoch"], 1);

    let shown = h.run(&["--json", "show", &id]).out();
    let by_name = h.run(&["--json", "show", "Plan"]).out();
    assert_eq!(shown, by_name, "a name resolves to the same stored ID");
    let def = &shown["definition"];
    assert_eq!(def["team"], "SWM");
    assert_eq!(def["description"], "the plan");
    assert_eq!(def["include"]["initiatives"][0]["ref"], "i-1");
    assert_eq!(
        def["include"]["initiatives"][0]["label"], "Completion",
        "no label in the file: the reference text the caller wrote"
    );
    assert_eq!(def["include"]["projects"][0]["ref"], "p-gamma");
    assert_eq!(def["include"]["milestones"][0]["ref"], "m-beta");
    assert_eq!(def["include"]["milestones"][0]["project"], "p-alpha");
    assert_eq!(def["include"]["tickets"][0]["ref"], "t-2");
    assert_eq!(def["exclude"]["tickets"][0]["ref"], "t-1");
    assert_eq!(shown["empty_selection"], false);
    assert_eq!(
        shown["owner"]["state"], "absent",
        "no live registration of the owner"
    );

    // The printed definition is itself a valid update file, and updating with it changes nothing.
    let saved = h.file("saved.json", def);
    let before = h.focus_keys();
    let same = h
        .run(&[
            "--json",
            "update",
            &id,
            "--file",
            &saved,
            "--expected-revision",
            "1",
        ])
        .out();
    assert_eq!(
        same,
        json!({"focus_id": id, "revision": 1, "changed": false})
    );
    assert_eq!(h.focus_keys(), before);
}

#[test]
#[trace("FR-061-AC-1")]
fn the_plain_text_forms_print_the_same_facts() {
    let Some(mut h) = Harness::new("cli-text") else {
        return;
    };
    let file = h.file("plan.json", &definition("Plan"));
    let made = h.run(&["create", "--file", &file]);
    assert_eq!(made.status, 0, "{}", made.stderr);
    assert!(made.stdout.starts_with("created foc-"), "{}", made.stdout);
    let shown = h.run(&["show", "Plan"]);
    assert!(
        shown.stdout.contains("revision 1 of 1 (active)"),
        "{}",
        shown.stdout
    );
    assert!(
        shown
            .stdout
            .contains("owner: terra/sess-a epoch 1 (absent)"),
        "{}",
        shown.stdout
    );
    assert!(
        shown
            .stdout
            .contains("include milestone m-beta in p-alpha (Beta)")
    );
    let listed = h.run(&["list"]);
    assert!(
        listed
            .stdout
            .contains("\"Plan\" r1 active owner terra/sess-a epoch 1")
    );
}

// --- AC-2: unresolved references --------------------------------------------------------------

#[test]
#[trace("FR-061-AC-2")]
fn references_that_do_not_resolve_are_refused_naming_the_reference_and_write_nothing() {
    let Some(mut h) = Harness::new("cli-unresolved") else {
        return;
    };
    let cases = [
        ("an ambiguous name", json!({"projects": ["Twin"]}), "Twin"),
        (
            "a zero-match reference",
            json!({"projects": ["Nowhere"]}),
            "Nowhere",
        ),
        (
            "a milestone of another project",
            json!({"milestones": [{"ref": "Beta", "project": "Gamma"}]}),
            "Beta",
        ),
        (
            "a ticket of another team",
            json!({"tickets": ["OTH-1"]}),
            "OTH-1",
        ),
    ];
    for (what, include, reference) in cases {
        let file = h.file(
            "bad.json",
            &json!({"team": "SWM", "name": "Bad", "include": include}),
        );
        let out = h.run(&["--json", "create", "--file", &file]);
        let error = out.error();
        assert_eq!(error["code"], "unresolved_reference", "{what}: {error}");
        assert_eq!(error["reference"], reference, "{what}");
    }
    assert!(h.keys().is_empty(), "no record changed");
}

// --- AC-5: invalid files and the empty selection ---------------------------------------------

#[test]
#[trace("FR-061-AC-5")]
fn a_file_with_a_focus_reference_two_teams_or_an_unknown_member_is_invalid_and_reads_nothing() {
    let Some(mut h) = Harness::new("cli-invalid-file") else {
        return;
    };
    let bad = [
        json!({"team": "SWM", "name": "A", "include": {}, "focus": "foc-0"}),
        json!({"team": ["SWM", "OTH"], "name": "A", "include": {}}),
        json!({"team": "SWM", "name": "A", "include": {"initiatives": [], "everything": []}}),
        json!({"team": "SWM", "name": "A", "include": {"projects": [{"ref": "Alpha", "x": 1}]}}),
        json!({"team": "SWM", "name": "A"}),
    ];
    for (n, value) in bad.iter().enumerate() {
        let file = h.file(&format!("bad-{n}.json"), value);
        let out = h.run(&["--json", "create", "--file", &file]);
        assert_eq!(out.code(), "invalid_argument", "{value}");
    }
    let unreadable = h.run(&["--json", "create", "--file", "/nonexistent/definition.json"]);
    assert_eq!(unreadable.code(), "invalid_argument");
    assert_eq!(
        h.source.reads(),
        0,
        "nothing was read before the file was accepted"
    );
    assert!(h.keys().is_empty());
}

#[test]
#[trace("FR-061-AC-5")]
fn an_empty_inclusion_list_creates_and_is_never_described_as_the_whole_team() {
    let Some(mut h) = Harness::new("cli-empty") else {
        return;
    };
    let file = h.file(
        "empty.json",
        &json!({"team": "SWM", "name": "Nothing", "include": {}}),
    );
    let made = h.run(&["--json", "create", "--file", &file]).out();
    let id = made["focus_id"].as_str().unwrap();
    let shown = h.run(&["--json", "show", id]).out();
    assert_eq!(shown["empty_selection"], true);
    assert_eq!(shown["definition"]["include"]["projects"], json!([]));
    let text = h.run(&["show", id]);
    assert!(text.stdout.contains("selects nothing"), "{}", text.stdout);
    let lowered = text.stdout.to_lowercase();
    assert!(
        !lowered.contains("whole") && !lowered.contains("all team"),
        "{lowered}"
    );
}

// --- AC-19: the snapshot's age and state ------------------------------------------------------

#[test]
#[trace("FR-061-AC-19")]
fn a_snapshot_four_minutes_old_resolves_without_a_refresh() {
    let Some(mut h) = Harness::new("cli-fresh") else {
        return;
    };
    // The first read fills the shared cache.
    let first = h.file("a.json", &definition("A"));
    h.run(&["--json", "create", "--file", &first]).out();
    assert_eq!(h.source.reads(), 1);

    h.clock.pass(240);
    h.source.behave(Behavior::Broken);
    let second = h.file("b.json", &definition("B"));
    let made = h.run(&["--json", "create", "--file", &second]).out();
    assert_eq!(made["revision"], 1);
    assert_eq!(
        h.source.reads(),
        1,
        "no refresh: the cached snapshot was used"
    );

    let id = made["focus_id"].as_str().unwrap().to_owned();
    let update = h.file("c.json", &definition("B2"));
    let updated = h
        .run(&[
            "--json",
            "update",
            &id,
            "--file",
            &update,
            "--expected-revision",
            "1",
        ])
        .out();
    assert_eq!(updated["revision"], 2);
    assert_eq!(h.source.reads(), 1);
}

#[test]
#[trace("FR-061-AC-19")]
fn a_missing_stale_partial_or_failed_snapshot_is_source_unavailable_and_writes_nothing() {
    let Some(mut h) = Harness::new("cli-source") else {
        return;
    };
    let file = h.file("plan.json", &definition("Plan"));
    let cases = [
        (Behavior::NoCredential, "missing"),
        (Behavior::Incomplete, "partial"),
        (Behavior::Broken, "failed"),
    ];
    for (behavior, reason) in cases {
        h.source.behave(behavior);
        let out = h.run(&["--json", "create", "--file", &file]);
        let error = out.error();
        assert_eq!(error["code"], "source_unavailable", "{reason}");
        assert_eq!(error["reason"], reason);
        assert_eq!(
            h.keys(),
            BTreeMap::new(),
            "{reason}: the store is untouched"
        );
    }

    // Older than five minutes in the cache: it is refreshed, and a failing refresh never falls
    // back to the stale entry.
    h.source.behave(Behavior::Succeed);
    h.run(&["--json", "create", "--file", &file]).out();
    let before = h.keys();
    h.clock.pass(301);
    h.source.behave(Behavior::Broken);
    let again = h.file("again.json", &definition("Again"));
    let out = h.run(&["--json", "create", "--file", &again]);
    assert_eq!(out.error()["reason"], "failed");
    assert_eq!(h.keys(), before);

    // With the source healthy again the stale entry is refreshed under the same call.
    let reads = h.source.reads();
    h.source.behave(Behavior::Succeed);
    h.run(&["--json", "create", "--file", &again]).out();
    assert_eq!(h.source.reads(), reads + 1, "exactly one refresh");
}

#[test]
#[trace("FR-061-AC-19", "FR-061-AC-20")]
fn expensive_name_resolution_on_a_cache_hit_times_out_under_the_one_deadline() {
    let Some(mut h) = Harness::with_workspace("cli-expensive-resolution", crowded_workspace())
    else {
        return;
    };
    // Warm the shared cache with a light selection; resolution then needs no source read.
    let warm = h.file("warm.json", &light("Warm"));
    h.run(&["--json", "create", "--file", &warm]).out();
    assert_eq!(h.source.reads(), 1);

    // Same cache entry, same policy, same clock for every run below: each reading of the
    // monotonic time costs 50 ms. The cache lookup and decode take a handful of readings
    // whatever the selection is, so a light selection finishes well inside 20 seconds.
    // The costly selection (12 ticket names, each scanning ~110 issues, more than a thousand
    // checks) can only exceed 20 seconds in the resolution scans themselves. The unbounded
    // `canonicalize_named_selection` of the earlier integration never reads this clock and
    // would let it resolve, so this fails if the integration regresses to it.
    h.clock.each_reading_costs(Duration::from_millis(50));
    let fast = h.file("fast.json", &light("Fast"));
    h.run(&["--json", "create", "--file", &fast, "--read-timeout", "20"])
        .out();
    let before = h.keys();
    let slow = h.file("slow.json", &costly("Slow"));
    let out = h.run(&["--json", "create", "--file", &slow, "--read-timeout", "20"]);
    let error = out.error();
    assert_eq!(error["code"], "source_unavailable");
    assert_eq!(error["reason"], "timeout");
    assert_eq!(
        h.source.reads(),
        1,
        "a cache hit: no source read, no second read"
    );
    assert_eq!(
        h.keys(),
        before,
        "no definition, name, index or pin was written"
    );

    // Control: the same costly input, cache and per-reading cost resolve under a generous
    // deadline, so the refusal above was the resolution running out of budget.
    let made = h
        .run(&["--json", "create", "--file", &slow, "--read-timeout", "600"])
        .out();
    assert_eq!(made["revision"], 1);
    assert_eq!(h.source.reads(), 1, "still the one warm read");
}

/// The standard workspace plus 100 filler tickets and 12 `WANT-n` tickets after them, so a
/// ticket lookup scans every issue before it matches.
fn crowded_workspace() -> WorkspaceSnapshot {
    let mut value = serde_json::to_value(workspace()).unwrap();
    let issues = value["issues"].as_array_mut().unwrap();
    for n in 0..100 {
        issues.push(issue(
            &format!("t-fill-{n}"),
            &format!("FILL-{n}"),
            "SWM",
            Some("p-alpha"),
        ));
    }
    for n in 0..12 {
        issues.push(issue(
            &format!("t-want-{n}"),
            &format!("WANT-{n}"),
            "SWM",
            Some("p-gamma"),
        ));
    }
    serde_json::from_value(value).unwrap()
}

/// One project reference: little resolution work.
fn light(name: &str) -> Value {
    json!({"team": "SWM", "name": name, "include": {"projects": ["Gamma"]}})
}

/// Twelve ticket references by identifier: a full issue scan each.
fn costly(name: &str) -> Value {
    let tickets: Vec<String> = (0..12).map(|n| format!("WANT-{n}")).collect();
    json!({"team": "SWM", "name": name, "include": {"tickets": tickets}})
}

// --- AC-20: the deadline -----------------------------------------------------------------------

#[test]
#[trace("FR-061-AC-20")]
fn a_source_that_never_completes_times_out_within_the_deadline_and_changes_nothing() {
    for (flag, deadline) in [(None, 120_u64), (Some("1"), 1), (Some("600"), 600)] {
        let Some(mut h) = Harness::new("cli-deadline") else {
            return;
        };
        h.source.behave(Behavior::NeverComplete);
        let file = h.file("plan.json", &definition("Plan"));
        let mut args = vec!["--json", "create", "--file", file.as_str()];
        if let Some(seconds) = flag {
            args.extend(["--read-timeout", seconds]);
        }
        let out = h.run(&args);
        let error = out.error();
        assert_eq!(error["code"], "source_unavailable", "{deadline}");
        assert_eq!(error["reason"], "timeout", "{deadline}");
        // The time the read consumed never exceeds the deadline: each request is bounded by
        // the time that remains, and the single request is the only one made.
        assert!(
            h.clock.elapsed() <= Duration::from_secs(deadline),
            "{deadline}"
        );
        let timeouts = h.source.timeouts.lock().unwrap().clone();
        assert_eq!(
            timeouts,
            [Duration::from_secs(deadline.min(30))],
            "{deadline}"
        );
        assert_eq!(h.source.reads(), 1, "no retry and no background read");
        assert_eq!(
            h.keys(),
            BTreeMap::new(),
            "{deadline}: the store is untouched"
        );
        assert_eq!(
            h.source.reads(),
            1,
            "still one read after the command returned"
        );
    }
}

#[test]
#[trace("FR-061-AC-20")]
fn the_read_timeout_must_be_between_one_and_six_hundred_seconds() {
    let Some(mut h) = Harness::new("cli-timeout-range") else {
        return;
    };
    let file = h.file("plan.json", &definition("Plan"));
    for bad in ["0", "601", "-1", "ten"] {
        let out = h.run(&["create", "--file", &file, "--read-timeout", bad]);
        assert_eq!(out.status, 2, "{bad}: {}", out.stderr);
        assert!(out.stdout.is_empty());
    }
    assert_eq!(h.source.reads(), 0);
}

// --- AC-3, AC-4, AC-6: updates and ownership through the command ---------------------------

#[test]
#[trace("FR-061-AC-3", "FR-061-AC-4")]
fn update_writes_the_next_revision_conflicts_on_a_stale_expectation_and_renames() {
    let Some(mut h) = Harness::new("cli-update") else {
        return;
    };
    let id = created(&mut h, "Plan");
    let renamed = h.file("renamed.json", &definition("Renamed"));
    let ok = h
        .run(&[
            "--json",
            "update",
            &id,
            "--file",
            &renamed,
            "--expected-revision",
            "1",
        ])
        .out();
    assert_eq!(ok, json!({"focus_id": id, "revision": 2, "changed": true}));

    let stale = h.run(&[
        "--json",
        "update",
        &id,
        "--file",
        &renamed,
        "--expected-revision",
        "1",
    ]);
    let error = stale.error();
    assert_eq!(error["code"], "revision_conflict");
    assert_eq!(error["current_revision"], 2);
    assert_eq!(error["focus_id"], id.as_str());

    // The old name is free and the new one resolves to the same ID.
    assert_eq!(
        h.run(&["--json", "show", "Renamed"]).out()["focus_id"],
        id.as_str()
    );
    assert_eq!(h.run(&["--json", "show", "Plan"]).code(), "not_found");
    created(&mut h, "Plan");
    let taken = h.run(&[
        "--json",
        "update",
        "Plan",
        "--file",
        &renamed,
        "--expected-revision",
        "1",
    ]);
    assert_eq!(taken.code(), "name_taken");

    assert_eq!(
        h.run(&["--json", "show", &id, "--revision", "1"]).out()["revision"],
        1
    );
    assert_eq!(
        h.run(&["--json", "show", &id, "--revision", "9"]).code(),
        "not_found"
    );
}

#[test]
#[trace("FR-061-AC-6")]
fn a_different_session_cannot_write_and_the_error_names_the_owner() {
    let Some(mut h) = Harness::new("cli-not-owner") else {
        return;
    };
    let id = created(&mut h, "Plan");
    let before = h.focus_keys();
    h.as_session("sess-b");
    let file = h.file("next.json", &definition("Next"));
    let update = h.run(&[
        "--json",
        "update",
        &id,
        "--file",
        &file,
        "--expected-revision",
        "1",
    ]);
    let error = update.error();
    assert_eq!(error["code"], "not_owner");
    assert_eq!(error["current_owner_id"], "terra/sess-a");
    assert_eq!(error["current_owner_epoch"], 1);
    assert_eq!(
        h.run(&["--json", "archive", &id, "--expected-revision", "1"])
            .code(),
        "not_owner"
    );
    assert_eq!(h.focus_keys(), before);

    // Reads need no binding.
    assert_eq!(
        h.run(&["--json", "show", &id]).out()["focus_id"],
        id.as_str()
    );
    // `--session` naming a different native session than the pane hosts is refused.
    h.as_session("sess-a");
    let wrong = h.run(&[
        "--json",
        "--session",
        "other",
        "archive",
        &id,
        "--expected-revision",
        "1",
    ]);
    assert_eq!(wrong.code(), "invalid_argument");
    assert_eq!(h.focus_keys(), before);
}

// --- AC-7, AC-8, AC-10, AC-11: claim, transfer and archive ---------------------------------

#[test]
#[trace("FR-061-AC-7", "FR-061-AC-8")]
fn claim_needs_a_gone_owner_and_force_needs_a_reason() {
    let Some(mut h) = Harness::new("cli-claim") else {
        return;
    };
    let id = created(&mut h, "Plan");
    h.bus.relay.register(&live("sess-a")).unwrap();
    h.as_session("sess-b");

    let refused = h.run(&["--json", "claim", &id, "--expected-owner-epoch", "1"]);
    assert_eq!(refused.code(), "not_owner", "the owner is live");

    // `--force` and `--reason` come together.
    for args in [
        [
            "--json",
            "claim",
            &id,
            "--expected-owner-epoch",
            "1",
            "--force",
        ]
        .as_slice(),
        [
            "--json",
            "claim",
            &id,
            "--expected-owner-epoch",
            "1",
            "--reason",
            "why",
        ]
        .as_slice(),
    ] {
        assert_eq!(h.run(args).status, 2);
    }
    let stale = h.run(&[
        "--json",
        "claim",
        &id,
        "--expected-owner-epoch",
        "5",
        "--force",
        "--reason",
        "x",
    ]);
    assert_eq!(stale.code(), "owner_epoch_conflict");

    let forced = h
        .run(&[
            "--json",
            "claim",
            &id,
            "--expected-owner-epoch",
            "1",
            "--force",
            "--reason",
            "the owner is wedged",
        ])
        .out();
    assert_eq!(forced["owner_id"], "terra/sess-b");
    assert_eq!(forced["owner_epoch"], 2);
    assert_eq!(forced["change"]["kind"], "forced_claim");
    assert_eq!(forced["change"]["reason"], "the owner is wedged");
    assert_eq!(forced["change"]["previous_owner_id"], "terra/sess-a");

    // Now sess-b owns it; with sess-b absent, sess-a's registration is irrelevant to a claim by
    // a third session without force: the owner is gone.
    h.as_session("sess-c");
    let claimed = h
        .run(&["--json", "claim", &id, "--expected-owner-epoch", "2"])
        .out();
    assert_eq!(claimed["owner_epoch"], 3);
    assert_eq!(claimed["change"]["kind"], "claim");
}

#[test]
#[trace("FR-061-AC-10")]
fn transfer_goes_only_to_exactly_one_verified_live_session() {
    let Some(mut h) = Harness::new("cli-transfer") else {
        return;
    };
    let id = created(&mut h, "Plan");
    let before = h.focus_keys();

    // Zero sessions; a partial address; a registered session without verified placement.
    for to in ["terra:QSL:nobody", "QSL:sess-b", "nonsense"] {
        let out = h.run(&[
            "--json",
            "transfer",
            &id,
            "--to",
            to,
            "--expected-owner-epoch",
            "1",
        ]);
        assert_eq!(out.code(), "invalid_argument", "{to}");
    }
    let mut unplaced = live("sess-b");
    unplaced.place = None;
    h.bus.relay.register(&unplaced).unwrap();
    let out = h.run(&[
        "--json",
        "transfer",
        &id,
        "--to",
        "terra:QSL:sess-b",
        "--expected-owner-epoch",
        "1",
    ]);
    assert_eq!(out.code(), "invalid_argument");
    assert_eq!(h.focus_keys(), before, "nothing changed");

    h.bus.relay.register(&live("sess-b")).unwrap();
    let stale = h.run(&[
        "--json",
        "transfer",
        &id,
        "--to",
        "terra:QSL:sess-b",
        "--expected-owner-epoch",
        "4",
    ]);
    assert_eq!(stale.code(), "owner_epoch_conflict");
    assert_eq!(h.focus_keys(), before);

    let moved = h
        .run(&[
            "--json",
            "transfer",
            &id,
            "--to",
            "terra:QSL:sess-b",
            "--expected-owner-epoch",
            "1",
        ])
        .out();
    assert_eq!(moved["owner_id"], "terra/sess-b");
    assert_eq!(moved["owner_epoch"], 2);
    assert_eq!(moved["change"]["kind"], "transfer");

    // The old owner can no longer write; the new one can.
    let file = h.file("next.json", &definition("Next"));
    let old = h.run(&[
        "--json",
        "update",
        &id,
        "--file",
        &file,
        "--expected-revision",
        "1",
    ]);
    assert_eq!(old.code(), "not_owner");
    h.as_session("sess-b");
    let new = h.run(&[
        "--json",
        "update",
        &id,
        "--file",
        &file,
        "--expected-revision",
        "1",
    ]);
    assert_eq!(new.out()["revision"], 2);
}

#[test]
#[trace("FR-061-AC-10")]
fn a_transfer_to_an_address_held_by_several_live_sessions_changes_nothing() {
    let Some(mut h) = Harness::new("cli-transfer-ambiguous") else {
        return;
    };
    let id = created(&mut h, "Plan");
    // Two sessions, both verified live, share the workspace and tab label `shared`.
    for session in ["sess-b", "sess-c"] {
        let mut registration = live(session);
        registration.place.as_mut().unwrap().tab = "shared".into();
        h.bus.relay.register(&registration).unwrap();
    }
    let registered = h.bus.relay.agents().unwrap();
    assert_eq!(registered.registrations.len(), 2);
    assert!(
        registered
            .registrations
            .iter()
            .all(|r| r.full_address().is_some())
    );
    let before = h.keys();
    let out = h.run(&[
        "--json",
        "transfer",
        &id,
        "--to",
        "terra:QSL:shared",
        "--expected-owner-epoch",
        "1",
    ]);
    assert_eq!(out.code(), "invalid_argument");
    assert_eq!(h.keys(), before, "no ownership change and no other write");
    let shown = h.run(&["--json", "show", &id]).out();
    assert_eq!(shown["owner"]["owner_id"], "terra/sess-a");
    assert_eq!(shown["owner"]["epoch"], 1);
}

#[test]
#[trace("FR-061-AC-11")]
fn an_archived_definition_is_readable_listed_on_request_and_releases_its_name() {
    let Some(mut h) = Harness::new("cli-archive") else {
        return;
    };
    let id = created(&mut h, "Plan");
    let archived = h
        .run(&["--json", "archive", &id, "--expected-revision", "1"])
        .out();
    assert_eq!(archived["state"], "archived");
    assert_eq!(archived["latest_revision"], 1);

    let file = h.file("next.json", &definition("Plan"));
    let update = h.run(&[
        "--json",
        "update",
        &id,
        "--file",
        &file,
        "--expected-revision",
        "1",
    ]);
    assert_eq!(update.code(), "archived");
    assert_eq!(
        h.run(&["--json", "archive", &id, "--expected-revision", "1"])
            .code(),
        "archived"
    );

    assert_eq!(h.run(&["--json", "show", &id]).out()["state"], "archived");
    let hidden = h.run(&["--json", "list"]).out();
    assert_eq!(hidden["definitions"], json!([]));
    let shown = h.run(&["--json", "list", "--archived"]).out();
    assert_eq!(shown["definitions"][0]["focus_id"], id.as_str());
    assert_eq!(shown["definitions"][0]["state"], "archived");

    let again = created(&mut h, "Plan");
    assert_ne!(again, id, "the name is reusable");
    let listed = h.run(&["--json", "list", "--team", "swm"]).out();
    assert_eq!(listed["definitions"].as_array().unwrap().len(), 1);
    assert_eq!(
        h.run(&["--json", "list", "--team", "OTH"]).out()["definitions"],
        json!([])
    );
}

// --- AC-18: unavailable definitions are listed by ID ------------------------------------------

#[test]
#[trace("FR-061-AC-18")]
fn an_unreadable_definition_is_listed_with_its_reason_and_refuses_writes() {
    let Some(mut h) = Harness::new("cli-unavailable") else {
        return;
    };
    let good = created(&mut h, "Good");
    let bad = created(&mut h, "Bad");
    let key = h.bus.key(&format!("focus:def:{bad}"));
    redis::cmd("HSET")
        .arg(&key)
        .arg("meta")
        .arg(b"\xff not json".as_slice())
        .exec(&mut h.bus.raw)
        .unwrap();

    let listed = h.run(&["--json", "list"]).out();
    let rows = listed["definitions"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter()
            .any(|r| r["focus_id"] == good.as_str() && r["name"] == "Good")
    );
    let unavailable = rows.iter().find(|r| r["focus_id"] == bad.as_str()).unwrap();
    assert_eq!(unavailable["available"], false);
    assert_eq!(
        unavailable["code"], "unavailable_definition",
        "the row carries the stable code of the refusal it stands for"
    );
    assert_eq!(unavailable["reason"], "undecodable");
    let text = h.run(&["list"]);
    assert!(
        text.stdout
            .contains(&format!("{bad} unavailable_definition: undecodable"))
    );

    let before = h.focus_keys();
    let file = h.file("next.json", &definition("Next"));
    let write = h.run(&[
        "--json",
        "update",
        &bad,
        "--file",
        &file,
        "--expected-revision",
        "1",
    ]);
    let error = write.error();
    assert_eq!(error["code"], "unavailable_definition");
    assert_eq!(error["reason"], "undecodable");
    assert_eq!(h.focus_keys(), before, "never overwritten");
    assert_eq!(
        h.run(&["--json", "show", &bad]).code(),
        "unavailable_definition"
    );
}

// --- AC-21: the output contract and what the commands leave alone ----------------------------

#[test]
#[trace("FR-061-AC-21")]
fn the_commands_touch_no_inbox_no_registration_and_print_one_compact_line() {
    let Some(mut h) = Harness::new("cli-contract") else {
        return;
    };
    h.bus.relay.register(&live("sess-a")).unwrap();
    h.bus.relay.register(&live("sess-b")).unwrap();
    let outside = |h: &mut Harness| -> BTreeMap<String, Vec<u8>> {
        let own = h.bus.key("focus:");
        h.keys()
            .into_iter()
            .filter(|(k, _)| !k.starts_with(&own))
            .collect()
    };
    let before = outside(&mut h);

    let id = created(&mut h, "Plan");
    let file = h.file("next.json", &definition("Next"));
    let outputs = [
        h.run(&[
            "--json",
            "update",
            &id,
            "--file",
            &file,
            "--expected-revision",
            "1",
        ]),
        h.run(&["--json", "show", &id]),
        h.run(&["--json", "list"]),
        h.run(&[
            "--json",
            "transfer",
            &id,
            "--to",
            "terra:QSL:sess-b",
            "--expected-owner-epoch",
            "1",
        ]),
    ];
    for out in &outputs {
        out.out();
        assert!(
            !out.stdout.contains("\": ") && !out.stdout.contains("\", "),
            "compact: {}",
            out.stdout
        );
    }
    assert_eq!(
        outside(&mut h),
        before,
        "no inbox, registration or other key changed"
    );
}

#[test]
#[trace("FR-061-AC-21")]
fn every_failure_is_a_typed_object_with_a_code_from_the_stable_list() {
    use relay_core::named_focus::{SourceReason, UnavailableReason};
    let focus = "foc-00000000000000000000000000000001".parse().unwrap();
    let errors = [
        FocusError::RevisionConflict {
            focus: focus_id(),
            latest: 3,
        },
        FocusError::OwnerEpochConflict {
            focus: focus_id(),
            owner_id: "m/s".into(),
            epoch: 2,
        },
        FocusError::NotOwner {
            focus: focus_id(),
            owner_id: "m/s".into(),
            epoch: 2,
            detail: "d".into(),
        },
        FocusError::NameTaken {
            team: "T".into(),
            name: "n".into(),
            holder: None,
        },
        FocusError::NameMoved {
            team: "T".into(),
            name: "n".into(),
            focus: focus_id(),
            holder: None,
        },
        FocusError::Archived { focus: focus_id() },
        FocusError::limit("name bytes", 128),
        FocusError::UnresolvedReference {
            reference: "r".into(),
            message: "m".into(),
        },
        FocusError::SourceUnavailable {
            reason: SourceReason::Timeout,
            origin: "ix-board workspace snapshot".into(),
            message: "m".into(),
        },
        FocusError::UnavailableDefinition {
            focus,
            reason: UnavailableReason::Undecodable,
        },
        FocusError::NotFound { focus: "x".into() },
        FocusError::invalid("file", "bad"),
    ];
    let stable = [
        "revision_conflict",
        "owner_epoch_conflict",
        "not_owner",
        "name_taken",
        "archived",
        "limit",
        "unresolved_reference",
        "source_unavailable",
        "unavailable_definition",
        "not_found",
        "already_active",
        "not_active",
        "invalid_argument",
    ];
    for error in &errors {
        let mut line = Vec::new();
        output::failure(&mut line, true, error);
        let text = String::from_utf8(line).unwrap();
        assert!(
            text.ends_with('\n') && text.matches('\n').count() == 1,
            "{text}"
        );
        let value: Value = serde_json::from_str(&text).unwrap();
        assert!(stable.contains(&value["code"].as_str().unwrap()), "{value}");
        assert!(value["message"].as_str().is_some_and(|m| !m.is_empty()));
    }
    let mut text = Vec::new();
    output::failure(&mut text, false, &FocusError::limit("name bytes", 128));
    assert_eq!(
        String::from_utf8(text).unwrap(),
        "agent-msg: limit: limit reached: name bytes (at most 128)\n"
    );
}

fn focus_id() -> relay_core::named_focus::FocusId {
    "foc-00000000000000000000000000000002".parse().unwrap()
}

// --- AC-5: the file parser on its own ---------------------------------------------------------

#[test]
#[trace("FR-061-AC-5")]
fn the_file_parser_is_strict_and_bounded() {
    let parsed = file::parse_definition(
        br#"{"team":"SWM","name":"n","include":{"projects":["a",{"ref":"b","label":"B"}]}}"#,
    )
    .unwrap();
    assert_eq!(parsed.include.projects.len(), 2);
    assert!(parsed.exclude.projects.is_empty() && parsed.description.is_empty());
    // The caller's label is kept as parsed; the resolution joins it to its member.
    assert_eq!(parsed.include.projects[0].label, None);
    assert_eq!(parsed.include.projects[1].label.as_deref(), Some("B"));
    assert!(
        file::parse_definition(
            br#"{"team":"SWM","name":"n","include":{"projects":[{"ref":"b","label":7}]}}"#
        )
        .is_err(),
        "a label that is not a string is refused, not dropped"
    );

    for bad in [
        &br#"[]"#[..],
        br#"{"team":"SWM","name":"n","include":{},"exclude":{"milestones":[{"project":"p"}]}}"#,
        br#"{"team":"SWM","name":"n","include":{"projects":"a"}}"#,
        br#"{"team":"SWM","name":"n","include":{"projects":[1]}}"#,
        br#"{"team":"SWM","name":"n","include":{"tickets":[{"ref":"a","project":"p"}]}}"#,
        b"not json",
    ] {
        let error = file::parse_definition(bad).unwrap_err();
        assert_eq!(
            error.code().as_str(),
            "invalid_argument",
            "{}",
            String::from_utf8_lossy(bad)
        );
    }

    let many: Vec<String> = (0..129).map(|n| format!("p{n}")).collect();
    let too_many = json!({"team": "SWM", "name": "n", "include": {"projects": many}});
    let error = file::parse_definition(&serde_json::to_vec(&too_many).unwrap()).unwrap_err();
    assert_eq!(error.code().as_str(), "limit");
    let exactly = json!({"team": "SWM", "name": "n", "include": {"projects": &many[..128]}});
    assert!(file::parse_definition(&serde_json::to_vec(&exactly).unwrap()).is_ok());
    let long = "x".repeat(129);
    let long_ref = json!({"team": "SWM", "name": "n", "include": {"projects": [long]}});
    let error = file::parse_definition(&serde_json::to_vec(&long_ref).unwrap()).unwrap_err();
    assert_eq!(error.code().as_str(), "limit");
}

// --- AC-1: labels follow the SDK's association, not a position ------------------------------------

/// The members of one list of a shown definition, as `(ref, label)`.
fn listed(side: &Value, kind: &str) -> Vec<(String, String)> {
    side[kind]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            (
                m["ref"].as_str().unwrap().to_owned(),
                m["label"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn pair(reference: &str, label: &str) -> (String, String) {
    (reference.to_owned(), label.to_owned())
}

#[test]
#[trace("FR-061-AC-1", "FR-061-AC-21")]
fn each_member_keeps_the_label_of_its_own_input_through_aliases_repeats_and_both_lists() {
    let Some(mut h) = Harness::new("cli-labels") else {
        return;
    };
    let file = h.file(
        "labels.json",
        &json!({
            "team": "SWM",
            "name": "Labelled",
            "include": {
                // One initiative three ways: the first explicit label wins, the bare alias
                // and the later explicit label do not replace it.
                "initiatives": [
                    {"ref": "I-1", "label": "first alias label"},
                    "i-1",
                    {"ref": "Completion", "label": "second explicit"},
                ],
                // One project twice: an explicit label beats an earlier reference text.
                "projects": ["p-gamma", {"ref": "Gamma", "label": "Gamma label"}],
                // Supplied in the reverse of the canonical order (t-1 before t-2).
                "tickets": [{"ref": "SWM-2", "label": "two"}, {"ref": "SWM-1"}],
            },
            // The same ticket on the other side keeps its own occurrence's text.
            "exclude": {"tickets": ["SWM-2"]},
        }),
    );
    let made = h.run(&["--json", "create", "--file", &file]).out();
    let id = made["focus_id"].as_str().unwrap();
    let shown = h.run(&["--json", "show", id]).out();
    let (include, exclude) = (
        &shown["definition"]["include"],
        &shown["definition"]["exclude"],
    );
    assert_eq!(
        listed(include, "initiatives"),
        [pair("i-1", "first alias label")]
    );
    assert_eq!(
        listed(include, "projects"),
        [pair("p-gamma", "Gamma label")]
    );
    assert_eq!(
        listed(include, "tickets"),
        [pair("t-1", "SWM-1"), pair("t-2", "two")],
        "canonical order, each label from its own occurrence: the identifier the caller typed \
         when no label was written"
    );
    assert_eq!(listed(exclude, "tickets"), [pair("t-2", "SWM-2")]);

    // The same definition read back from the store, not only from the command's output.
    let again = h.run(&["--json", "show", "--revision", "1", id]).out();
    assert_eq!(again["definition"], shown["definition"]);
}

#[test]
#[trace("FR-061-AC-1", "FR-061-AC-12")]
fn a_label_is_cut_to_64_bytes_on_a_character_boundary_and_kept_on_one_line() {
    let Some(mut h) = Harness::new("cli-label-bounds") else {
        return;
    };
    let long = "é".repeat(40);
    let file = h.file(
        "long.json",
        &json!({
            "team": "SWM",
            "name": "Long labels",
            "include": {"projects": [
                {"ref": "Gamma", "label": long},
                {"ref": "Alpha", "label": "two\nlines\u{1b}[0m"},
            ]},
        }),
    );
    let made = h.run(&["--json", "create", "--file", &file]).out();
    let id = made["focus_id"].as_str().unwrap();
    let shown = h.run(&["--json", "show", id]).out();
    let projects = listed(&shown["definition"]["include"], "projects");
    let gamma = projects.iter().find(|(r, _)| r == "p-gamma").unwrap();
    assert_eq!(
        gamma.1.len(),
        64,
        "forty two-byte characters cut to 64 bytes"
    );
    assert!(gamma.1.chars().all(|c| c == 'é'));
    let alpha = projects.iter().find(|(r, _)| r == "p-alpha").unwrap();
    assert!(
        alpha.1.starts_with("two") && alpha.1.contains("lines"),
        "{}",
        alpha.1
    );
    assert!(!alpha.1.chars().any(char::is_control), "{:?}", alpha.1);
}

#[test]
#[trace("FR-061-AC-2", "FR-061-AC-20")]
fn a_refused_or_timed_out_resolution_publishes_neither_the_selection_nor_the_labels() {
    let Some(mut h) = Harness::new("cli-labels-refused") else {
        return;
    };
    let file = h.file(
        "bad.json",
        &json!({
            "team": "SWM",
            "name": "Refused",
            "include": {"projects": [
                {"ref": "Gamma", "label": "fine"},
                {"ref": "Nowhere", "label": "never stored"},
            ]},
        }),
    );
    let refused = h.run(&["--json", "create", "--file", &file]);
    assert_eq!(refused.error()["code"], "unresolved_reference");
    assert!(
        h.keys().is_empty(),
        "nothing of the selection or its labels was stored"
    );

    let good = h.file(
        "good.json",
        &json!({"team": "SWM", "name": "Timed out",
                "include": {"projects": [{"ref": "Gamma", "label": "not stored"}]}}),
    );
    // The refused attempt cached a valid snapshot, which a fresh cache would legitimately
    // reuse. Let it age past the five-minute policy so the reader must go to the source.
    h.clock.pass(301);
    let reads = h.source.reads();
    h.source.behave(Behavior::NeverComplete);
    let timed_out = h.run(&["--json", "create", "--file", &good]);
    assert_eq!(timed_out.error()["reason"], "timeout");
    assert_eq!(
        h.source.reads(),
        reads + 1,
        "the bounded source read was attempted"
    );
    assert!(h.keys().is_empty());
}

// --- the shared read's answer is checked, never trusted blindly ----------------------------------

/// A change made to the real service's answer before the command sees it.
type Tamper = fn(&mut ix_board::ResolvedNamedSelection);

/// A source that returns what the real service returns after `tamper` changed it.
struct Tampered<'a> {
    inner: &'a mut ServiceSnapshots<FakeSource, ManualClock>,
    tamper: Tamper,
}

impl SnapshotSource for Tampered<'_> {
    fn resolve_named(
        &mut self,
        policy: &ix_board::ReadPolicy,
        input: &ix_board::NamedSelectionInput,
    ) -> Result<ix_board::ResolvedNamedSelection, Error> {
        let mut resolved = self.inner.resolve_named(policy, input)?;
        (self.tamper)(&mut resolved);
        Ok(resolved)
    }
}

#[test]
#[trace("FR-061-AC-19")]
fn an_answer_that_does_not_match_the_input_is_a_failed_read_and_yields_no_content() {
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new(START);
    let source = FakeSource {
        snapshot: workspace(),
        clock: clock.clone(),
        behavior: Arc::new(Mutex::new(Behavior::Succeed)),
        reads: Arc::new(AtomicUsize::new(0)),
        timeouts: Arc::new(Mutex::new(Vec::new())),
    };
    let mut service = ServiceSnapshots(
        ReadService::with_clock(source, &dir.path().join("cache.sqlite3"), clock).unwrap(),
    );
    let definition = file::parse_definition(
        br#"{"team":"SWM","name":"n","include":{"projects":["Gamma",{"ref":"Alpha","label":"A"}]}}"#,
    )
    .unwrap();

    // Control: untouched, the labels join by association.
    let content = resolve::resolve(
        &definition,
        &mut Tampered {
            inner: &mut service,
            tamper: |_| {},
        },
        Duration::from_secs(30),
    )
    .unwrap();
    let labels: Vec<(&str, &str)> = content
        .include
        .iter()
        .map(|m| (m.id.as_str(), m.label.as_str()))
        .collect();
    assert_eq!(labels, [("p-alpha", "A"), ("p-gamma", "Gamma")]);

    let tampers: [(&str, Tamper); 3] = [
        ("an association is missing", |r| {
            r.associations.pop();
        }),
        ("an association carries another input", |r| {
            let other = r.associations[1].input.clone();
            r.associations[0].input = other;
        }),
        ("a canonical member has no occurrence", |r| {
            r.associations[0].member = ix_board::NamedMember::Project {
                id: "p-elsewhere".into(),
            };
        }),
    ];
    for (what, tamper) in tampers {
        let error = resolve::resolve(
            &definition,
            &mut Tampered {
                inner: &mut service,
                tamper,
            },
            Duration::from_secs(30),
        )
        .unwrap_err();
        let FocusError::SourceUnavailable { reason, .. } = &error else {
            panic!("{what}: {error:?}");
        };
        assert_eq!(reason.as_str(), "failed", "{what}");
    }
}

// --- AC-22: the existing commands keep their meanings -----------------------------------------

#[test]
#[trace("FR-061-AC-22")]
fn the_singular_focus_and_work_commands_are_unchanged_and_focuses_is_separate() {
    use crate::{Cli, Cmd};
    let parse = |args: &[&str]| Cli::try_parse_from(args).map(|cli| cli.cmd);
    assert!(matches!(
        parse(&["agent-msg", "focus", "phase 2"]),
        Ok(Cmd::Focus { focus: Some(text), clear: false }) if text == "phase 2"
    ));
    assert!(matches!(
        parse(&["agent-msg", "focus", "--clear"]),
        Ok(Cmd::Focus {
            focus: None,
            clear: true
        })
    ));
    assert!(matches!(
        parse(&["agent-msg", "work", "focus", "summary"]),
        Ok(Cmd::Work(_))
    ));
    assert!(matches!(
        parse(&["agent-msg", "work", "show"]),
        Ok(Cmd::Work(_))
    ));
    assert!(matches!(
        parse(&["agent-msg", "focuses", "list"]),
        Ok(Cmd::Focuses(_))
    ));
    // The plural family does not accept the singular's free text, nor the reverse.
    assert!(parse(&["agent-msg", "focuses", "phase 2"]).is_err());
    assert!(parse(&["agent-msg", "focus", "list", "--json"]).is_err());
}

// --- AC-19: every SDK failure has a reason, chosen on purpose ---------------------------------

#[test]
#[trace("FR-061-AC-19")]
fn each_sdk_failure_is_placed_under_one_of_the_contract_reasons() {
    use ix_board::Error as E;
    let io = || std::io::Error::other("fixture");
    let table: Vec<(E, &str)> = vec![
        (
            E::ReadTimeout {
                deadline: Duration::from_secs(1),
            },
            "timeout",
        ),
        (E::MissingApiKey, "missing"),
        (E::EmptyCredentialInput, "missing"),
        (E::CredentialInput(io()), "missing"),
        (E::MissingWorkspaceData, "partial"),
        (E::PaginationLimit { pages: 3 }, "partial"),
        (
            E::NestedPaginationLimit {
                resource: "r".into(),
            },
            "partial",
        ),
        (E::MissingCursor, "partial"),
        (
            E::InvalidCursor {
                resource: "r".into(),
            },
            "partial",
        ),
        (
            E::RelationsLimit {
                issue: "SWM-1".into(),
            },
            "partial",
        ),
        (
            E::MissingDependency {
                issue: "SWM-1".into(),
            },
            "partial",
        ),
        (
            E::DependencyCycle {
                issues: "SWM-1".into(),
            },
            "partial",
        ),
        (
            E::ParentCycle {
                identifier: "SWM-1".into(),
            },
            "partial",
        ),
        (
            E::MissingProjectMilestone {
                issue: "SWM-1".into(),
                milestone: "m".into(),
                project: "p".into(),
            },
            "partial",
        ),
        (
            E::FocusPlanIncomplete {
                issue: "SWM-1".into(),
                field: "state",
            },
            "partial",
        ),
        (
            E::FocusHygieneIncomplete {
                issue: "SWM-1".into(),
            },
            "partial",
        ),
        (
            E::GraphQl {
                messages: "m".into(),
            },
            "failed",
        ),
        (
            E::LinearRateLimited {
                retry_after: Duration::from_secs(1),
            },
            "failed",
        ),
        (E::BoundUnsupported, "failed"),
        (E::InvalidReadPolicy { reason: "r" }, "failed"),
        (E::CacheTimestamp, "failed"),
        (E::CacheDirectoryUnavailable, "failed"),
    ];
    for (error, reason) in table {
        let FocusError::SourceUnavailable {
            reason: got,
            origin,
            ..
        } = resolve::source_error(&error)
        else {
            panic!("{error}: not a source_unavailable");
        };
        assert_eq!(got.as_str(), reason, "{error}");
        assert_eq!(origin, "ix-board workspace snapshot");
    }
    // A fault of the caller's own reference is never blamed on the source.
    let scope = E::ScopeSelection {
        kind: "project",
        identifier: "Twin".into(),
        matches: 2,
    };
    let FocusError::UnresolvedReference { reference, .. } = resolve::unresolved(scope) else {
        panic!("a reference that matches twice is unresolved");
    };
    assert_eq!(reference, "Twin");
    assert_eq!(
        resolve::unresolved(E::InvalidTeam).code().as_str(),
        "invalid_argument"
    );
}

// --- AC-1, AC-4: stored text and the printed forms ----------------------------------------------

#[test]
#[trace("FR-061-AC-1", "FR-061-AC-21")]
fn a_name_is_stored_as_one_safe_line_and_a_description_keeps_its_lines_but_prints_indented() {
    let Some(mut h) = Harness::new("cli-text-rules") else {
        return;
    };
    let mut value = definition("unused");
    value["name"] = json!("Plan\u{1b}[31m\nSecond line");
    value["description"] = json!("first\nowner: terra/forged epoch 9 (live)\u{1b}[2J");
    let file = h.file("odd.json", &value);
    let made = h.run(&["--json", "create", "--file", &file]).out();
    let name = made["name"].as_str().unwrap();
    assert!(
        !name.contains('\n') && !name.chars().any(char::is_control),
        "a name is one line of plain text: {name:?}"
    );
    assert!(name.starts_with("Plan"), "{name:?}");
    let id = made["focus_id"].as_str().unwrap();
    assert_eq!(
        h.run(&["--json", "show", name]).out()["focus_id"],
        id,
        "the stored name is the name that resolves"
    );

    let shown = h.run(&["--json", "show", id]).out();
    let description = shown["definition"]["description"].as_str().unwrap();
    assert!(
        description.contains('\n'),
        "the description keeps its line break"
    );
    assert!(
        !description.chars().any(|c| c.is_control() && c != '\n'),
        "{description:?}"
    );

    let text = h.run(&["show", id]);
    let forged = text
        .stdout
        .lines()
        .filter(|line| line.starts_with("owner:"))
        .count();
    assert_eq!(
        forged, 1,
        "only the real owner line starts with `owner:`: {}",
        text.stdout
    );
    assert!(!text.stdout.contains('\u{1b}'));
}

// --- AC-4: the name on create ---------------------------------------------------------------

#[test]
#[trace("FR-061-AC-4")]
fn creating_a_second_definition_with_a_name_in_use_is_name_taken_in_any_letter_case() {
    let Some(mut h) = Harness::new("cli-create-name-taken") else {
        return;
    };
    let first = created(&mut h, "Plan");
    let before = h.focus_keys();
    for name in ["Plan", "PLAN", "plan"] {
        let file = h.file("again.json", &definition(name));
        let out = h.run(&["--json", "create", "--file", &file]);
        let error = out.error();
        assert_eq!(error["code"], "name_taken", "{name}");
        assert_eq!(error["name"], name, "{name}");
        assert_eq!(
            error["focus_id"],
            first.as_str(),
            "the holder is named: {name}"
        );
    }
    assert_eq!(h.focus_keys(), before, "nothing was written");
}

// --- AC-7: two claimants of the same owner -----------------------------------------------------

#[test]
#[trace("FR-061-AC-8")]
fn the_second_of_two_claims_made_at_the_same_epoch_loses_and_is_told_who_won() {
    let Some(mut h) = Harness::new("cli-claim-vs-claim") else {
        return;
    };
    let id = created(&mut h, "Plan");
    h.as_session("sess-b");
    let won = h
        .run(&["--json", "claim", &id, "--expected-owner-epoch", "1"])
        .out();
    assert_eq!(won["owner_id"], "terra/sess-b");
    assert_eq!(won["owner_epoch"], 2);
    let after_winner = h.focus_keys();

    h.as_session("sess-c");
    let lost = h.run(&["--json", "claim", &id, "--expected-owner-epoch", "1"]);
    let error = lost.error();
    assert_eq!(error["code"], "owner_epoch_conflict");
    assert_eq!(
        error["current_owner_id"], "terra/sess-b",
        "the winner is named"
    );
    assert_eq!(error["current_owner_epoch"], 2);
    assert_eq!(h.focus_keys(), after_winner, "the loser changed nothing");
}

// --- AC-16: a skipped registry record --------------------------------------------------------

#[test]
#[trace("FR-061-AC-16", "FR-061-AC-10")]
fn a_registry_read_that_skipped_a_record_shows_unknown_and_refuses_a_transfer_target() {
    let Some(mut h) = Harness::new("cli-skipped-registration") else {
        return;
    };
    let id = created(&mut h, "Plan");
    h.bus.relay.register(&live("sess-a")).unwrap();
    h.bus.relay.register(&live("sess-b")).unwrap();
    let state = |h: &mut Harness| {
        h.run(&["--json", "show", &id]).out()["owner"]["state"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(
        state(&mut h),
        "live",
        "a complete read of one verified registration"
    );

    h.bus.plant("terra-garbage", b"\xff not a registration");
    assert_eq!(h.bus.relay.agents().unwrap().skipped.len(), 1);
    assert_eq!(
        state(&mut h),
        "unknown",
        "one verified registration is not enough when a record was skipped"
    );
    let before = h.focus_keys();
    let refused = h.run(&[
        "--json",
        "transfer",
        &id,
        "--to",
        "terra:QSL:sess-b",
        "--expected-owner-epoch",
        "1",
    ]);
    assert_eq!(refused.code(), "invalid_argument");
    assert_eq!(h.focus_keys(), before);

    redis::cmd("HDEL")
        .arg(h.bus.key("agents"))
        .arg("terra-garbage")
        .exec(&mut h.bus.raw)
        .unwrap();
    let moved = h
        .run(&[
            "--json",
            "transfer",
            &id,
            "--to",
            "terra:QSL:sess-b",
            "--expected-owner-epoch",
            "1",
        ])
        .out();
    assert_eq!(moved["owner_id"], "terra/sess-b");
}

// --- AC-21: the stamps of a write are taken after the read -------------------------------------

#[test]
#[trace("FR-061-AC-21")]
fn every_stamp_of_a_write_is_taken_after_the_snapshot_read_and_not_before_it() {
    let Some(mut h) = Harness::new("cli-stamps") else {
        return;
    };
    let started = START * 1_000;
    let after_read = (START + 300) * 1_000;

    // An unverified caller is refused before the read starts.
    let file = h.file("plan.json", &definition("Plan"));
    let wrong = h.run(&["--json", "--session", "other", "create", "--file", &file]);
    assert_eq!(wrong.code(), "invalid_argument");
    assert_eq!(h.source.reads(), 0);

    h.source.behave(Behavior::SucceedAfter(300));
    let made = h.run(&["--json", "create", "--file", &file]).out();
    let id = made["focus_id"].as_str().unwrap().to_owned();
    let shown = h.run(&["--json", "show", &id]).out();
    assert_eq!(shown["written_at_ms"], after_read, "the revision");
    assert_eq!(
        shown["owner"]["binding"]["verified_at"], after_read,
        "the verification"
    );
    assert_eq!(
        shown["owner"]["change"]["at"], after_read,
        "the owner change"
    );
    assert_ne!(shown["written_at_ms"], started);

    let next = h.file("next.json", &definition("Next"));
    h.source.behave(Behavior::SucceedAfter(200));
    h.run(&[
        "--json",
        "update",
        &id,
        "--file",
        &next,
        "--expected-revision",
        "1",
    ])
    .out();
    let updated = h.run(&["--json", "show", &id]).out();
    assert_eq!(updated["written_at_ms"], (START + 500) * 1_000);
    assert_eq!(
        updated["owner"]["binding"]["verified_at"],
        (START + 500) * 1_000,
        "the owner's binding is verified again after the read"
    );
}

// --- AC-21: a result that cannot be printed --------------------------------------------------

/// A standard output that fails every write with `kind`.
struct Failing(std::io::ErrorKind);

impl std::io::Write for Failing {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::from(self.0))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Err(std::io::Error::from(self.0))
    }
}

#[test]
#[trace("FR-061-AC-21")]
fn a_closed_standard_output_after_a_committed_write_is_not_an_invalid_argument() {
    let Some(mut h) = Harness::new("cli-closed-stdout") else {
        return;
    };
    let file = h.file("plan.json", &definition("Plan"));

    // A closed pipe (`| head`): the write is committed, the reader chose to stop reading.
    let error = h
        .run_to(
            &["--json", "create", "--file", &file],
            &mut Failing(std::io::ErrorKind::BrokenPipe),
        )
        .unwrap_err();
    assert!(
        matches!(&error, CommandError::Output(e) if e.kind() == std::io::ErrorKind::BrokenPipe),
        "{error:?}"
    );
    let mut stderr = Vec::new();
    assert_eq!(report(&error, true, &mut stderr), 0);
    assert!(stderr.is_empty(), "{}", String::from_utf8_lossy(&stderr));
    let listed = h.run(&["--json", "list"]).out();
    assert_eq!(
        listed["definitions"].as_array().unwrap().len(),
        1,
        "the write stands"
    );

    // Any other failure to print is reported as that, with no code of the contract.
    let other = h
        .run_to(
            &["--json", "list"],
            &mut Failing(std::io::ErrorKind::StorageFull),
        )
        .unwrap_err();
    let mut stderr = Vec::new();
    assert_eq!(report(&other, true, &mut stderr), 1);
    let text = String::from_utf8(stderr).unwrap();
    assert!(
        text.contains("could not be written to standard output"),
        "{text}"
    );
    assert!(
        !text.contains("invalid_argument") && !text.contains("\"code\""),
        "{text}"
    );
}
