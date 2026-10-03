//! The pop safety window (AGE-2033), and what stands behind it since AGE-2032.
//!
//! The inbox pop is one `EVAL` that takes messages out of the inbox
//! server-side. If its reply arrives after the client's read timeout the client
//! never sees them; they are in flight, and return when the lease expires. These tests put a small TCP proxy between the hook
//! and a real Redis that holds back replies. Redis-backed: they skip without
//! `IX_RELAY_TEST_URL` (`make test-redis`), and each passes on its own against
//! a fresh Redis (no script preloaded).
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: a failed unwrap/expect is the failure signal"
)]

mod support;

use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs},
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use relay_core::{AgentName, Claim, Message, Now, Popper, Registration, Relay, Target};
use support::{base_cmd, run, spool_path};

/// Holds back the reply to any request containing `needle` (any request when
/// it is empty) by `delay`.
struct Rule {
    needle: &'static [u8],
    delay: Duration,
}

impl Rule {
    fn matches(&self, request: &[u8]) -> bool {
        self.needle.is_empty() || request.windows(self.needle.len()).any(|w| w == self.needle)
    }
}

/// A TCP proxy in front of Redis. It forwards every byte, delays the reply
/// that follows a request a rule matches, and counts requests by needle.
/// Threads are left to end with the test process.
struct FaultProxy {
    addr: SocketAddr,
    requests: Arc<std::sync::Mutex<Vec<Vec<u8>>>>,
    accepted: Arc<AtomicUsize>,
}

impl FaultProxy {
    fn start(upstream: SocketAddr, rules: Vec<Rule>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let rules = Arc::new(rules);
        let requests: Arc<std::sync::Mutex<Vec<Vec<u8>>>> = Arc::default();
        let accepted = Arc::new(AtomicUsize::new(0));
        let (seen, count) = (Arc::clone(&requests), Arc::clone(&accepted));
        thread::spawn(move || {
            for client in listener.incoming() {
                let Ok(client) = client else { return };
                let Ok(server) = TcpStream::connect(upstream) else {
                    return;
                };
                count.fetch_add(1, Ordering::SeqCst);
                Self::pipe(client, server, Arc::clone(&rules), Arc::clone(&seen));
            }
        });
        Self {
            addr,
            requests,
            accepted,
        }
    }

    fn pipe(
        client: TcpStream,
        server: TcpStream,
        rules: Arc<Vec<Rule>>,
        seen: Arc<std::sync::Mutex<Vec<Vec<u8>>>>,
    ) {
        // Milliseconds to hold the next reply for; set by the request thread.
        let delay_ms = Arc::new(AtomicU64::new(0));
        let (mut client_r, mut client_w) = (client.try_clone().unwrap(), client);
        let (mut server_r, mut server_w) = (server.try_clone().unwrap(), server);
        let pending = Arc::clone(&delay_ms);
        thread::spawn(move || {
            let mut buf = [0u8; 16 * 1024];
            while let Ok(n @ 1..) = client_r.read(&mut buf) {
                seen.lock().unwrap().push(buf[..n].to_vec());
                for rule in rules.iter().filter(|r| r.matches(&buf[..n])) {
                    let ms = u64::try_from(rule.delay.as_millis()).unwrap();
                    pending.store(ms, Ordering::SeqCst);
                }
                if server_w.write_all(&buf[..n]).is_err() {
                    return;
                }
            }
        });
        thread::spawn(move || {
            let mut buf = [0u8; 16 * 1024];
            while let Ok(n @ 1..) = server_r.read(&mut buf) {
                let ms = delay_ms.swap(0, Ordering::SeqCst);
                if ms > 0 {
                    thread::sleep(Duration::from_millis(ms));
                }
                if client_w.write_all(&buf[..n]).is_err() {
                    return;
                }
            }
        });
    }

    fn url(&self) -> String {
        format!("redis://{}", self.addr)
    }

    /// How many requests the proxy has seen containing `needle`.
    fn saw(&self, needle: &[u8]) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.windows(needle.len()).any(|w| w == needle))
            .count()
    }
}

fn test_url() -> Option<String> {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
        return None;
    };
    Some(url)
}

fn upstream_of(url: &str) -> SocketAddr {
    url.strip_prefix("redis://")
        .and_then(|rest| rest.to_socket_addrs().ok()?.next())
        .expect("IX_RELAY_TEST_URL is redis://host:port")
}

