//! A small HTTP/1.1 client for one POST, over `std::net::TcpStream`.
//!
//! It exists so the exporter needs no HTTP or TLS dependency, and so its limits
//! are visible: plain `http://` only, one request per connection
//! (`Connection: close`), a connect timeout, a total deadline for the whole
//! exchange (not just per read, so a collector that drips a byte at a time cannot
//! hold the thread), a response cap, and redirects are never followed (a 3xx is
//! returned to the caller as a status).

use std::{
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    time::{Duration, Instant},
};

use relay_core::config::otlp::{OtlpEndpoint, OtlpHeaders};

/// The longest a connect may take.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
/// The longest the whole request, from connect to the last byte read, may take.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// The most response bytes read; the rest is ignored.
pub const MAX_RESPONSE_BYTES: usize = 64 * 1024;
/// The most addresses one host name is tried on.
const MAX_ADDRS: usize = 4;
/// How long resolved collector addresses are reused before the name is looked up
/// again. A slow resolver (one that waits out a missing AAAA answer, say) is paid at
/// most once per lifetime, not on every push; a collector that moves is found within
/// it, or sooner when every cached address refuses a connection.
pub const ADDR_CACHE_TTL: Duration = Duration::from_secs(300);

/// How a request failed before there was a response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportError {
    /// The name did not resolve, or no address accepted the connection.
    Connect,
    /// The connect or the whole request ran out of time.
    Timeout,
    /// The connection failed or closed mid-exchange.
    Io,
    /// Bytes came back that are not an HTTP response.
    BadResponse,
}

impl TransportError {
    /// A short stable code, for the heartbeat and logs.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Connect => "connect_failed",
            Self::Timeout => "timeout",
            Self::Io => "io_error",
            Self::BadResponse => "bad_response",
        }
    }
}

/// What the collector answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// The status code.
    pub status: u16,
    /// The `Retry-After` header as seconds, when it is a whole number.
    pub retry_after_secs: Option<u64>,
    /// The body, up to [`MAX_RESPONSE_BYTES`].
    pub body: Vec<u8>,
}

/// The time limits of a request. [`Timeouts::default`] is the production set.
#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    /// Per address.
    pub connect: Duration,
    /// For the whole request.
    pub request: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            connect: CONNECT_TIMEOUT,
            request: REQUEST_TIMEOUT,
        }
    }
}

/// Where a request can be sent: what the exporter's sender posts through, so a
/// test can stand in for the network.
pub trait Transport: Send {
    /// POSTs `body` (JSON) and returns the collector's answer.
    ///
    /// # Errors
    ///
    /// A [`TransportError`] when there was no answer.
    fn post(&mut self, body: &[u8]) -> Result<Response, TransportError>;
}

/// Turns a collector host name into addresses: the seam that lets a test count
/// lookups and fail them without a network.
pub trait Resolve: Send {
    /// The addresses of `host` at `port`.
    ///
    /// # Errors
    ///
    /// The lookup's I/O error when the name did not resolve.
    fn resolve(&self, host: &str, port: u16) -> io::Result<Vec<SocketAddr>>;
}

/// The system resolver (`getaddrinfo`).
struct SystemResolver;

impl Resolve for SystemResolver {
    fn resolve(&self, host: &str, port: u16) -> io::Result<Vec<SocketAddr>> {
        Ok((host, port).to_socket_addrs()?.collect())
    }
}

/// Resolved addresses and when they were looked up.
struct CachedAddrs {
    addrs: Vec<SocketAddr>,
    resolved_at: Instant,
}

/// The real transport: one TCP connection per request, to addresses resolved at most
/// once per [`ADDR_CACHE_TTL`].
pub struct HttpTransport {
    endpoint: OtlpEndpoint,
    headers: OtlpHeaders,
    timeouts: Timeouts,
    user_agent: String,
    resolver: Box<dyn Resolve>,
    clock: Box<dyn Fn() -> Instant + Send>,
    cache: Option<CachedAddrs>,
}

