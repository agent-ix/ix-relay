//! Process-level checks of what `relay-watcher` does at start-up, run against
//! the built binary.
#![expect(
    clippy::expect_used,
    reason = "test code: a failed expect is the failure signal"
)]

use std::{
    path::Path,
    process::{Command, Output},
};

/// Runs the watcher with a clean environment whose config lives in `dir`.
fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_relay-watcher"))
        .env_clear()
        .env("IX_RELAY_CONFIG", dir.join("config.toml"))
        .args(args)
        .output()
        .expect("spawn relay-watcher")
}

/// NFR-003: a configuration error is one of the watcher's only exits, and it
/// happens at start-up: a URL that cannot parse exits 1 instead of retrying
/// forever.
#[test]
fn a_malformed_redis_url_exits_1_at_startup() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(dir.path(), &["--url", "definitely not a url"]);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("invalid redis url"), "{stderr}");
}

/// `--once` is a manual check and must not touch the shared state file, even
/// though its one poll fails here (nothing listens on port 1).
#[test]
fn once_never_writes_the_state_file() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(dir.path(), &["--once", "--url", "redis://127.0.0.1:1"]);
    assert_eq!(out.status.code(), Some(1), "one failed poll exits 1");
    assert!(
        !dir.path().join("state").exists(),
        "no state directory, so no watcher.json"
    );
}

