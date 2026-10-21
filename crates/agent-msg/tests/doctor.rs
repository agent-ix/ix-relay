//! `agent-msg doctor` as a process: exit codes, the typed output, Redis down,
//! and that it changes nothing. Redis-backed cases skip without
//! `IX_RELAY_TEST_URL` (`make test-redis`); the Redis-down cases need none.
//!
//! Each run gets a private `PATH` holding stub `herdr` and `systemctl` scripts,
//! so the machine's real herdr and units are never asked and the answers are
//! known.
#![expect(
    clippy::unwrap_used,
    reason = "test code: a failed unwrap/expect is the failure signal"
)]

mod support;

use std::{
    os::unix::fs::PermissionsExt,
    path::Path,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use relay_core::{
    Message, Registration, Relay,
    health::{WATCHER_FILE_NAME, WatcherFile, WatcherHeartbeat},
};

const MACHINE: &str = "hub";

/// A herdr that reports a supported version and no agents.
const HERDR_OK: &str = "#!/bin/sh\ncase \"$1\" in\n  --version) echo 'herdr 0.9.1';;\n  agent) echo '{\"result\":{\"agents\":[]}}';;\n  workspace) echo '{\"result\":{\"workspaces\":[]}}';;\n  tab) echo '{\"result\":{\"tabs\":[]}}';;\nesac\n";

/// A systemctl that says the unit is active.
const SYSTEMCTL_ACTIVE: &str = "#!/bin/sh\necho active\n";

fn stub(dir: &Path, name: &str, body: &str) {
    let path = dir.join(name);
    std::fs::write(&path, body).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn now_ms() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
}

/// One doctor run's world: a config directory, a `PATH` directory and the bus.
struct World {
    config: support::Scratch,
    bin: support::Scratch,
    url: String,
    prefix: String,
    /// Whether the bus is real (the cases with a dead or silent Redis wrote
    /// nothing and cannot be reached to clean).
    cleanup: bool,
}

impl World {
    /// A world on `url` with a herdr and a systemctl that pass.
    fn new(url: &str, test: &str) -> Self {
        let world = Self {
            config: support::Scratch::new("doctor-config-"),
            bin: support::Scratch::new("doctor-bin-"),
            url: url.to_owned(),
            prefix: format!("test:{}:doctor-{test}:", std::process::id()),
            cleanup: std::env::var("IX_RELAY_TEST_URL").is_ok_and(|real| real == url),
        };
        stub(world.bin.path(), "herdr", HERDR_OK);
        stub(world.bin.path(), "systemctl", SYSTEMCTL_ACTIVE);
        world
    }

    fn run(&self, extra: &[&str]) -> support::Run {
        let mut args = vec!["doctor"];
        args.extend_from_slice(extra);
        let mut cmd = support::base_cmd(&args, None);
        cmd.env("IX_RELAY_URL", &self.url)
            .env("IX_RELAY_PREFIX", &self.prefix)
            .env("IX_RELAY_MACHINE", MACHINE)
            .env("IX_RELAY_CONFIG", self.config.path().join("config.toml"))
            .env("PATH", self.bin.path());
        support::run(cmd, "")
    }

    fn relay(&self) -> Relay {
        Relay::connect(&self.url, Duration::from_secs(2))
            .unwrap()
            .with_prefix(self.prefix.clone())
    }

    /// A watcher.json for this process, as a healthy watcher writes it.
    fn watcher_file(&self, pid: u32) {
        let dir = self.config.path().join("state");
        std::fs::create_dir_all(&dir).unwrap();
        let file = WatcherFile {
            pid,
            started_at_ms: 1,
            last_ok_tick_ms: Some(now_ms()),
            last_error: None,
        };
        std::fs::write(
            dir.join(WATCHER_FILE_NAME),
            serde_json::to_vec(&file).unwrap(),
        )
        .unwrap();
    }

    fn heartbeat(&self, machine: &str, pid: u32) {
        let hb = WatcherHeartbeat {
            machine: machine.parse().unwrap(),
            pid,
            version: "test".into(),
            started_at_ms: 1,
            last_tick_ms: now_ms(),
            last_sweep_ms: None,
            sweep_secs: 15,
            herdr_ok: true,
            herdr_problem: None,
            herdr_blocks_delivery: false,
            redis_ok: true,
            leads_watched: 1,
            otlp: Some(relay_core::health::OtlpStatus::Disabled),
            last_error: None,
        };
        self.relay()
            .write_heartbeat(&hb, Duration::from_secs(60))
            .unwrap();
    }

    fn register(&self, name: &str, machine: &str) {
        self.relay()
            .register(&Registration {
                name: name.parse().unwrap(),
                machine: machine.parse().unwrap(),
                kind: "claude".into(),
                session_id: None,
                cwd: None,
                herdr_pane: None,
                place: None,
                last_seen_ms: now_ms(),
            })
            .unwrap();
    }

    /// Removes every key the case wrote.
    fn clean(&self) {
        let mut raw = redis::Client::open(self.url.as_str())
            .unwrap()
            .get_connection()
            .unwrap();
        let keys: Vec<String> = redis::cmd("KEYS")
            .arg(format!("{}*", self.prefix))
            .query(&mut raw)
            .unwrap();
        if !keys.is_empty() {
            redis::cmd("DEL").arg(keys).exec(&mut raw).unwrap();
        }
    }
}

impl Drop for World {
    fn drop(&mut self) {
        if self.cleanup {
            self.clean();
        }
    }
}

fn test_url() -> Option<String> {
    let url = std::env::var("IX_RELAY_TEST_URL").ok();
    if url.is_none() {
        eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
    }
    url
}

fn json(run: &support::Run) -> serde_json::Value {
    serde_json::from_str(run.stdout.trim()).unwrap_or_else(|e| panic!("{e}: {}", run.stdout))
}

fn codes(report: &serde_json::Value, status: &str) -> Vec<String> {
    report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["status"] == status)
        .map(|c| c["code"].as_str().unwrap().to_owned())
        .collect()
}