fn name(s: &str) -> AgentName {
    if s == "receiver" {
        AgentName::for_session(&"hub".parse().unwrap(), &"s1".parse().unwrap())
    } else if s == "sender" {
        "hub:EA:sender".parse().unwrap()
    } else {
        s.parse().unwrap()
    }
}

/// A bus with `receiver` registered and `count` messages queued for it,
/// reached directly (not through the proxy).
fn seeded(url: &str, prefix: &str, count: usize) -> Relay {
    let mut relay = Relay::connect(url, Duration::from_secs(2))
        .unwrap()
        .with_prefix(prefix);
    relay
        .register(&Registration {
            name: name("receiver"),
            machine: "hub".parse().unwrap(),
            kind: "claude".into(),
            session_id: None,
            cwd: None,
            herdr_pane: None,
            place: None,
            last_seen_ms: 1,
        })
        .unwrap();
    relay
        .commit_identity(&relay_core::IdentityCommit {
            session_name: Some(("s1".parse().unwrap(), name("receiver"))),
            ..relay_core::IdentityCommit::default()
        })
        .unwrap();
    for i in 0..count {
        relay
            .send(&Message {
                from: name("sender"),
                to: Target::Agent(name("receiver")),
                text: format!("message {i}"),
                sent_at_ms: 1,
            })
            .unwrap();
    }
    relay
}

/// Runs a hook for `event` as `receiver` on `url`, in its own config dir;
/// returns its stdout and the records it left in the spool.
fn hook_via(url: &str, prefix: &str, event: &str) -> (String, Vec<serde_json::Value>) {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    let mut cmd = base_cmd(&["hook", "claude"], Some("receiver"));
    cmd.env("IX_RELAY_CONFIG", &config);
    cmd.env("IX_RELAY_URL", url);
    cmd.env("IX_RELAY_PREFIX", prefix);
    let stdin = format!(r#"{{"hook_event_name":"{event}","session_id":"s1","cwd":"/work"}}"#);
    let _topology = support::transport_topology(&mut cmd);
    let r = run(cmd, &stdin);
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);
    let records = std::fs::read_to_string(spool_path(&config))
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    (r.stdout, records)
}

/// `SCRIPT FLUSH`: leaves the server with no script cached, as after a restart.
fn flush_scripts(url: &str) {
    let mut raw = redis::Client::open(url).unwrap().get_connection().unwrap();
    let () = redis::cmd("SCRIPT").arg("FLUSH").query(&mut raw).unwrap();
}

/// Tracing: TC-030-07
/// ACs: FR-030-AC-7
///
/// A slow link (every reply 80 ms late, so the first request the hook makes
/// measures ~80 ms and asks for a ~180 ms window) leaves a `UserPromptSubmit` hook, with its 300 ms
/// budget, too little to pop safely: it defers, silently. Every message stays
/// queued, none is delivered, and nothing is recorded: the mail arrives at the
/// next `PostToolUse` or `Stop`.
#[test]
fn tc_030_07_a_slow_link_makes_a_user_prompt_hook_defer_its_pop_without_a_record() {
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:window-defer:", std::process::id());
    let mut relay = seeded(&url, &prefix, 3);
    let proxy = FaultProxy::start(
        upstream_of(&url),
        vec![Rule {
            needle: b"",
            delay: Duration::from_millis(100),
        }],
    );

    // The link costs 80 ms a request and the hook has 300 ms in all, so a loaded
    // machine can spend the whole budget before the hook reaches the pop check;
    // it then records that (`budget`), which is not the deferral under test. It
    // never pops either way, and is run again.
    let (stdout, records) = (1..=support::OUT_OF_TIME_ATTEMPTS)
        .map(|_| hook_via(&proxy.url(), &prefix, "UserPromptSubmit"))
        .inspect(|(stdout, _)| {
            assert_eq!(proxy.saw(b"inbox:"), 0, "the pop was never sent");
            assert_eq!(stdout, "", "nothing was delivered");
        })
        .find(|(_, records)| !records.iter().any(|r| r["stage"] == "budget"))
        .expect("every hook ran out of time: this machine is too loaded");

    let queued = relay.inbox_len(&name("receiver")).unwrap();
    let delivered = stdout.matches("message ").count();
    assert_eq!(queued + delivered, 3, "stdout: {stdout}");
    assert_eq!(delivered, 0, "the pop was deferred, not attempted late");
    assert_eq!(proxy.saw(b"inbox:"), 0, "the pop was never sent");
    assert!(records.is_empty(), "an expected deferral: {records:?}");
}