/// Tracing: TC-028-01
/// ACs: FR-028-AC-1
///
/// The running binary publishes its heartbeat with no message sent, keeps it
/// fresh, carries the expiry the sweep interval gives (30 s at least), and the
/// key is what a killed watcher leaves to expire. Redis lane.
#[test]
fn a_running_watcher_publishes_a_fresh_heartbeat_with_an_expiry() {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
        return;
    };
    let prefix = format!("test:{}:hb-process:", std::process::id());
    let dir = tempfile::tempdir().unwrap();
    let child = std::process::Command::new(env!("CARGO_BIN_EXE_relay-watcher"))
        .env_clear()
        .env("IX_RELAY_CONFIG", dir.path().join("config.toml"))
        .env("IX_RELAY_MACHINE", "hbhost")
        .args(["--url", &url, "--prefix", &prefix, "--sweep-secs", "1"])
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn relay-watcher");
    // The watcher never exits by itself: whatever fails below must still kill it.
    let mut child = KillOnDrop(child);
    let mut relay = relay_core::Relay::connect(&url, std::time::Duration::from_secs(2))
        .unwrap()
        .with_prefix(prefix.clone());
    let read = |relay: &mut relay_core::Relay| match relay
        .heartbeat(&"hbhost".parse().unwrap())
        .unwrap()
    {
        Some(relay_core::health::HeartbeatRead::Live(hb)) => Some(*hb),
        _ => None,
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let first = loop {
        if let Some(hb) = read(&mut relay) {
            break hb;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no heartbeat appeared"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    assert_eq!(
        (first.pid, first.machine.as_str()),
        (child.0.id(), "hbhost")
    );
    // Idle, nothing sent: the sweep still refreshes the record.
    let refreshed = loop {
        match read(&mut relay) {
            Some(hb) if hb.last_tick_ms > first.last_tick_ms => break hb,
            _ => {}
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the heartbeat went stale"
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    assert_eq!(refreshed.started_at_ms, first.started_at_ms);
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    let mut raw = redis::Client::open(url.as_str())
        .unwrap()
        .get_connection()
        .unwrap();
    let ttl_ms: i64 = redis::cmd("PTTL")
        .arg(format!("{prefix}health:watcher:hbhost"))
        .query(&mut raw)
        .unwrap();
    assert!(
        (1..=30_000).contains(&ttl_ms),
        "a killed watcher leaves a key that expires within 30 s, ttl {ttl_ms}"
    );
    redis::cmd("DEL")
        .arg(format!("{prefix}health:watcher:hbhost"))
        .exec(&mut raw)
        .unwrap();
}

/// Kills and reaps the watcher when dropped, so an assertion that fails while it
/// runs does not leave it behind.
struct KillOnDrop(std::process::Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A watcher started with `env` against a Redis nothing listens on (it retries
/// forever, so it stays up), its stderr lines arriving on a channel.
struct Running {
    _child: KillOnDrop,
    lines: std::sync::mpsc::Receiver<String>,
    /// Every line taken off the channel so far, in order, for failure messages
    /// and for checks over the whole log.
    seen: std::cell::RefCell<Vec<String>>,
}

fn start_watcher(dir: &Path, env: &[(&str, String)]) -> Running {
    use std::io::BufRead;
    let mut command = Command::new(env!("CARGO_BIN_EXE_relay-watcher"));
    command
        .env_clear()
        .env("IX_RELAY_CONFIG", dir.join("config.toml"))
        .args(["--url", "redis://127.0.0.1:1"])
        .stderr(std::process::Stdio::piped());
    for (name, value) in env {
        command.env(name, value);
    }
    let mut child = command.spawn().expect("spawn relay-watcher");
    let stderr = child.stderr.take().expect("piped stderr");
    let (tx, lines) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stderr)
            .lines()
            .map_while(Result::ok)
        {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    Running {
        _child: KillOnDrop(child),
        lines,
        seen: std::cell::RefCell::new(Vec::new()),
    }
}

impl Running {
    /// The first not-yet-seen line containing any of `needles`, waiting for it on
    /// the channel. On failure the panic prints every line that did arrive.
    fn line_with_any(&self, needles: &[&str]) -> String {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(line) => {
                    self.seen.borrow_mut().push(line.clone());
                    if needles.iter().any(|n| line.contains(n)) {
                        return line;
                    }
                }
                Err(_) => panic!(
                    "no stderr line containing any of {needles:?}; saw {:#?}",
                    self.seen.borrow()
                ),
            }
        }
    }

    fn line_with(&self, needle: &str) -> String {
        self.line_with_any(&[needle])
    }

    #[cfg(target_os = "linux")]
    fn pid(&self) -> u32 {
        self._child.0.id()
    }
}

/// The local addresses (`ADDR:PORT`, hex) of the TCP sockets `pid` is listening
/// on, read from the OS: the sockets it holds (`/proc/<pid>/fd`) against the
/// kernel's table (`/proc/net/tcp*`, state `0A`).
#[cfg(target_os = "linux")]
fn listening_sockets(pid: u32) -> Vec<String> {
    let mut inodes = std::collections::HashSet::new();
    for entry in std::fs::read_dir(format!("/proc/{pid}/fd")).expect("read the fd table") {
        let Ok(target) = std::fs::read_link(entry.expect("fd entry").path()) else {
            continue;
        };
        if let Some(inode) = target
            .to_string_lossy()
            .strip_prefix("socket:[")
            .and_then(|rest| rest.strip_suffix(']'))
        {
            inodes.insert(inode.to_owned());
        }
    }
    let mut listening = Vec::new();
    for table in ["tcp", "tcp6"] {
        let text = std::fs::read_to_string(format!("/proc/{pid}/net/{table}"))
            .expect("read the tcp table");
        for row in text.lines().skip(1) {
            let cols: Vec<&str> = row.split_whitespace().collect();
            if let (Some(local), Some("0A"), Some(inode)) =
                (cols.get(1), cols.get(3).copied(), cols.get(9))
                && inodes.contains(*inode)
            {
                listening.push((*local).to_owned());
            }
        }
    }
    listening
}

/// A collector the test owns: a listener bound once (its port read from it, never
/// picked by bind-and-drop) that answers every POST 200 and hands each request
/// body to a channel.
struct Collector {
    listener: std::net::TcpListener,
}

impl Collector {
    fn bind() -> Self {
        Self {
            listener: std::net::TcpListener::bind("127.0.0.1:0").expect("bind the collector"),
        }
    }

    fn url(&self) -> String {
        format!(
            "http://{}",
            self.listener.local_addr().expect("collector address")
        )
    }

    /// Serves requests on a thread until the process ends, sending each body.
    fn serve(self) -> (String, std::sync::mpsc::Receiver<String>) {
        use std::io::{Read, Write};
        let url = self.url();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            while let Ok((mut stream, _)) = self.listener.accept() {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 8192];
                let body = loop {
                    let Ok(n) = stream.read(&mut chunk) else {
                        break None;
                    };
                    if n == 0 {
                        break None;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf).into_owned();
                    if let Some((head, rest)) = text.split_once("\r\n\r\n") {
                        let len = head
                            .lines()
                            .find_map(|l| l.strip_prefix("Content-Length: "))
                            .and_then(|v| v.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        if rest.len() >= len {
                            break Some(rest.to_owned());
                        }
                    }
                };
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
                if let Some(body) = body
                    && tx.send(body).is_err()
                {
                    break;
                }
            }
        });
        (url, rx)
    }
}

/// Tracing: TC-045-01
/// ACs: FR-045-AC-1
///
/// With no endpoint in any tier the watcher sends nothing, logs nothing about
/// OTLP or metrics, and (on Linux, from the OS) holds no listening socket. The
/// log is read from the process's first line to the supervisor's first line,
/// which is printed after the OTLP decision.
#[test]
fn an_unset_endpoint_binds_nothing_and_logs_nothing_about_otlp() {
    let dir = tempfile::tempdir().unwrap();
    // A malformed OTEL_RESOURCE_ATTRIBUTES is present, and still says nothing: with
    // no push starting there is nothing to attach it to.
    let watcher = start_watcher(dir.path(), &[("OTEL_RESOURCE_ATTRIBUTES", "bad".into())]);
    watcher.line_with("redis unreachable");
    let log = watcher.seen.borrow().clone();
    assert!(
        log.iter().all(|line| {
            let line = line.to_lowercase();
            !["metrics", "otlp", "otel", "malformed", "pair"]
                .iter()
                .any(|word| line.contains(word))
        }),
        "no OTLP line expected: {log:#?}"
    );
    #[cfg(target_os = "linux")]
    assert_eq!(listening_sockets(watcher.pid()), Vec::<String>::new());
}

/// Tracing: TC-045-01
/// ACs: FR-045-AC-1
///
/// When a push does start, the skipped `OTEL_RESOURCE_ATTRIBUTES` pairs are counted
/// in one line.
#[test]
fn skipped_resource_attribute_pairs_are_counted_when_a_push_starts() {
    let dir = tempfile::tempdir().unwrap();
    let collector = Collector::bind();
    let watcher = start_watcher(
        dir.path(),
        &[
            ("IX_RELAY_WATCHER_OTLP_ENDPOINT", collector.url()),
            ("OTEL_RESOURCE_ATTRIBUTES", "bad,ok=1,=x".into()),
        ],
    );
    let line = watcher.line_with("malformed pair(s)");
    assert!(line.contains("2 malformed"), "{line}");
}

/// Tracing: TC-045-16
/// ACs: FR-045-AC-16
///
/// Generic OpenTelemetry variables that this exporter cannot honour never stop the
/// watcher: it starts, sends nothing, and says once which variable it ignored.
#[test]
fn an_inherited_generic_setting_is_ignored_with_one_line_and_never_fatal() {
    for env in [
        vec![("OTEL_EXPORTER_OTLP_ENDPOINT", "https://collector:4318")],
        vec![
            ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://collector:4317"),
            ("OTEL_EXPORTER_OTLP_PROTOCOL", "grpc"),
        ],
    ] {
        let dir = tempfile::tempdir().unwrap();
        let env: Vec<(&str, String)> = env.into_iter().map(|(k, v)| (k, v.to_owned())).collect();
        let watcher = start_watcher(dir.path(), &env);
        let line = watcher.line_with("OTEL_EXPORTER_OTLP_ENDPOINT ignored");
        assert!(!line.contains("collector"), "no value in the line: {line}");
        watcher.line_with("redis unreachable");
        let log = watcher.seen.borrow().clone();
        assert_eq!(
            log.iter().filter(|l| l.contains("ignored")).count(),
            1,
            "one line: {log:#?}"
        );
        assert!(
            log.iter().all(|l| !l.contains("pushing metrics")),
            "{log:#?}"
        );
    }
}

/// Tracing: TC-045-14
/// ACs: FR-045-AC-14
///
/// An interval outside 5 to 3,600 seconds, however large, is refused at start-up
/// with exit 1 and a message naming the variable or the file's line: never a
/// panic (exit 101), from the environment or from the file.
#[test]
fn an_out_of_range_interval_is_refused_at_startup_and_never_panics() {
    let dir = tempfile::tempdir().unwrap();
    for value in [
        "18446744073709551615",
        "9223372036854775807",
        "3601",
        "4",
        "0",
    ] {
        let out = run_with_env(
            dir.path(),
            &[
                (
                    "IX_RELAY_WATCHER_OTLP_ENDPOINT",
                    "http://127.0.0.1:1".into(),
                ),
                ("IX_RELAY_WATCHER_OTLP_INTERVAL_SECS", value.into()),
            ],
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{value}: {stderr}");
        assert!(
            stderr.contains("IX_RELAY_WATCHER_OTLP_INTERVAL_SECS"),
            "{stderr}"
        );
        assert!(!stderr.contains("panicked"), "{stderr}");
    }
    for value in ["9223372036854775807", "3601"] {
        let file = tempfile::tempdir().unwrap();
        std::fs::write(
            file.path().join("config.toml"),
            format!(
                "[watcher]\notlp_endpoint = \"http://127.0.0.1:1\"\notlp_interval_secs = {value}\n"
            ),
        )
        .unwrap();
        let out = run_with_env(file.path(), &[]);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{value}: {stderr}");
        assert!(
            stderr.contains("config.toml:3:") && stderr.contains("otlp_interval_secs"),
            "{stderr}"
        );
        assert!(!stderr.contains("panicked"), "{stderr}");
    }
}

/// Tracing: TC-045-07
/// ACs: FR-045-AC-1, FR-045-AC-7
///
/// With an endpoint the watcher says once where it pushes (host and port only,
/// never the path or a header), and still binds nothing: it only connects out.
#[test]
fn a_configured_endpoint_is_announced_by_target_only_and_binds_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let collector = Collector::bind();
    let url = collector.url();
    let target = url.trim_start_matches("http://").to_owned();
    let watcher = start_watcher(
        dir.path(),
        &[
            ("OTEL_EXPORTER_OTLP_ENDPOINT", format!("{url}/private/path")),
            (
                "OTEL_EXPORTER_OTLP_HEADERS",
                "Authorization=Bearer s3cret".into(),
            ),
        ],
    );
    let line = watcher.line_with("pushing metrics to");
    assert!(line.contains(&target), "{line}");
    assert!(
        !line.contains("private") && !line.contains("s3cret"),
        "{line}"
    );
    watcher.line_with("redis unreachable");
    let log = watcher.seen.borrow().clone();
    assert!(log.iter().all(|l| !l.contains("s3cret")), "{log:#?}");
    #[cfg(target_os = "linux")]
    assert_eq!(
        listening_sockets(watcher.pid()),
        Vec::<String>::new(),
        "the watcher only connects out (the test's own listener is another process's)"
    );
    drop(collector);
}

/// Tracing: TC-045-02
/// ACs: FR-045-AC-2
///
/// An `https://` endpoint is refused at start-up, naming what supplied it and
/// why, and a credential in it is never echoed.
#[test]
fn an_https_endpoint_is_refused_at_startup_naming_its_source() {
    let dir = tempfile::tempdir().unwrap();
    for (env, want) in [
        (
            ("IX_RELAY_WATCHER_OTLP_ENDPOINT", "https://collector:4318"),
            "IX_RELAY_WATCHER_OTLP_ENDPOINT",
        ),
        (
            (
                "IX_RELAY_WATCHER_OTLP_ENDPOINT",
                "https://u:hunter2@collector/v1/metrics",
            ),
            "IX_RELAY_WATCHER_OTLP_ENDPOINT",
        ),
    ] {
        let out = run_with_env(dir.path(), &[(env.0, env.1.to_owned())]);
        assert_eq!(out.status.code(), Some(1), "{env:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains(want) && stderr.contains("no TLS"),
            "{stderr}"
        );
        assert!(!stderr.contains("hunter2"), "{stderr}");
    }
    std::fs::write(
        dir.path().join("config.toml"),
        "[watcher]\notlp_endpoint = \"https://collector:4318\"\n",
    )
    .unwrap();
    let out = run_with_env(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("config.toml:2:") && stderr.contains("otlp_endpoint"),
        "{stderr}"
    );
}

/// Tracing: TC-045-02
/// ACs: FR-045-AC-2
///
/// A bad watcher-only key in the file is refused by the watcher alone, at
/// start-up, with the file, the real line and the key.
#[test]
fn a_bad_otlp_value_in_the_file_is_refused_with_its_line() {
    for (text, line) in [
        ("[watcher]\notlp_endpoint = 5\n", "config.toml:2:"),
        (
            "[watcher]\ninterval_secs = 2\notlp_headers = \"novalue\"\n",
            "config.toml:3:",
        ),
        ("[watcher]\notlp_interval_secs = 1\n", "config.toml:2:"),
        (
            "[watcher]\notlp_interval_secs = 9223372036854775807\n",
            "config.toml:2:",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.toml"), text).unwrap();
        let out = run_with_env(dir.path(), &[]);
        assert_eq!(out.status.code(), Some(1), "{text:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains(line), "{text:?}: {stderr}");
        assert!(stderr.contains("otlp_"), "names no key? {stderr}");
    }
}

/// Tracing: TC-045-06
/// ACs: FR-045-AC-6, FR-045-AC-7
///
/// Against a real Redis and a collector the test owns: the running watcher pushes
/// OTLP JSON with the relay's series and the pane-independent health, the
/// heartbeat says `sending` to the collector's host and port, and the OS shows no
/// listener. Redis lane.
#[test]
fn a_running_watcher_pushes_to_the_collector_and_reports_sending() {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
        return;
    };
    let prefix = format!("test:{}:otlp-process:", std::process::id());
    let dir = tempfile::tempdir().unwrap();
    let (endpoint, bodies) = Collector::bind().serve();
    let target = endpoint.trim_start_matches("http://").to_owned();
    let child = Command::new(env!("CARGO_BIN_EXE_relay-watcher"))
        .env_clear()
        .env("IX_RELAY_CONFIG", dir.path().join("config.toml"))
        .env("IX_RELAY_MACHINE", "otlphost")
        .env("OTEL_EXPORTER_OTLP_ENDPOINT", &endpoint)
        .env("OTEL_SERVICE_NAME", "relay-under-test")
        .args(["--url", &url, "--prefix", &prefix, "--sweep-secs", "1"])
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn relay-watcher");
    let child = KillOnDrop(child);
    let body = bodies
        .recv_timeout(std::time::Duration::from_secs(30))
        .expect("the collector received a push");
    assert!(body.contains("\"resourceMetrics\""), "{body}");
    assert!(
        body.contains("\"stringValue\":\"relay-under-test\""),
        "{body}"
    );
    assert!(body.contains("\"stringValue\":\"otlphost\""), "{body}");
    assert!(body.contains("ix.relay.registrations"), "{body}");
    let mut relay = relay_core::Relay::connect(&url, std::time::Duration::from_secs(2))
        .unwrap()
        .with_prefix(prefix.clone());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        if let Some(relay_core::health::HeartbeatRead::Live(hb)) =
            relay.heartbeat(&"otlphost".parse().unwrap()).unwrap()
            && matches!(
                &hb.otlp,
                Some(relay_core::health::OtlpStatus::Sending { target: t, .. }) if *t == target
            )
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the heartbeat never said sending to {target}"
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    #[cfg(target_os = "linux")]
    assert_eq!(listening_sockets(child.0.id()), Vec::<String>::new());
    drop(child);
    let mut raw = redis::Client::open(url.as_str())
        .unwrap()
        .get_connection()
        .unwrap();
    redis::cmd("DEL")
        .arg(format!("{prefix}health:watcher:otlphost"))
        .exec(&mut raw)
        .unwrap();
}

/// Tracing: TC-045-01
/// ACs: FR-045-AC-1
///
/// `--once` sends nothing even with an endpoint. The process has exited when the
/// listener is polled, so nothing can still arrive: an empty non-blocking accept
/// is the proof, not a wait. Redis lane (a `--once` that cannot reach Redis
/// exits before it would push anyway).
#[test]
fn once_sends_nothing() {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let collector = Collector::bind();
    collector.listener.set_nonblocking(true).unwrap();
    // A poll that a transient Redis error (an interrupted read) fails proves
    // nothing, so it is run again; three failures in a row are a failure.
    let mut out = None;
    for _ in 0..3 {
        let run = Command::new(env!("CARGO_BIN_EXE_relay-watcher"))
            .env_clear()
            .env("IX_RELAY_CONFIG", dir.path().join("config.toml"))
            .env("IX_RELAY_MACHINE", "onceotlp")
            .env("OTEL_EXPORTER_OTLP_ENDPOINT", collector.url())
            .args([
                "--once",
                "--url",
                &url,
                "--prefix",
                &format!("test:{}:otlp-once:", std::process::id()),
            ])
            .output()
            .expect("spawn relay-watcher");
        let ok = run.status.success();
        out = Some(run);
        if ok {
            break;
        }
    }
    let out = out.expect("ran at least once");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("pushing metrics"), "{stderr}");
    assert!(!stderr.contains("malformed"), "{stderr}");
    assert!(
        matches!(collector.listener.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock),
        "--once connected to the collector"
    );
}

fn run_with_env(dir: &Path, env: &[(&str, String)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_relay-watcher"));
    command
        .env_clear()
        .env("IX_RELAY_CONFIG", dir.join("config.toml"))
        .args(["--url", "redis://127.0.0.1:1"]);
    for (name, value) in env {
        command.env(name, value);
    }
    command.output().expect("spawn relay-watcher")
}

/// Tracing: TC-044-08
/// ACs: FR-044-AC-8
///
/// A SIGTERM (a service manager's stop) ends the loop at its next turn and the
/// watcher removes its reply socket and status file on the way out. Redis lane: the
/// loop needs a bus to run its turns on.
#[test]
fn a_sigterm_removes_the_reply_socket_and_the_status_file() {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let runtime = dir.path().join("run");
    std::fs::create_dir_all(runtime.join("cc-socks")).unwrap();
    std::fs::create_dir_all(dir.path().join("claude")).unwrap();
    std::fs::write(
        dir.path().join("config.toml"),
        "[watcher]\ndelivery = \"socket\"\n",
    )
    .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_relay-watcher"))
        .env_clear()
        .env("IX_RELAY_CONFIG", dir.path().join("config.toml"))
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("IX_RELAY_CLAUDE_DIR", dir.path().join("claude"))
        .env(
            "IX_RELAY_PREFIX",
            format!("test:{}:sigterm:", std::process::id()),
        )
        .args(["--url", &url])
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn relay-watcher");
    let pid = child.id();
    let socket = runtime.join("cc-socks").join(format!("{pid}.sock"));
    let status = dir
        .path()
        .join("state")
        .join("socket")
        .join(format!("socket-{pid}.json"));
    let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !(socket.exists() && status.exists()) {
        assert!(
            std::time::Instant::now() < until,
            "the watcher never bound its socket"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let sent = Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .expect("run kill");
    assert!(sent.success());
    let code = loop {
        if let Some(code) = child.try_wait().unwrap() {
            break code;
        }
        if std::time::Instant::now() >= until {
            let _ = child.kill();
            panic!("the watcher did not stop on SIGTERM");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    assert!(code.success(), "{code:?}");
    assert!(!socket.exists(), "the reply socket is removed");
    assert!(!status.exists(), "the status file is removed");
}