/// Tracing: TC-029-01
/// ACs: FR-029-AC-1
///
/// A relay with a live heartbeat on every machine that has leads, a supported
/// herdr, a consistent state file and nothing failed or waiting exits 0.
#[test]
fn tc_029_01_a_healthy_relay_exits_0() {
    let Some(url) = test_url() else { return };
    let world = World::new(&url, "healthy");
    world.register("hub-lead", "hub");
    world.register("node-a-lead", "node-a");
    world.heartbeat("hub", std::process::id());
    world.heartbeat("node-a", 999);
    world.watcher_file(std::process::id());
    let run = world.run(&["--json"]);
    let report = json(&run);
    assert_eq!(run.status, 0, "{report}");
    assert_eq!(report["ok"], true);
    assert!(codes(&report, "fail").is_empty(), "{report}");
    // The text form is the same report.
    let text = world.run(&[]);
    assert_eq!(text.status, 0, "{}", text.stdout);
    assert!(
        text.stdout.trim_end().ends_with("relay healthy"),
        "{}",
        text.stdout
    );
}

/// Tracing: TC-029-02
/// ACs: FR-029-AC-2
#[test]
fn tc_029_02_a_machine_with_leads_and_no_heartbeat_fails_naming_it() {
    let Some(url) = test_url() else { return };
    let world = World::new(&url, "no-heartbeat");
    world.register("node-a-lead", "node-a");
    world.heartbeat("hub", std::process::id());
    world.watcher_file(std::process::id());
    let run = world.run(&[]);
    assert_eq!(run.status, 1, "{}", run.stdout);
    let line = run
        .stdout
        .lines()
        .find(|l| l.contains("watcher_heartbeat_missing") && l.contains("node-a"))
        .unwrap_or_else(|| {
            panic!(
                "no line names node-a and the missing watcher:\n{}",
                run.stdout
            )
        });
    assert!(line.contains("FAIL"), "{line}");
}