impl HttpTransport {
    /// A transport to `endpoint` sending `headers` on every request.
    #[must_use]
    pub fn new(
        endpoint: OtlpEndpoint,
        headers: OtlpHeaders,
        timeouts: Timeouts,
        user_agent: String,
    ) -> Self {
        Self::with_resolver(
            endpoint,
            headers,
            timeouts,
            user_agent,
            Box::new(SystemResolver),
            Box::new(Instant::now),
        )
    }

    /// [`HttpTransport::new`] with the resolver and the clock that ages its cache
    /// supplied.
    #[must_use]
    pub fn with_resolver(
        endpoint: OtlpEndpoint,
        headers: OtlpHeaders,
        timeouts: Timeouts,
        user_agent: String,
        resolver: Box<dyn Resolve>,
        clock: Box<dyn Fn() -> Instant + Send>,
    ) -> Self {
        Self {
            endpoint,
            headers,
            timeouts,
            user_agent,
            resolver,
            clock,
            cache: None,
        }
    }

    /// The collector's addresses, at most [`MAX_ADDRS`], in the order resolved: the
    /// cached ones while they are younger than [`ADDR_CACHE_TTL`], else a fresh
    /// lookup. A failed or empty lookup is never cached; when one fails at expiry the
    /// expired addresses are used again (and the next push looks up again), so a
    /// name-server blip does not fail pushes.
    fn addrs(&mut self) -> Result<Vec<SocketAddr>, TransportError> {
        let now = (self.clock)();
        if let Some(cached) = &self.cache
            && now.saturating_duration_since(cached.resolved_at) < ADDR_CACHE_TTL
        {
            return Ok(cached.addrs.clone());
        }
        match self
            .resolver
            .resolve(self.endpoint.host(), self.endpoint.port())
        {
            Ok(found) if !found.is_empty() => {
                let addrs: Vec<SocketAddr> = found.into_iter().take(MAX_ADDRS).collect();
                self.cache = Some(CachedAddrs {
                    addrs: addrs.clone(),
                    resolved_at: now,
                });
                Ok(addrs)
            }
            _ => self
                .cache
                .as_ref()
                .map(|stale| stale.addrs.clone())
                .ok_or(TransportError::Connect),
        }
    }

    /// Connects to the first of `addrs` that accepts. When every address was tried and
    /// failed, the cache is dropped so the next push looks the name up again; when the
    /// deadline ends the attempt before every address was tried, the cache is kept.
    fn connect(
        &mut self,
        addrs: &[SocketAddr],
        deadline: Instant,
    ) -> Result<TcpStream, TransportError> {
        let mut last = TransportError::Connect;
        for addr in addrs {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(TransportError::Timeout);
            }
            match TcpStream::connect_timeout(addr, self.timeouts.connect.min(left)) {
                Ok(stream) => return Ok(stream),
                Err(e) if e.kind() == io::ErrorKind::TimedOut => last = TransportError::Timeout,
                Err(_) => {}
            }
        }
        self.cache = None;
        Err(last)
    }

    fn request_head(&self, len: usize) -> String {
        let host = if self.endpoint.port() == 80 {
            self.endpoint.host().to_owned()
        } else {
            self.endpoint.target()
        };
        let host = if self.endpoint.host().contains(':') && self.endpoint.port() == 80 {
            format!("[{}]", self.endpoint.host())
        } else {
            host
        };
        let mut head = format!(
            "POST {} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: {}\r\nAccept: application/json\r\n\
             Content-Type: application/json\r\nContent-Length: {len}\r\nConnection: close\r\n",
            self.endpoint.path(),
            self.user_agent,
        );
        for (name, value) in self.headers.pairs() {
            head.push_str(name);
            head.push_str(": ");
            head.push_str(value);
            head.push_str("\r\n");
        }
        head.push_str("\r\n");
        head
    }
}

fn map_io(e: &std::io::Error) -> TransportError {
    match e.kind() {
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => TransportError::Timeout,
        _ => TransportError::Io,
    }
}

