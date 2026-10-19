//! Round trips per hook path (AGE-2028), measured, not assumed.
//!
//! A counting TCP proxy sits between the real `agent-msg` binary and a real
//! Redis. A *request* is what the client sends before it waits for a reply, so
//! a pipeline is one request however many commands it holds; the request count
//! is the round-trip count over a real tunnel. Redis-backed: these skip without
//! `IX_RELAY_TEST_URL` (`make test-redis`).
//!
//! Each test pins the count for one path, so adding a sequential read to a hook
//! fails here. The table in the README ("Hook round trips") is these numbers.
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
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use relay_core::{
    AgentName, HerdrPlaceIds, IdentityCommit, Message, Place, Registration, Relay, SessionId,
    Target,
};
use support::{base_cmd, run};

/// One request as the proxy saw it: the command names it carried and its raw bytes.
struct Request {
    commands: Vec<String>,
    raw: Vec<u8>,
}

/// A proxy that forwards every byte and records the requests. Threads end with
/// the test process.
struct CountingProxy {
    addr: SocketAddr,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl CountingProxy {
    fn start(upstream: SocketAddr) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let requests: Arc<Mutex<Vec<Request>>> = Arc::default();
        let seen = Arc::clone(&requests);
        thread::spawn(move || {
            for client in listener.incoming() {
                let Ok(client) = client else { return };
                let Ok(server) = TcpStream::connect(upstream) else {
                    return;
                };
                Self::pipe(client, server, Arc::clone(&seen));
            }
        });
        Self { addr, requests }
    }

    fn pipe(client: TcpStream, server: TcpStream, seen: Arc<Mutex<Vec<Request>>>) {
        // True once the server has answered since the client last wrote: the
        // client's next bytes then start a new request. Until then they continue
        // the current one (a large request arriving in several reads).
        let replied = Arc::new(AtomicBool::new(true));
        let (mut client_r, mut client_w) = (client.try_clone().unwrap(), client);
        let (mut server_r, mut server_w) = (server.try_clone().unwrap(), server);
        let flag = Arc::clone(&replied);
        thread::spawn(move || {
            let mut buf = [0u8; 16 * 1024];
            while let Ok(n @ 1..) = client_r.read(&mut buf) {
                let mut seen = seen.lock().unwrap();
                if flag.swap(false, Ordering::SeqCst) || seen.is_empty() {
                    seen.push(Request {
                        commands: Vec::new(),
                        raw: Vec::new(),
                    });
                }
                let request = seen.last_mut().unwrap();
                request.raw.extend_from_slice(&buf[..n]);
                request.commands.extend(command_names(&buf[..n]));
                drop(seen);
                if server_w.write_all(&buf[..n]).is_err() {
                    return;
                }
            }
        });
        thread::spawn(move || {
            let mut buf = [0u8; 16 * 1024];
            while let Ok(n @ 1..) = server_r.read(&mut buf) {
                replied.store(true, Ordering::SeqCst);
                if client_w.write_all(&buf[..n]).is_err() {
                    return;
                }
            }
        });
    }

    fn url(&self) -> String {
        format!("redis://{}", self.addr)
    }

    /// Forgets everything seen so far (the seeding), keeping later requests.
    fn reset(&self) {
        self.requests.lock().unwrap().clear();
    }