/// Tracing: TC-029-03
/// ACs: FR-029-AC-3
///
/// With Redis unreachable `doctor` exits 1 well inside 5 s, says so, and still
/// reports the checks that do not need it (herdr, the unit, the state file).
#[test]
fn tc_029_03_with_redis_down_doctor_fails_fast_and_still_reports_herdr() {
    let world = World::new("redis://127.0.0.1:1", "down");
    let started = Instant::now();
    let run = world.run(&["--json"]);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(run.status, 1);
    let report = json(&run);
    assert_eq!(report["ok"], false);
    assert!(codes(&report, "fail").contains(&"redis_unreachable".to_owned()));
    // TC-029-13: the refused connect reaches the check as its own fault.
    let redis = report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["code"] == "redis_unreachable")
        .unwrap();
    let cause = redis["cause"].as_str().unwrap();
    assert!(cause.contains("nothing listens"), "{cause}");
    let herdr = report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["check"] == "herdr")
        .expect("the herdr check is reported without Redis");
    assert_eq!(herdr["status"], "ok");
}

/// Tracing: TC-029-04
/// ACs: FR-029-AC-4
#[test]
fn tc_029_04_no_output_line_contains_the_redis_password() {
    let world = World::new("redis://user:s3cretpw@127.0.0.1:1", "password");
    for extra in [&["--json"][..], &[]] {
        let run = world.run(extra);
        assert_eq!(run.status, 1);
        assert!(!run.stdout.contains("s3cretpw"), "{}", run.stdout);
        assert!(!run.stderr.contains("s3cretpw"), "{}", run.stderr);
    }
    let text = world.run(&[]);
    assert!(
        text.stdout.contains("redis://***@127.0.0.1:1"),
        "{}",
        text.stdout
    );
}

/// Tracing: TC-029-05
/// ACs: FR-029-AC-5
///
/// `doctor` reads only: the same keys, the same inbox lengths and the same
/// heartbeat afterwards, and no spool or state file appears.
#[test]
fn tc_029_05_doctor_changes_nothing() {
    let Some(url) = test_url() else { return };
    let world = World::new(&url, "read-only");
    world.register("hub-lead", "hub");
    let mut relay = world.relay();
    for at in [10, 20] {
        relay
            .send(&Message {
                from: "hub-lead".parse().unwrap(),
                to: "hub-lead".parse().unwrap(),
                text: "queued".into(),
                sent_at_ms: at,
                forwarded: false,
            })
            .unwrap();
    }
    world.heartbeat("hub", std::process::id());
    for (key, rows) in [
        ("deadletter:hub-lead", vec!["x"]),
        ("inflight:hub-lead", vec!["1 hook 1 {}"]),
    ] {
        let mut cmd = redis::cmd("RPUSH");
        cmd.arg(format!("{}{key}", world.prefix));
        for row in rows {
            cmd.arg(row);
        }
        cmd.exec(
            &mut redis::Client::open(url.as_str())
                .unwrap()
                .get_connection()
                .unwrap(),
        )
        .unwrap();
    }
    let mut raw = redis::Client::open(url.as_str())
        .unwrap()
        .get_connection()
        .unwrap();
    // Every key's bytes (DUMP) and whether it has an expiry: a doctor that
    // rewrote a value, or renewed or dropped a TTL, changes this.
    let snapshot = |raw: &mut redis::Connection| {
        let keys: Vec<String> = redis::cmd("KEYS")
            .arg(format!("{}*", world.prefix))
            .query(raw)
            .unwrap();
        let mut state: Vec<(String, Vec<u8>, bool)> = keys
            .into_iter()
            .map(|k| {
                let dump: Vec<u8> = redis::cmd("DUMP").arg(&k).query(raw).unwrap();
                let ttl: i64 = redis::cmd("PTTL").arg(&k).query(raw).unwrap();
                (k, dump, ttl >= 0)
            })
            .collect();
        state.sort();
        state
    };
    let before = snapshot(&mut raw);
    assert!(
        before
            .iter()
            .any(|(k, _, ttl)| k.ends_with("health:watcher:hub") && *ttl)
    );
    for extra in [&["--json"][..], &["--strict"], &[]] {
        world.run(extra);
    }
    assert_eq!(snapshot(&mut raw), before);
    assert!(!world.config.path().join("spool").exists());
    assert!(!world.config.path().join("state").exists());
}

