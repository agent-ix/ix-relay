// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! Scoped startup through the SDK's real credential and collection seams.

use super::*;
use clap::Parser;
use ix_board::{LinearGraphQlSource, retro::sources::*};
use ix_cli_kit::secrets::{
    AppScope, CredentialBackend, DeleteStatus, Presence, SecretError, SecretKey, SecretStore,
    SecretValue,
};
use ix_trace_rs::trace;
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Instant,
};

#[derive(Clone, Copy)]
enum CredentialFixture {
    Saved,
    Denied,
    Missing,
}

struct SavedBackend {
    mode: CredentialFixture,
    reads: Arc<AtomicUsize>,
}

impl CredentialBackend for SavedBackend {
    fn get(&self, scope: &AppScope, key: &SecretKey) -> Result<Option<SecretValue>, SecretError> {
        assert_eq!(scope.as_str(), "ix-board");
        assert_eq!(key.as_str(), "linear-api-key");
        self.reads.fetch_add(1, Ordering::Relaxed);
        match self.mode {
            CredentialFixture::Saved => Ok(Some(SecretValue::new("fixture-saved-credential"))),
            CredentialFixture::Denied => Err(SecretError::Locked),
            CredentialFixture::Missing => Ok(None),
        }
    }
    fn set(&self, _: &AppScope, _: &SecretKey, _: &SecretValue) -> Result<(), SecretError> {
        panic!("credential mutation is forbidden");
    }
    fn delete(&self, _: &AppScope, _: &SecretKey) -> Result<DeleteStatus, SecretError> {
        panic!("credential mutation is forbidden");
    }
    fn status(&self, _: &AppScope, _: &SecretKey) -> Result<Presence, SecretError> {
        panic!("unexpected credential status lookup");
    }
}

struct OtherSources;

impl GithubSource for OtherSources {
    fn pull_requests(&self, _: &str, _: f64) -> Result<Vec<PullRequest>, String> {
        Err("fixture GitHub unavailable".into())
    }
    fn pull_request(&self, _: &str, _: u64) -> Result<PullRequestInfo, String> {
        Err("fixture GitHub unavailable".into())
    }
    fn main_commits(&self, _: &str, _: &str, _: f64) -> Result<Vec<MainCommit>, String> {
        Err("fixture GitHub unavailable".into())
    }
}
impl AgentSource for OtherSources {
    fn report(&self, _: &[String]) -> Result<ix_board::agents::AgentsReport, String> {
        Err("fixture pane unavailable".into())
    }
}
impl GateLogSource for OtherSources {
    fn lines(&self, _: &Path) -> Result<Vec<GateLine>, String> {
        Err("fixture gate log unavailable".into())
    }
}
impl RelaySource for OtherSources {
    fn pending_sent_at_ms(&self, _: &str) -> Result<Vec<i64>, String> {
        Err("fixture relay unavailable".into())
    }
}
impl LocalSource for OtherSources {
    fn prunable_worktrees(&self, _: &Path) -> Result<u64, String> {
        Err("fixture host unavailable".into())
    }
    fn disk_bytes(&self, _: &Path) -> Result<u64, String> {
        Err("fixture host unavailable".into())
    }
    fn processes(&self) -> Result<Vec<ProcessRow>, String> {
        Err("fixture host unavailable".into())
    }
}