impl Transport for HttpTransport {
    fn post(&mut self, body: &[u8]) -> Result<Response, TransportError> {
        // The lookup is not part of the request budget: a slow one is paid on this
        // exporter thread, and the request then has its full time.
        let addrs = self.addrs()?;
        let now = Instant::now();
        let deadline = now.checked_add(self.timeouts.request).unwrap_or(now);
        let mut stream = self.connect(&addrs, deadline)?;
        let head = self.request_head(body.len());
        write_all(&mut stream, head.as_bytes(), deadline)?;
        write_all(&mut stream, body, deadline)?;
        read_response(&mut stream, deadline)
    }
}

fn write_all(
    stream: &mut TcpStream,
    mut bytes: &[u8],
    deadline: Instant,
) -> Result<(), TransportError> {
    while !bytes.is_empty() {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(TransportError::Timeout);
        }
        stream
            .set_write_timeout(Some(left))
            .map_err(|_| TransportError::Io)?;
        match stream.write(bytes) {
            Ok(0) => return Err(TransportError::Io),
            Ok(n) => bytes = bytes.get(n..).unwrap_or(&[]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(map_io(&e)),
        }
    }
    Ok(())
}

fn read_response(stream: &mut TcpStream, deadline: Instant) -> Result<Response, TransportError> {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        if response_complete(&buf) {
            break;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(TransportError::Timeout);
        }
        stream
            .set_read_timeout(Some(left))
            .map_err(|_| TransportError::Io)?;
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(chunk.get(..n).unwrap_or(&[]));
                if buf.len() >= MAX_RESPONSE_BYTES {
                    break;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(map_io(&e)),
        }
    }
    parse_response(&buf)
}

/// The end of the head, if it has arrived.
fn head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

/// The value of header `name` (lower case) in `head`, trimmed.
fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().skip(1).find_map(|line| {
        let (k, v) = line.split_once(':')?;
        k.trim().eq_ignore_ascii_case(name).then(|| v.trim())
    })
}

/// Whether the whole response is in `buf`, so the read need not wait for the
/// collector to close a connection it may keep open.
fn response_complete(buf: &[u8]) -> bool {
    let Some(end) = head_end(buf) else {
        return false;
    };
    let head = String::from_utf8_lossy(buf.get(..end).unwrap_or(&[]));
    let body = buf.get(end..).unwrap_or(&[]);
    if header(&head, "transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked")) {
        return body.ends_with(b"0\r\n\r\n");
    }
    header(&head, "content-length")
        .and_then(|v| v.parse::<usize>().ok())
        .is_some_and(|len| body.len() >= len)
}

fn parse_response(buf: &[u8]) -> Result<Response, TransportError> {
    let end = head_end(buf).ok_or(TransportError::BadResponse)?;
    let head = String::from_utf8_lossy(buf.get(..end).unwrap_or(&[])).into_owned();
    let status_line = head.lines().next().ok_or(TransportError::BadResponse)?;
    let mut parts = status_line.split_whitespace();
    let version = parts.next().ok_or(TransportError::BadResponse)?;
    if !version.starts_with("HTTP/1.") {
        return Err(TransportError::BadResponse);
    }
    let status = parts
        .next()
        .and_then(|s| s.parse::<u16>().ok())
        .filter(|s| (100..=599).contains(s))
        .ok_or(TransportError::BadResponse)?;
    let raw_body = buf.get(end..).unwrap_or(&[]);
    let body =
        if header(&head, "transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked")) {
            dechunk(raw_body)
        } else {
            raw_body.to_vec()
        };
    Ok(Response {
        status,
        retry_after_secs: header(&head, "retry-after").and_then(retry_after_secs),
        body,
    })
}

/// A `Retry-After` in whole seconds. A run of digits too large for `u64` is a
/// request to wait as long as anyone may ask, not the absence of one, so it reads as
/// `u64::MAX` (the sender bounds it). An HTTP date or anything else is `None`.
fn retry_after_secs(value: &str) -> Option<u64> {
    let value = value.trim();
    (!value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()))
        .then(|| value.parse().unwrap_or(u64::MAX))
}

