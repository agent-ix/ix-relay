//! Read-only commands ride out one timed-out read; `send` never retries (AGE-2028).
//!
//! A TCP proxy in front of a real Redis swallows every request on the first
//! connection it accepts, so that connection's read times out, and forwards
//! every later connection untouched. Redis-backed: these skip without
//! `IX_RELAY_TEST_URL` (`make test-redis`). Each waits out the CLI's 3 s read
//! timeout once, so they run in parallel and take a few seconds.
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: a failed unwrap/expect is the failure signal"
)]

mod support;

use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use relay_core::{AgentName, Message, Registration, Relay, Target};
use support::{base_cmd, run};

/// What the proxy swallows: a request it never forwards, and never answers.
#[derive(Clone, Copy)]
enum Swallow {
    /// Every request on the first connection accepted.
    FirstConnection,
    /// Every request on every connection: a Redis that accepts and never answers.
    Everything,
    /// Nothing is dropped, but the reply that follows a request containing
    /// these bytes is held back past the CLI's 3 s read timeout, so the request
    /// takes effect and the client sees a timeout.
    SlowReplyTo(&'static [u8]),
}

/// The proxy: how many connections it accepted, and what it swallows.
struct Proxy {
    addr: SocketAddr,
    accepted: Arc<AtomicUsize>,
}

impl Proxy {
    fn start(upstream: SocketAddr, swallow: Swallow) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let accepted = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&accepted);
        thread::spawn(move || {
            for client in listener.incoming().flatten() {
                let first = count.fetch_add(1, Ordering::SeqCst) == 0;
                thread::spawn(move || {
                    let Ok(server) = TcpStream::connect(upstream) else {
                        return;
                    };
                    forward(client, server, swallow, first);
                });
            }
        });
        Self { addr, accepted }
    }

    fn url(&self) -> String {
        format!("redis://{}", self.addr)
    }

    fn accepted(&self) -> usize {
        self.accepted.load(Ordering::SeqCst)
    }
}

/// Copies both directions until either side closes, dropping what `swallow`
/// names (and everything after it, as a blackholed link does) or holding a reply.
fn forward(client: TcpStream, server: TcpStream, swallow: Swallow, first: bool) {
    let (mut client_r, mut client_w) = (client.try_clone().unwrap(), client);
    let (mut server_r, mut server_w) = (server.try_clone().unwrap(), server);
    let hold = Arc::new(AtomicBool::new(false));
    let held = Arc::clone(&hold);
    thread::spawn(move || {
        let mut buf = [0u8; 16 * 1024];
        let mut blackholed = false;
        while let Ok(n @ 1..) = client_r.read(&mut buf) {
            let chunk = &buf[..n];
            blackholed |= match swallow {
                Swallow::FirstConnection => first,
                Swallow::Everything => true,
                Swallow::SlowReplyTo(needle) => {
                    if chunk.windows(needle.len()).any(|w| w == needle) {
                        held.store(true, Ordering::SeqCst);
                    }
                    false
                }
            };
            if !blackholed && server_w.write_all(chunk).is_err() {
                return;
            }
        }
    });
    let mut buf = [0u8; 16 * 1024];
    while let Ok(n @ 1..) = server_r.read(&mut buf) {
        if hold.swap(false, Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(3500));
        }
        if client_w.write_all(&buf[..n]).is_err() {
            return;
        }
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
    s.parse().unwrap()
}

/// A bus with `receiver` registered, reached directly.
fn seeded(url: &str, prefix: &str) -> Relay {
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
}

/// Runs `agent-msg <args>` through `proxy` as `sender`.
fn agent_msg(proxy: &Proxy, prefix: &str, args: &[&str]) -> support::Run {
    let mut cmd = base_cmd(args, Some("sender"));
    cmd.env("IX_RELAY_URL", proxy.url());
    cmd.env("IX_RELAY_PREFIX", prefix);
    cmd.env("IX_RELAY_MACHINE", "hub");
    run(cmd, "")
}