struct LoopbackLinear {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    requests: Arc<AtomicUsize>,
    authentication_valid: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl LoopbackLinear {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(AtomicUsize::new(0));
        let authentication_valid = Arc::new(AtomicBool::new(true));
        let worker_stop = Arc::clone(&stop);
        let worker_requests = Arc::clone(&requests);
        let worker_auth = Arc::clone(&authentication_valid);
        let handle = thread::spawn(move || {
            for connection in listener.incoming() {
                if worker_stop.load(Ordering::Acquire) {
                    break;
                }
                let mut connection = connection.unwrap();
                connection
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                connection
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut chunk = [0; 1024];
                let header_end = loop {
                    let count = connection.read(&mut chunk).unwrap();
                    assert!(count > 0, "fixture HTTP request ended before headers");
                    request.extend_from_slice(&chunk[..count]);
                    assert!(
                        request.len() < 1024 * 1024,
                        "fixture HTTP request too large"
                    );
                    if let Some(offset) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                        break offset + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.strip_prefix("content-length:")
                            .map(|value| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap();
                while request.len() - header_end < length {
                    let count = connection.read(&mut chunk).unwrap();
                    assert!(count > 0, "fixture HTTP request ended before body");
                    request.extend_from_slice(&chunk[..count]);
                    assert!(
                        request.len() < 1024 * 1024,
                        "fixture HTTP request too large"
                    );
                }
                worker_requests.fetch_add(1, Ordering::Relaxed);
                if !headers
                    .lines()
                    .any(|line| line.trim() == "authorization: fixture-saved-credential")
                {
                    worker_auth.store(false, Ordering::Release);
                }
                let body: serde_json::Value =
                    serde_json::from_slice(&request[header_end..header_end + length]).unwrap();
                let query = body["query"].as_str().unwrap();
                let field = if query.contains("projects(") {
                    "projects"
                } else if query.contains("comments(") {
                    "comments"
                } else {
                    "issues"
                };
                let response = serde_json::json!({"data": {field: {"nodes": [], "pageInfo": {"hasNextPage": false, "endCursor": null}}}}).to_string();
                write!(connection, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
            }
        });
        Self {
            address,
            stop,
            requests,
            authentication_valid,
            handle: Some(handle),
        }
    }
    fn endpoint(&self) -> String {
        format!("http://{}/graphql", self.address)
    }
}

impl Drop for LoopbackLinear {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = TcpStream::connect(self.address);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn scoped_startup(directory: &Path) -> (crate::Args, crate::monitor::App, retro::RetroConfig) {
    let path = directory.join("retro.json");
    let value = serde_json::json!({
        "team": "SWM", "linear_teams": ["SWM"], "projects": [], "gate_tickets": [], "repos": [],
        "department": {"version": 1, "policy_id": "fixture-policy", "scope": {"teams": [{"id": "SWM", "label": "Fixture"}]}, "freshness_seconds": {"linear": 3600}, "clock_tolerance_seconds": 60}
    });
    std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let args = crate::Args::try_parse_from([
        "dashboard",
        "--team",
        "SWM",
        "--retro-config",
        path.to_str().unwrap(),
    ])
    .unwrap();
    let config = load_config(&path, args.team.as_deref(), &args.project).unwrap();
    let (app, binding) = crate::prepare_app(
        &args,
        |_| panic!("live startup must not read saved data"),
        |_, _| panic!("scoped startup must not probe Herdr"),
    )
    .unwrap();
    assert!(binding.is_none());
    (args, app, config)
}

fn sdk_collection(
    config: &retro::RetroConfig,
    store: &SecretStore<SavedBackend>,
    endpoint: &str,
    cache: &Path,
) -> Result<Delta, String> {
    let source = LinearGraphQlSource::from_credential_store(
        store,
        None,
        || panic!("saved credential or denied store must not select fallback"),
        endpoint,
    )
    .map_err(|error| crate::monitor_ops::source_diagnostic(&error))?;
    let linear = retro::live::LiveLinear::with_source(source, cache)
        .map_err(|error| crate::monitor_ops::source_diagnostic(&error))?;
    let other = OtherSources;
    let sources = retro::Sources {
        linear: &linear,
        github: &other,
        agents: &other,
        gate_log: &other,
        relay: &other,
        local: &other,
    };
    let frame = retro::Frame {
        now: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64(),
        window_secs: 86400,
        end_exclusive: false,
    };
    collect_from_sources(config, frame, &sources)
}

#[trace("FR-059-AC-1", "FR-059-AC-6")]
#[test]
fn saved_provider_without_override_reaches_scoped_startup_worker_and_shared_collection() {
    let directory = tempfile::tempdir().unwrap();
    let (_args, mut app, config) = scoped_startup(directory.path());
    let server = LoopbackLinear::start();
    let endpoint = server.endpoint();
    let reads = Arc::new(AtomicUsize::new(0));
    let store = SecretStore::new(SavedBackend {
        mode: CredentialFixture::Saved,
        reads: Arc::clone(&reads),
    });
    let cache = directory.path().join("tracker.sqlite");
    let worker = DepartmentWorker::spawn_with_read(move || {
        sdk_collection(&config, &store, &endpoint, &cache)
    })
    .unwrap();
    let result = worker.events.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(result.is_ok(), "{result:?}");
    app.apply_department_refresh(result, Instant::now());
    drop(worker);
    assert_eq!(reads.load(Ordering::Relaxed), 1);
    assert!(server.requests.load(Ordering::Relaxed) > 0);
    assert!(server.authentication_valid.load(Ordering::Acquire));
    assert_eq!(app.view, crate::monitor::View::Status);
    let report = &app.department.value.as_ref().unwrap().current;
    assert_eq!(report.scope.teams[0].id, "SWM");
    assert_eq!(report.measurements["wip_active"].status, retro::Status::Ok);
    assert_eq!(
        report.measurements["wip_active"].value,
        Some(retro::department::Value::Scalar(0.0))
    );
    assert_eq!(crate::quit_outcome(&app).0, ix_cli_kit::exit::Outcome::Ok);
}

#[trace("FR-059-AC-6")]
#[test]
fn denied_saved_provider_uses_no_fallback_or_http_and_keeps_nonzero_live_outcome() {
    let directory = tempfile::tempdir().unwrap();
    let (_args, mut app, config) = scoped_startup(directory.path());
    let server = LoopbackLinear::start();
    let reads = Arc::new(AtomicUsize::new(0));
    let store = SecretStore::new(SavedBackend {
        mode: CredentialFixture::Denied,
        reads: Arc::clone(&reads),
    });
    let result = sdk_collection(
        &config,
        &store,
        &server.endpoint(),
        &directory.path().join("denied.sqlite"),
    );
    app.apply_department_refresh(result, Instant::now());
    assert_eq!(reads.load(Ordering::Relaxed), 1);
    assert_eq!(server.requests.load(Ordering::Relaxed), 0);
    assert!(!directory.path().join("denied.sqlite").exists());
    let (outcome, diagnostic) = crate::quit_outcome(&app);
    assert_ne!(outcome, ix_cli_kit::exit::Outcome::Ok);
    assert!(diagnostic.unwrap().contains("credential owner"));
}

#[trace("FR-059-AC-2", "FR-059-AC-6")]
#[test]
fn missing_provider_is_actionable_and_offline_startup_never_resolves_it() {
    let directory = tempfile::tempdir().unwrap();
    let (_args, mut app, _config) = scoped_startup(directory.path());
    let reads = Arc::new(AtomicUsize::new(0));
    let store = SecretStore::new(SavedBackend {
        mode: CredentialFixture::Missing,
        reads: Arc::clone(&reads),
    });
    let failure = LinearGraphQlSource::from_credential_store(
        &store,
        None,
        || None,
        "http://127.0.0.1:9/graphql",
    )
    .err()
    .unwrap();
    assert!(matches!(failure, ix_board::Error::MissingApiKey));
    app.apply_department_refresh(
        Err(crate::monitor_ops::source_diagnostic(&failure)),
        Instant::now(),
    );
    let (outcome, diagnostic) = crate::quit_outcome(&app);
    assert_ne!(outcome, ix_cli_kit::exit::Outcome::Ok);
    assert!(diagnostic.unwrap().contains("ix-board auth login"));
    let path = directory.path().join("saved.json");
    let record = retro::Record {
        schema_version: 2,
        kind: retro::RecordKind::Snapshot,
        taken_at: "2026-11-19T00:00:00Z".into(),
        team: "SWM".into(),
        window: "24h".into(),
        metrics: Default::default(),
        department: Some(crate::monitor_department::tests::fixture().current),
    };
    std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    let args =
        crate::Args::try_parse_from(["dashboard", "--report", path.to_str().unwrap()]).unwrap();
    let (mut offline, binding) =
        crate::prepare_app(&args, read_report, |_, _| panic!("offline binding probe")).unwrap();
    let before = reads.load(Ordering::Relaxed);
    let source = crate::start_tracker(&args, &mut offline, || {
        LinearGraphQlSource::from_credential_store(
            &store,
            None,
            || panic!("offline fallback"),
            "http://127.0.0.1:9/graphql",
        )
        .map_err(io::Error::other)
    });
    assert!(source.is_none());
    assert!(binding.is_none());
    assert_eq!(reads.load(Ordering::Relaxed), before);
    assert_eq!(
        crate::quit_outcome(&offline).0,
        ix_cli_kit::exit::Outcome::Ok
    );
}