/// Tracing: TC-029-06
/// ACs: FR-029-AC-6
/// Trace: FR-049-AC-1
///
/// The JSON is one typed object: `ok` and `checks`, each check with exactly the
/// documented fields, a stable code and one of three statuses.
#[test]
fn tc_029_06_the_json_output_is_typed() {
    let world = World::new("redis://127.0.0.1:1", "json");
    let run = world.run(&["--json"]);
    assert_eq!(run.stdout.trim().lines().count(), 1, "one JSON line");
    assert!(
        run.stdout
            .starts_with("{\"ok\":false,\"strict\":false,\"checks\":[")
    );
    assert!(run.stdout.ends_with("]}\n"));
    let report = json(&run);
    let mut top: Vec<_> = report.as_object().unwrap().keys().cloned().collect();
    top.sort();
    assert_eq!(top, ["checks", "ok", "strict"]);
    for check in report["checks"].as_array().unwrap() {
        let mut keys: Vec<_> = check.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "cause", "check", "code", "details", "fix", "message", "status", "subject"
            ]
        );
        assert!(["ok", "warn", "fail", "unknown"].contains(&check["status"].as_str().unwrap()));
        assert!(
            check["code"]
                .as_str()
                .unwrap()
                .chars()
                .all(|c| c.is_ascii_lowercase() || c == '_'),
            "{check}"
        );
    }
}

/// Tracing: TC-029-07
/// ACs: FR-029-AC-7
///
/// herdr missing is a failing check with its own code; a service manager that
/// cannot be asked is `unknown` and does not fail the exit by itself.
#[test]
fn tc_029_07_a_missing_herdr_fails_and_an_unaskable_service_manager_does_not() {
    let Some(url) = test_url() else { return };
    let world = World::new(&url, "faults");
    world.heartbeat("hub", std::process::id());
    world.watcher_file(std::process::id());
    // Nothing on PATH at all: no herdr, no systemctl, no launchctl.
    std::fs::remove_file(world.bin.path().join("herdr")).unwrap();
    std::fs::remove_file(world.bin.path().join("systemctl")).unwrap();
    let report = json(&world.run(&["--json"]));
    assert_eq!(codes(&report, "fail"), ["herdr_missing"], "{report}");
    assert!(codes(&report, "unknown").contains(&"service_unknown".to_owned()));
    assert_eq!(report["ok"], false);

    // Restore Herdr and verify the unavailable service-manager classification.
    stub(world.bin.path(), "herdr", HERDR_OK);
    let run = world.run(&["--json"]);
    assert_eq!(run.status, 0, "{}", run.stdout);
    let report = json(&run);
    let service = report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["check"] == "service")
        .unwrap();
    assert_eq!(service["code"], "service_unknown", "{report}");
    assert_eq!(service["status"], "unknown", "{report}");
    assert!(
        service["message"]
            .as_str()
            .unwrap()
            .contains("neither systemctl nor launchctl")
    );
    assert_eq!(report["ok"], true, "{report}");
    // Other process probes may correctly be unknown under their independent
    // deadline. Their classification is verified by dedicated Herdr cases.
}