/// The payload of a chunked body, as far as it decodes.
fn dechunk(mut raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(line_end) = raw.windows(2).position(|w| w == b"\r\n") {
        let size = std::str::from_utf8(raw.get(..line_end).unwrap_or(&[]))
            .ok()
            .and_then(|s| usize::from_str_radix(s.split(';').next().unwrap_or("").trim(), 16).ok());
        let Some(size) = size.filter(|s| *s > 0) else {
            break;
        };
        let start = line_end + 2;
        let Some(chunk) = raw.get(start..start.saturating_add(size)) else {
            break;
        };
        out.extend_from_slice(chunk);
        raw = raw.get(start + size + 2..).unwrap_or(&[]);
    }
    out
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;

    use super::*;
    use crate::otlp::test_collector::{FakeCollector, Reply, read_request};

    fn transport(url: &str, headers: &str, timeouts: Timeouts) -> HttpTransport {
        HttpTransport::new(
            OtlpEndpoint::parse(url, true).unwrap(),
            OtlpHeaders::parse(headers).unwrap(),
            timeouts,
            "ua/1".into(),
        )
    }

    /// Tracing: TC-045-10
    /// ACs: FR-045-AC-10
    #[test]
    fn tc_045_10_a_post_carries_the_json_content_type_the_path_and_the_headers() {
        let collector = FakeCollector::start(vec![Reply::json(200, "{}")]);
        let mut t = transport(
            &collector.url(),
            "Authorization=Bearer%20x",
            Timeouts::default(),
        );
        let response = t.post(b"{\"a\":1}").unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"{}");
        let seen = collector.recv();
        assert!(
            seen.head.starts_with("POST /v1/metrics HTTP/1.1\r\n"),
            "{}",
            seen.head
        );
        assert!(
            seen.head.contains("\r\nContent-Type: application/json\r\n"),
            "{}",
            seen.head
        );
        assert!(
            seen.head.contains("\r\nContent-Length: 7\r\n"),
            "{}",
            seen.head
        );
        assert!(
            seen.head.contains("\r\nAuthorization: Bearer x\r\n"),
            "{}",
            seen.head
        );
        assert!(seen.head.contains("\r\nHost: 127.0.0.1:"), "{}", seen.head);
        assert_eq!(seen.body, b"{\"a\":1}");
    }

    /// Tracing: TC-045-10
    /// ACs: FR-045-AC-10
    #[test]
    fn tc_045_10_a_response_is_read_by_length_chunks_or_close_and_a_redirect_is_a_status() {
        for (raw, status, retry, body) in [
            (&b"HTTP/1.1 429 Too Many\r\nRetry-After: 7\r\nContent-Length: 0\r\n\r\n"[..], 429, Some(7), &b""[..]),
            (b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\n\r\n", 200, None, b"{}"),
            (b"HTTP/1.0 200 OK\r\n\r\nabc", 200, None, b"abc"),
            (b"HTTP/1.1 301 Moved\r\nLocation: http://elsewhere/\r\nContent-Length: 0\r\n\r\n", 301, None, b""),
            (b"HTTP/1.1 503 X\r\nRetry-After: Wed, 21 Oct 2026 07:28:00 GMT\r\nContent-Length: 0\r\n\r\n", 503, None, b""),
        ] {
            let collector = FakeCollector::start(vec![Reply::Raw(raw.to_vec())]);
            let mut t = transport(&collector.url(), "", Timeouts::default());
            let r = t.post(b"{}").unwrap();
            assert_eq!((r.status, r.retry_after_secs, r.body.as_slice()), (status, retry, body));
        }
    }

    /// Garbage is a bad response; a host that does not resolve (`.invalid` never
    /// does) is a connect failure; neither panics.
    ///
    /// Tracing: TC-045-10
    /// ACs: FR-045-AC-10
    #[test]
    fn tc_045_10_garbage_and_an_unresolvable_host_are_errors_not_panics() {
        let collector =
            FakeCollector::start(vec![Reply::Raw(b"\x00\x01 not http\r\n\r\n".to_vec())]);
        let mut t = transport(&collector.url(), "", Timeouts::default());
        assert_eq!(t.post(b"{}"), Err(TransportError::BadResponse));
        let mut t = transport("http://collector.invalid:4318", "", Timeouts::default());
        assert_eq!(t.post(b"{}"), Err(TransportError::Connect));
        // A connection closed before any answer is an I/O error.
        let collector = FakeCollector::start(vec![Reply::Raw(Vec::new())]);
        let mut t = transport(&collector.url(), "", Timeouts::default());
        assert_eq!(t.post(b"{}"), Err(TransportError::BadResponse));
    }

    /// A collector that accepts and never answers, and one that drips a byte at a
    /// time, are cut at the request deadline, not held for ever.
    ///
    /// Tracing: TC-045-10
    /// ACs: FR-045-AC-10
    #[test]
    fn tc_045_10_a_silent_or_dripping_collector_is_cut_at_the_request_deadline() {
        let short = Timeouts {
            connect: Duration::from_millis(200),
            request: Duration::from_millis(300),
        };
        let collector = FakeCollector::start(vec![Reply::Silent]);
        let mut t = transport(&collector.url(), "", short);
        assert_eq!(t.post(b"{}"), Err(TransportError::Timeout));

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let mut t = transport(&url, "", short);
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let _ = read_request(&mut s);
            // One byte every 50 ms: each read succeeds, so only the total
            // deadline can stop the client.
            while done_rx.try_recv().is_err() {
                if s.write_all(b"H").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        assert_eq!(t.post(b"{}"), Err(TransportError::Timeout));
        done_tx.send(()).unwrap();
        server.join().unwrap();
    }

    #[test]
    fn a_response_over_the_cap_is_read_only_that_far() {
        let mut big = b"HTTP/1.1 200 OK\r\n\r\n".to_vec();
        big.extend(std::iter::repeat_n(b'x', MAX_RESPONSE_BYTES * 4));
        let collector = FakeCollector::start(vec![Reply::Raw(big)]);
        let mut t = transport(&collector.url(), "", Timeouts::default());
        let r = t.post(b"{}").unwrap();
        assert!(
            r.body.len() <= MAX_RESPONSE_BYTES + 4096,
            "{}",
            r.body.len()
        );
    }

    /// A `Retry-After` that overflows `u64` is a very long wait, not no wait; an HTTP
    /// date or junk is none.
    ///
    /// Tracing: TC-045-17
    /// ACs: FR-045-AC-17
    #[test]
    fn tc_045_17_a_retry_after_too_large_for_u64_reads_as_the_largest_wait() {
        assert_eq!(retry_after_secs("7"), Some(7));
        assert_eq!(retry_after_secs(" 30 "), Some(30));
        assert_eq!(retry_after_secs("99999999999999999999"), Some(u64::MAX));
        assert_eq!(retry_after_secs("18446744073709551615"), Some(u64::MAX));
        for none in ["", "-5", "1.5", "Wed, 21 Oct 2026 07:28:00 GMT", "7s", "+7"] {
            assert_eq!(retry_after_secs(none), None, "{none:?}");
        }
        let collector = FakeCollector::start(vec![Reply::with_header(
            429,
            "Retry-After: 99999999999999999999",
        )]);
        let mut t = transport(&collector.url(), "", Timeouts::default());
        assert_eq!(t.post(b"{}").unwrap().retry_after_secs, Some(u64::MAX));
    }

    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    };

    /// A resolver that counts its lookups and answers from a script: each call takes
    /// the next reply, the last one repeating.
    struct FakeResolver {
        calls: Arc<AtomicUsize>,
        replies: Mutex<Vec<io::Result<Vec<SocketAddr>>>>,
    }

    impl Resolve for FakeResolver {
        fn resolve(&self, _host: &str, _port: u16) -> io::Result<Vec<SocketAddr>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut replies = self.replies.lock().unwrap();
            if replies.len() > 1 {
                return replies.remove(0);
            }
            match replies.first().unwrap() {
                Ok(a) => Ok(a.clone()),
                Err(e) => Err(io::Error::new(e.kind(), e.to_string())),
            }
        }
    }

    struct Harness {
        transport: HttpTransport,
        calls: Arc<AtomicUsize>,
        advance_secs: Arc<AtomicU64>,
    }

    fn harness(replies: Vec<io::Result<Vec<SocketAddr>>>) -> Harness {
        harness_with(replies, Timeouts::default())
    }

    fn harness_with(replies: Vec<io::Result<Vec<SocketAddr>>>, timeouts: Timeouts) -> Harness {
        let calls = Arc::new(AtomicUsize::new(0));
        let advance_secs = Arc::new(AtomicU64::new(0));
        let base = Instant::now();
        let offset = Arc::clone(&advance_secs);
        let transport = HttpTransport::with_resolver(
            OtlpEndpoint::parse("http://collector.test:4318", true).unwrap(),
            OtlpHeaders::parse("").unwrap(),
            timeouts,
            "ua/1".into(),
            Box::new(FakeResolver {
                calls: Arc::clone(&calls),
                replies: Mutex::new(replies),
            }),
            Box::new(move || base + Duration::from_secs(offset.load(Ordering::SeqCst))),
        );
        Harness {
            transport,
            calls,
            advance_secs,
        }
    }

    /// Addresses nothing listens on, by construction: port 1 of two loopback
    /// addresses refuses at once, and no test binds it, so nothing races for it.
    fn dead_addr() -> SocketAddr {
        "127.0.0.1:1".parse().unwrap()
    }

    fn other_dead_addr() -> SocketAddr {
        // Use the same loopback host: macOS need not route 127.0.0.2.
        "127.0.0.1:2".parse().unwrap()
    }

    fn live_addr(collector: &FakeCollector) -> SocketAddr {
        collector
            .url()
            .trim_start_matches("http://")
            .parse()
            .unwrap()
    }

    /// Tracing: TC-045-24
    /// ACs: FR-045-AC-24
    #[test]
    fn tc_045_24_addresses_are_resolved_once_per_lifetime() {
        let collector = FakeCollector::start(vec![Reply::json(200, "{}"), Reply::json(200, "{}")]);
        let mut h = harness(vec![Ok(vec![live_addr(&collector)])]);
        h.transport.post(b"{}").unwrap();
        h.transport.post(b"{}").unwrap();
        assert_eq!(h.calls.load(Ordering::SeqCst), 1);
        // One second short of the lifetime is still cached.
        h.advance_secs
            .store(ADDR_CACHE_TTL.as_secs() - 1, Ordering::SeqCst);
        assert!(h.transport.addrs().is_ok());
        assert_eq!(h.calls.load(Ordering::SeqCst), 1);
    }

    /// Tracing: TC-045-24
    /// ACs: FR-045-AC-24
    #[test]
    fn tc_045_24_an_expired_cache_is_resolved_again() {
        let collector = FakeCollector::start(vec![Reply::json(200, "{}"), Reply::json(200, "{}")]);
        let mut h = harness(vec![Ok(vec![live_addr(&collector)])]);
        h.transport.post(b"{}").unwrap();
        h.advance_secs
            .store(ADDR_CACHE_TTL.as_secs(), Ordering::SeqCst);
        h.transport.post(b"{}").unwrap();
        assert_eq!(h.calls.load(Ordering::SeqCst), 2);
    }

    /// Tracing: TC-045-25
    /// ACs: FR-045-AC-25
    #[test]
    fn tc_045_25_when_every_cached_address_fails_the_next_push_resolves_again() {
        let collector = FakeCollector::start(vec![Reply::json(200, "{}")]);
        let mut h = harness(vec![
            Ok(vec![dead_addr(), other_dead_addr()]),
            Ok(vec![live_addr(&collector)]),
        ]);
        assert_eq!(h.transport.post(b"{}"), Err(TransportError::Connect));
        assert_eq!(h.calls.load(Ordering::SeqCst), 1);
        // No time has passed, yet the failure dropped the cache.
        assert_eq!(h.transport.post(b"{}").unwrap().status, 200);
        assert_eq!(h.calls.load(Ordering::SeqCst), 2);
    }

    /// Tracing: TC-045-26
    /// ACs: FR-045-AC-26
    #[test]
    fn tc_045_26_a_failed_or_empty_resolve_is_not_cached() {
        let collector = FakeCollector::start(vec![Reply::json(200, "{}")]);
        let mut h = harness(vec![
            Err(io::Error::other("no such host")),
            Ok(Vec::new()),
            Ok(vec![live_addr(&collector)]),
        ]);
        assert_eq!(h.transport.post(b"{}"), Err(TransportError::Connect));
        assert_eq!(h.transport.post(b"{}"), Err(TransportError::Connect));
        assert_eq!(h.transport.post(b"{}").unwrap().status, 200);
        assert_eq!(h.calls.load(Ordering::SeqCst), 3);
    }

    /// Tracing: TC-045-25
    /// ACs: FR-045-AC-25
    #[test]
    fn tc_045_25_a_deadline_that_ends_the_attempt_early_keeps_the_cache() {
        let collector = FakeCollector::start(vec![Reply::json(200, "{}")]);
        let out_of_time = Timeouts {
            connect: CONNECT_TIMEOUT,
            request: Duration::ZERO,
        };
        let mut h = harness_with(vec![Ok(vec![live_addr(&collector)])], out_of_time);
        assert_eq!(h.transport.post(b"{}"), Err(TransportError::Timeout));
        assert_eq!(h.transport.post(b"{}"), Err(TransportError::Timeout));
        assert_eq!(h.calls.load(Ordering::SeqCst), 1);
        assert!(h.transport.cache.is_some());
    }

    /// Tracing: TC-045-26
    /// ACs: FR-045-AC-26
    #[test]
    fn tc_045_26_expired_addresses_are_used_when_the_new_lookup_fails_or_is_empty() {
        let collector = FakeCollector::start(vec![Reply::json(200, "{}"), Reply::json(200, "{}")]);
        let mut h = harness(vec![
            Ok(vec![live_addr(&collector)]),
            Err(io::Error::other("servfail")),
            Ok(Vec::new()),
        ]);
        h.transport.post(b"{}").unwrap();
        h.advance_secs
            .store(ADDR_CACHE_TTL.as_secs(), Ordering::SeqCst);
        assert_eq!(h.transport.post(b"{}").unwrap().status, 200);
        assert_eq!(h.calls.load(Ordering::SeqCst), 2);
        // Still expired, so the next push looks up again.
        assert_eq!(h.transport.addrs().unwrap(), vec![live_addr(&collector)]);
        assert_eq!(h.calls.load(Ordering::SeqCst), 3);
    }

    /// Tracing: TC-045-27
    /// ACs: FR-045-AC-27
    #[test]
    fn tc_045_27_a_slow_lookup_does_not_spend_the_request_budget() {
        struct Slow(SocketAddr);
        impl Resolve for Slow {
            fn resolve(&self, _: &str, _: u16) -> io::Result<Vec<SocketAddr>> {
                // A lookup longer than the whole request budget, spent on the
                // clock rather than by sleeping.
                let until = Instant::now() + Duration::from_millis(150);
                while Instant::now() < until {
                    std::hint::spin_loop();
                }
                Ok(vec![self.0])
            }
        }
        let collector = FakeCollector::start(vec![Reply::json(200, "{}")]);
        let mut t = HttpTransport::with_resolver(
            OtlpEndpoint::parse("http://collector.test:4318", true).unwrap(),
            OtlpHeaders::parse("").unwrap(),
            Timeouts {
                connect: Duration::from_millis(50),
                request: Duration::from_millis(100),
            },
            "ua/1".into(),
            Box::new(Slow(live_addr(&collector))),
            Box::new(Instant::now),
        );
        assert_eq!(t.post(b"{}").unwrap().status, 200);
    }

    /// Tracing: TC-045-27
    /// ACs: FR-045-AC-27
    #[test]
    fn tc_045_27_no_more_than_four_addresses_are_kept() {
        let many: Vec<SocketAddr> = (1..=6)
            .map(|n| SocketAddr::from(([127, 0, 0, n], 1)))
            .collect();
        let mut h = harness(vec![Ok(many.clone())]);
        assert_eq!(h.transport.addrs().unwrap(), many[..MAX_ADDRS]);
    }
}