/// On a fresh Redis (no script cached) the pop still costs one request: `EVAL`
/// with the body, never `EVALSHA` then `SCRIPT LOAD` then `EVALSHA`. So the
/// window's one-reply assumption holds on the very first call, over a slow link.
#[test]
fn the_first_pop_on_a_fresh_redis_is_one_eval_and_delivers() {
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:window-fresh:", std::process::id());
    let mut relay = seeded(&url, &prefix, 3);
    flush_scripts(&url);
    let proxy = FaultProxy::start(
        upstream_of(&url),
        vec![Rule {
            needle: b"",
            delay: Duration::from_millis(30),
        }],
    );

    // PostToolUse has a 300 ms budget: room for a 30 ms link.
    let (stdout, records) = hook_via(&proxy.url(), &prefix, "PostToolUse");

    assert_eq!(stdout.matches("message ").count(), 3, "stdout: {stdout}");
    assert_eq!(relay.inbox_len(&name("receiver")).unwrap(), 0);
    assert!(records.is_empty(), "{records:?}");
    assert_eq!(proxy.saw(b"EVALSHA"), 0);
    assert_eq!(proxy.saw(b"SCRIPT"), 0);
    assert_eq!(proxy.saw(b"inbox:"), 1, "exactly one pop request");
}

/// The same bus with a generous budget and no delay: the window is not in the
/// way and the messages are delivered. A local round trip costs about a
/// millisecond against `PostToolUse`'s 300 ms.
#[test]
fn a_hook_with_budget_to_spare_pops_and_delivers() {
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:window-ok:", std::process::id());
    let mut relay = seeded(&url, &prefix, 3);
    let proxy = FaultProxy::start(upstream_of(&url), Vec::new());

    let (stdout, records) = hook_via(&proxy.url(), &prefix, "PostToolUse");

    assert_eq!(stdout.matches("message ").count(), 3, "stdout: {stdout}");
    assert_eq!(relay.inbox_len(&name("receiver")).unwrap(), 0);
    assert!(records.is_empty(), "{records:?}");
    assert!(proxy.accepted.load(Ordering::SeqCst) >= 1);
}

/// What the window cannot prevent: a pop whose single reply outlives the read
/// timeout has already taken the messages, and the caller sees only an error.
/// The window makes this unlikely (the hook pops only with the round trip it
/// measured, doubled, to spare); the in-flight record (AGE-2032) is what makes
/// it harmless: nothing is lost, the messages are back once the lease expires.
///
/// Tracing: TC-020-03
/// ACs: FR-020-AC-3
#[test]
fn a_pop_reply_that_outlives_the_read_timeout_loses_nothing() {
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:window-hazard:", std::process::id());
    let mut direct = seeded(&url, &prefix, 3);
    let proxy = FaultProxy::start(
        upstream_of(&url),
        vec![Rule {
            needle: b"inbox:",
            delay: Duration::from_millis(600),
        }],
    );

    let mut through = Relay::connect(&proxy.url(), Duration::from_millis(200))
        .unwrap()
        .with_prefix(prefix);
    let popped = through.pop_inbox_budget(
        &name("receiver"),
        NonZeroUsize::new(50).unwrap(),
        6144,
        Claim::new(Popper::Hook),
    );

    assert!(popped.is_err(), "the reply came after the 200ms timeout");
    assert_eq!(
        direct.inbox_len(&name("receiver")).unwrap(),
        0,
        "and the script had already taken the messages out of the inbox"
    );
    let queued = direct.inflight_len(&name("receiver")).unwrap();
    assert!(queued > 0, "into flight, where the lease still holds them");
    // AGE-2032: the lost reply loses nothing. Once the lease has run out the
    // messages are back at the front of the inbox.
    let reaped = direct
        .reap_expired(&name("receiver"), Now::AtMs(i64::MAX / 2))
        .unwrap();
    assert_eq!(reaped.requeued, queued);
    assert_eq!(direct.inbox_len(&name("receiver")).unwrap(), queued);
}