/// Tracing: TC-029-09
/// ACs: FR-029-AC-9
///
/// Exit 1 means something: a dead letter is a warning (exit 0, reported with the
/// `agent-msg inbox --dead` hint), `--strict` makes it fail, and the JSON carries
/// the severity.
#[test]
fn tc_029_09_a_warning_exits_0_and_strict_makes_it_fail() {
    let Some(url) = test_url() else { return };
    let world = World::new(&url, "warn");
    world.register("hub-lead", "hub");
    world.heartbeat("hub", std::process::id());
    world.watcher_file(std::process::id());
    let mut raw = redis::Client::open(url.as_str())
        .unwrap()
        .get_connection()
        .unwrap();
    redis::cmd("RPUSH")
        .arg(format!("{}deadletter:hub-lead", world.prefix))
        .arg("a dead entry")
        .exec(&mut raw)
        .unwrap();
    let lax = world.run(&["--json"]);
    assert_eq!(lax.status, 0, "{}", lax.stdout);
    let report = json(&lax);
    assert_eq!(codes(&report, "warn"), ["dead_letters_present"]);
    assert_eq!(report["ok"], true);
    assert_eq!(report["strict"], false);
    let strict = world.run(&["--json", "--strict"]);
    assert_eq!(strict.status, 1, "{}", strict.stdout);
    assert_eq!(json(&strict)["ok"], false);
    assert!(world.run(&[]).stdout.contains("agent-msg inbox --dead"));
}

/// A Redis that accepts and never answers, standing in for a hung tunnel.
fn silent_redis() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        // Hold every connection open and read nothing back.
        let mut held = Vec::new();
        for stream in listener.incoming().flatten() {
            held.push(stream);
        }
    });
    format!("redis://{addr}")
}

/// Tracing: TC-029-03
/// ACs: FR-029-AC-3
///
/// The bound holds with everything failing at once: a Redis that accepts and
/// never answers, and a `herdr` and a `systemctl` that hang. The children are
/// killed at their own deadlines and the whole run is inside 5 s. (A wall-clock
/// assertion: the slowest allowed is about 4 s, so a loaded machine is the only
/// way to fail it spuriously.)
#[test]
fn tc_029_03_doctor_stays_inside_five_seconds_with_a_silent_redis_and_hung_children() {
    let world = World::new(&silent_redis(), "hung");
    stub(
        world.bin.path(),
        "herdr",
        "#!/bin/sh
exec /bin/sleep 30
",
    );
    stub(
        world.bin.path(),
        "systemctl",
        "#!/bin/sh
exec /bin/sleep 30
",
    );
    let started = Instant::now();
    let run = world.run(&["--json"]);
    let took = started.elapsed();
    assert!(took < Duration::from_secs(5), "took {took:?}");
    assert_eq!(run.status, 1);
    let report = json(&run);
    assert!(codes(&report, "fail").contains(&"redis_unreachable".to_owned()));
    // The hung children are unknown, not a failure of herdr or the unit.
    assert!(codes(&report, "unknown").contains(&"herdr_unchecked".to_owned()));
    assert!(codes(&report, "unknown").contains(&"service_unknown".to_owned()));
}

/// A fake Redis: answers `PING` at once, `HGETALL` with an empty array, and each
/// `SCAN` after `delay` with a cursor that is never zero, so a scan never ends.
fn slow_scan_redis(delay: Duration) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || {
                use std::io::{Read, Write};
                let mut stream = stream;
                let mut buf = [0u8; 4096];
                while let Ok(n @ 1..) = stream.read(&mut buf) {
                    let text = String::from_utf8_lossy(&buf[..n]).to_uppercase();
                    let reply: &[u8] = if text.contains("PING") {
                        b"+PONG\r\n"
                    } else if text.contains("SCAN") {
                        std::thread::sleep(delay);
                        b"*2\r\n$1\r\n7\r\n*0\r\n"
                    } else {
                        b"*0\r\n"
                    };
                    if stream.write_all(reply).is_err() {
                        return;
                    }
                }
            });
        }
    });
    format!("redis://{addr}")
}

/// Tracing: TC-029-03
/// ACs: FR-029-AC-3
///
/// The Redis budget is one deadline for the whole section, not one per call: a
/// Redis that answers every `SCAN` in 0.9 s with a cursor that never ends would
/// have cost 64 calls (a minute) with a per-call timeout. Doctor stops the scan at
/// its deadline, reports that the heartbeats were not read, and is inside 5 s
/// even with hung children.
#[test]
fn tc_029_03_a_slow_multi_call_scan_is_cut_by_the_one_deadline() {
    let world = World::new(&slow_scan_redis(Duration::from_millis(900)), "slowscan");
    stub(world.bin.path(), "herdr", "#!/bin/sh\nexec /bin/sleep 30\n");
    stub(
        world.bin.path(),
        "systemctl",
        "#!/bin/sh\nexec /bin/sleep 30\n",
    );
    let started = Instant::now();
    let run = world.run(&["--json"]);
    let took = started.elapsed();
    assert!(took < Duration::from_secs(5), "took {took:?}");
    let report = json(&run);
    assert!(
        codes(&report, "warn").contains(&"heartbeat_scan_truncated".to_owned()),
        "{report}"
    );
    assert_eq!(run.status, 1, "the reads after the deadline could not run");
}