/// Tracing: TC-039-02
/// ACs: FR-039-AC-2
///
/// `who` and `log` survive a first connection that never answers: the read
/// times out, they redial once and print the answer.
#[test]
fn tc_039_02_read_only_commands_retry_a_timed_out_read_on_a_fresh_connection() {
    let Some(url) = test_url() else { return };
    let mut relay = seeded(&url, &format!("test:{}:retry-read:", std::process::id()));
    relay
        .send(&Message {
            from: name("sender"),
            to: Target::Agent(name("receiver")),
            text: "hello".into(),
            sent_at_ms: 1,
        })
        .unwrap();

    for args in [&["who"][..], &["log", "--since", "1000000d"][..]] {
        let proxy = Proxy::start(upstream_of(&url), Swallow::FirstConnection);
        let prefix = format!("test:{}:retry-read:", std::process::id());
        let r = agent_msg(&proxy, &prefix, args);
        assert_eq!(r.status, 0, "{args:?}: stderr: {}", r.stderr);
        let expect = if args[0] == "who" {
            "receiver"
        } else {
            "hello"
        };
        assert!(r.stdout.contains(expect), "{args:?}: {}", r.stdout);
        assert_eq!(
            proxy.accepted(),
            2,
            "{args:?}: one retry, on a new connection"
        );
    }
}

/// Tracing: TC-039-03
/// ACs: FR-039-AC-3
///
/// `send` is not idempotent. The proxy holds back the reply to the send
/// transaction (the request holding the `XADD`) past the 3 s read timeout, so the
/// name resolves, the transaction *takes effect*, and `send` reports a timeout.
/// It must not dial again: a retry would queue the message a second time. A
/// `send` wrapped in the read retry does exactly that here, so this fails for it
/// (checked by mutation).
#[test]
fn tc_039_03_send_is_never_retried() {
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:retry-send:", std::process::id());
    let mut relay = seeded(&url, &prefix);
    let proxy = Proxy::start(upstream_of(&url), Swallow::SlowReplyTo(b"XADD"));
    let r = agent_msg(&proxy, &prefix, &["send", "receiver", "once"]);
    assert_ne!(r.status, 0, "the timed-out send is reported, not retried");
    assert_eq!(proxy.accepted(), 1, "no second connection was opened");
    assert_eq!(
        relay.inbox_len(&name("receiver")).unwrap(),
        1,
        "the send took effect once and was not repeated"
    );
}

/// Tracing: TC-039-04
/// ACs: FR-039-AC-4
///
/// A Redis that accepts and never answers fails every read-only command that
/// retries in about one 5 s deadline, not one per read: `who`, `whoami` and `log`
/// (which reads twice). `inbox` does not retry its pop, so it fails at the CLI's
/// 3 s. Wall-clock bounds: this is the `#[ignore]`d lane
/// (`cargo test -p agent-msg --test read_retry -- --ignored`).
#[test]
#[ignore = "benchmark lane: wall-clock bound, each case waits out a real deadline"]
fn tc_039_04_a_hard_down_redis_fails_read_only_commands_within_the_deadline() {
    let Some(url) = test_url() else { return };
    let prefix = format!("test:{}:retry-down:", std::process::id());
    let _relay = seeded(&url, &prefix);
    let cases: [(&[&str], f64); 4] = [
        (&["who"], 6.0),
        (&["log"], 6.0),
        (&["whoami"], 6.0),
        (&["inbox", "--json"], 3.8),
    ];
    thread::scope(|scope| {
        for (args, bound) in cases {
            let (url, prefix) = (url.clone(), prefix.clone());
            scope.spawn(move || {
                let proxy = Proxy::start(upstream_of(&url), Swallow::Everything);
                let started = std::time::Instant::now();
                let r = agent_msg(&proxy, &prefix, args);
                let took = started.elapsed().as_secs_f64();
                eprintln!("{args:?} failed in {took:.2}s");
                // `whoami` with a given name only loses its address column.
                if args[0] != "whoami" {
                    assert_ne!(r.status, 0, "{args:?}");
                }
                assert!(took < bound, "{args:?} took {took:.1}s, bound {bound}s");
            });
        }
    });
}