    /// The commands of each request, in order, e.g. `[["EXISTS", "MGET"], ["EVAL"]]`.
    fn commands(&self) -> Vec<Vec<String>> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.commands.clone())
            .collect()
    }

    /// Whether any request before the pop read the whole registry.
    fn read_whole_registry_before_pop(&self) -> bool {
        self.commands()
            .iter()
            .take(self.before_pop())
            .flatten()
            .any(|c| c == "HGETALL")
    }

    /// How many requests came before the pop (the one that names an inbox key),
    /// or all of them when there was no pop.
    fn before_pop(&self) -> usize {
        let requests = self.requests.lock().unwrap();
        requests
            .iter()
            .position(|r| contains(&r.raw, b"inbox:"))
            .unwrap_or(requests.len())
    }

    fn total(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// The command name of every RESP array (`*N`, `$len`, name) in `chunk`.
fn command_names(chunk: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(chunk);
    let mut names = Vec::new();
    let mut lines = text.split("\r\n");
    while let Some(line) = lines.next() {
        if line.starts_with('*') {
            let _len = lines.next();
            if let Some(name) = lines.next() {
                names.push(name.to_ascii_uppercase());
            }
        }
    }
    names
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

const PANE: &str = "w1:p1";

/// A bus with `receiver` registered in pane [`PANE`] on `hub`, its session `s1`
/// cached, and `queued` messages waiting.
fn seeded(url: &str, prefix: &str, queued: usize) -> Relay {
    let mut relay = Relay::connect(url, std::time::Duration::from_secs(2))
        .unwrap()
        .with_prefix(prefix);
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let session_id: SessionId = "s1".parse().unwrap();
    let registration = Registration {
        name: name("receiver"),
        machine: "hub".parse().unwrap(),
        kind: "claude".into(),
        session_id: Some(session_id.clone()),
        cwd: Some("/work".into()),
        herdr_pane: Some(PANE.parse().unwrap()),
        place: Some(Place {
            workspace: "EA".into(),
            tab: "code".into(),
            herdr_ids: Some(HerdrPlaceIds {
                workspace: "workspace-id".parse().unwrap(),
                tab: "tab-id".parse().unwrap(),
                pane: PANE.parse().unwrap(),
                session_id: session_id.clone(),
                verified_at_ms: now_ms,
            }),
        }),
        last_seen_ms: now_ms,
    };
    relay
        .commit_identity(&IdentityCommit {
            registration: Some(registration),
            session_name: Some((session_id, name("receiver"))),
            ..IdentityCommit::default()
        })
        .unwrap();
    for i in 0..queued {
        relay
            .send(&Message {
                from: name("sender"),
                to: Target::Agent(name("receiver")),
                text: format!("message {i}"),
                sent_at_ms: 1,
                forwarded: false,
            })
            .unwrap();
    }
    relay
}

/// Runs one hook of `event` through `proxy` as a lead in [`PANE`], explicitly
/// named or derived from its session cache; returns stdout and stderr.
fn hook(proxy: &CountingProxy, prefix: &str, event: &str, explicit: bool) -> (String, String) {
    let mut cmd = base_cmd(&["hook", "claude"], explicit.then_some("receiver"));
    cmd.env("IX_RELAY_URL", proxy.url());
    cmd.env("IX_RELAY_PREFIX", prefix);
    cmd.env("IX_RELAY_MACHINE", "hub");
    cmd.env("HERDR_PANE_ID", PANE);
    let stdin = format!(r#"{{"hook_event_name":"{event}","session_id":"s1","cwd":"/work"}}"#);
    let _topology = support::transport_topology(&mut cmd);
    let r = run(cmd, &stdin);
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);
    (r.stdout, r.stderr)
}

/// One measured hook: `(requests before the pop, all requests, stdout)`.
fn measure(
    name: &str,
    event: &str,
    explicit: bool,
    queued: usize,
) -> Option<(usize, usize, String)> {
    let url = test_url()?;
    // The counts below are those of a hook that ran to the end. A `UserPromptSubmit`
    // has 300 ms for six requests, so on a loaded machine one can run out of time
    // first: it then defers its pop, or records why on stderr, and its count says
    // nothing about the path. Such a run is repeated, on a fresh bus.
    for attempt in 1..=support::OUT_OF_TIME_ATTEMPTS {
        let prefix = format!("test:{}:rt-{name}-{attempt}:", std::process::id());
        let _relay = seeded(&url, &prefix, queued);
        let proxy = CountingProxy::start(upstream_of(&url));
        proxy.reset();
        let (stdout, stderr) = hook(&proxy, &prefix, event, explicit);
        eprintln!("{name}: {:?}", proxy.commands());
        // A run that ran out of time either says so (the budget error or a Redis
        // read timeout on stderr) or, for a `UserPromptSubmit`, defers its pop in
        // silence; anything else on stderr is a real failure.
        let out_of_time = stderr.contains("time budget exhausted") || stderr.contains("timed out");
        let deferred = stderr.is_empty() && queued > 0 && !stdout.contains("message 0");
        if out_of_time || deferred {
            eprintln!("{name}: attempt {attempt} ran out of time, trying again: {stderr}");
            continue;
        }
        assert!(stderr.is_empty(), "{name}: stderr: {stderr}");
        assert!(
            !proxy.read_whole_registry_before_pop(),
            "{name} read the whole registry before the pop: {:?}",
            proxy.commands()
        );
        return Some((proxy.before_pop(), proxy.total(), stdout));
    }
    panic!(
        "{name}: {} hooks in a row ran out of time: this machine is too loaded",
        support::OUT_OF_TIME_ATTEMPTS
    );
}

/// Tracing: TC-018-05
/// ACs: FR-018-AC-5
///
/// Connecting sends nothing: no `CLIENT SETINFO` handshake, so the first
/// request on a fresh connection is the caller's own.
#[test]
fn tc_018_05_connecting_sends_no_client_setinfo() {
    let Some(url) = test_url() else { return };
    let proxy = CountingProxy::start(upstream_of(&url));
    let mut relay = Relay::connect(&proxy.url(), std::time::Duration::from_secs(2))
        .unwrap()
        .with_prefix(format!("test:{}:rt-connect:", std::process::id()));
    assert_eq!(proxy.total(), 0, "connecting sent {:?}", proxy.commands());
    relay.inbox_len(&name("receiver")).unwrap();
    assert_eq!(proxy.commands(), vec![vec!["LLEN".to_owned()]]);
}

/// Tracing: TC-018-06
/// ACs: FR-018-AC-6
///
/// A derived-name `UserPromptSubmit` reaches the pop in three requests: one
/// pipelined read (suppressed mark and session cache), one point read of the
/// lead's registration, one commit. It used to take five.
#[test]
fn tc_018_06_a_derived_name_user_prompt_submit_reaches_the_pop_in_three_requests() {
    let Some((before, total, stdout)) = measure("ups-derived", "UserPromptSubmit", false, 1) else {
        return;
    };
    assert!(stdout.contains("message 0"), "stdout: {stdout}");
    assert_eq!(before, 1, "cached genuine session lookup before the pop");
    assert_eq!(
        total, 4,
        "requests in all: the pop, the address book and (AGE-2032) the acknowledgement follow"
    );
}

/// Tracing: TC-018-06
/// ACs: FR-018-AC-6
///
/// An explicitly named `UserPromptSubmit` in a pane reaches the pop in two
/// requests: the registration's place, then the commit.
#[test]
fn tc_018_06_an_explicit_name_user_prompt_submit_reaches_the_pop_in_two_requests() {
    let Some((before, total, stdout)) = measure("ups-explicit", "UserPromptSubmit", true, 1) else {
        return;
    };
    assert!(stdout.contains("message 0"), "stdout: {stdout}");
    assert_eq!(before, 1, "cached genuine session lookup before the pop");
    assert_eq!(
        total, 4,
        "requests in all: the pop, the address book and (AGE-2032) the acknowledgement follow"
    );
}

/// Tracing: TC-018-07
/// ACs: FR-018-AC-7
///
/// `PostToolUse` with a cached session name: one pipelined read, then the pop
/// (and, when something was popped, the address book and the acknowledgement).
/// Explicit name: the pop alone.
#[test]
fn tc_018_07_post_tool_use_costs_one_read_then_the_pop() {
    let Some((before, _, _)) = measure("ptu-derived", "PostToolUse", false, 0) else {
        return;
    };
    assert_eq!(before, 1, "derived, empty inbox");
    let Some((before, total, _)) = measure("ptu-explicit", "PostToolUse", true, 0) else {
        return;
    };
    assert_eq!(
        (before, total),
        (1, 2),
        "environment metadata does not bypass session lookup"
    );
}