/// Tracing: TC-029-10
/// ACs: FR-029-AC-10
/// Trace: FR-049-AC-2
///
/// A closed pipe is not a failure of the relay: `agent-msg doctor | head` exits
/// with the relay's health (0 here), not 1.
#[test]
fn tc_029_10_a_closed_pipe_exits_with_the_relays_health() {
    let Some(url) = test_url() else { return };
    let world = World::new(&url, "epipe");
    world.register("hub-lead", "hub");
    world.heartbeat("hub", std::process::id());
    world.watcher_file(std::process::id());
    // A slow herdr so the reader has closed the pipe long before doctor writes.
    stub(world.bin.path(), "herdr", "#!/bin/sh\nexec /bin/sleep 3\n");
    for args in [&["doctor"][..], &["doctor", "--json"][..]] {
        let mut cmd = support::base_cmd(args, None);
        cmd.env("IX_RELAY_URL", &world.url)
            .env("IX_RELAY_PREFIX", &world.prefix)
            .env("IX_RELAY_MACHINE", MACHINE)
            .env("IX_RELAY_CONFIG", world.config.path().join("config.toml"))
            .env("PATH", world.bin.path())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = cmd.spawn().unwrap();
        drop(child.stdout.take());
        let status = child.wait().unwrap();
        assert_eq!(
            status.code(),
            Some(0),
            "a healthy relay exits 0 with a closed pipe for {args:?}"
        );
    }
}

/// Tracing: TC-043-07
/// ACs: FR-043-AC-7
///
/// A relink and a rename the watcher recorded are listed by `doctor` as
/// information: the exit stays 0 and the check is `ok`.
#[test]
fn tc_043_07_doctor_lists_recorded_moves_without_failing() {
    let Some(url) = test_url() else { return };
    let world = World::new(&url, "moves");
    world.heartbeat("hub", std::process::id());
    world.watcher_file(std::process::id());
    let place = |workspace: &str| relay_core::Place {
        workspace: workspace.into(),
        tab: "1".into(),
        herdr_ids: None,
    };
    let mut relay = world.relay();
    let seen = Registration {
        name: "hub-lead".parse().unwrap(),
        machine: "hub".parse().unwrap(),
        kind: "claude".into(),
        session_id: Some("sess-1".parse().unwrap()),
        cwd: None,
        herdr_pane: Some("w1:p3".parse().unwrap()),
        place: Some(place("wsA")),
        last_seen_ms: now_ms(),
    };
    relay.register(&seen).unwrap();
    relay
        .relink_pane(&seen, &"w2:p2".parse().unwrap(), &place("wsB"), now_ms())
        .unwrap()
        .unwrap();
    relay
        .refresh_place(&seen.name, &place("wsB-renamed"), now_ms(), true)
        .unwrap();

    let run = world.run(&["--json"]);
    let report = json(&run);
    assert_eq!(run.status, 0, "{report}");
    let check = report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["check"] == "moves")
        .unwrap_or_else(|| panic!("no moves check: {report}"));
    assert_eq!(check["status"], "ok");
    let details: Vec<&str> = check["details"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d.as_str().unwrap())
        .collect();
    assert_eq!(details.len(), 2, "{details:?}");
    assert!(details[0].contains("renamed"), "{details:?}");
    assert!(
        details[1].contains("hub-lead moved pane w1:p3 to w2:p2")
            && details[1].contains("hub:wsA:1 -> hub:wsB:1"),
        "{details:?}"
    );
}
