//! A fake OTLP collector for tests: a real listener on `127.0.0.1:0` inside the
//! test process. The listener is bound once and kept, and the port is read from
//! it, so there is no bind-and-drop race.
//!
//! It answers each connection from a script and hands the requests it saw to the
//! test over a channel, so a test waits on `recv` rather than on a sleep.

use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::JoinHandle,
    time::Duration,
};

/// What the collector does with one connection.
#[derive(Clone)]
pub(crate) enum Reply {
    /// Answers with this raw response.
    Raw(Vec<u8>),
    /// Reads the request and then holds the connection open, answering nothing,
    /// until the collector is dropped.
    Silent,
}

impl Reply {
    pub(crate) fn status(code: u16) -> Self {
        Self::Raw(format!("HTTP/1.1 {code} X\r\nContent-Length: 0\r\n\r\n").into_bytes())
    }

    pub(crate) fn with_header(code: u16, header: &str) -> Self {
        Self::Raw(
            format!("HTTP/1.1 {code} X\r\n{header}\r\nContent-Length: 0\r\n\r\n").into_bytes(),
        )
    }

    pub(crate) fn json(code: u16, body: &str) -> Self {
        Self::Raw(
            format!(
                "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .into_bytes(),
        )
    }
}

/// One request the collector received.
#[derive(Debug, Clone)]
pub(crate) struct Seen {
    pub(crate) head: String,
    pub(crate) body: Vec<u8>,
}

pub(crate) struct FakeCollector {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    requests: mpsc::Receiver<Seen>,
    thread: Option<JoinHandle<()>>,
}

fn head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

/// Reads one request: the head and `Content-Length` bytes of body.
pub(crate) fn read_request(stream: &mut TcpStream) -> Option<Seen> {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .ok()?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let n = stream.read(&mut chunk).ok().filter(|n| *n > 0)?;
        buf.extend_from_slice(chunk.get(..n)?);
        if let Some(end) = head_end(&buf) {
            let head = String::from_utf8_lossy(buf.get(..end)?).into_owned();
            let len: usize = head
                .lines()
                .find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    k.trim()
                        .eq_ignore_ascii_case("content-length")
                        .then(|| v.trim().parse().ok())?
                })
                .unwrap_or(0);
            if buf.len() >= end + len {
                return Some(Seen {
                    head,
                    body: buf.get(end..end + len)?.to_vec(),
                });
            }
        }
    }
}

impl FakeCollector {
    /// A collector that answers every request with `replies` in turn, then with
    /// the last one again.
    pub(crate) fn start(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        let addr = listener.local_addr().expect("the bound address");
        let stop = Arc::new(AtomicBool::new(false));
        let script = Arc::new(Mutex::new(replies));
        let (tx, requests) = mpsc::channel();
        let thread = {
            let (stop, script) = (Arc::clone(&stop), Arc::clone(&script));
            std::thread::spawn(move || {
                let mut n = 0usize;
                let mut held: Vec<TcpStream> = Vec::new();
                while let Ok((mut stream, _)) = listener.accept() {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    let reply = {
                        let script = script.lock().unwrap_or_else(PoisonError::into_inner);
                        script
                            .get(n)
                            .or(script.last())
                            .cloned()
                            .unwrap_or(Reply::status(200))
                    };
                    n += 1;
                    let Some(seen) = read_request(&mut stream) else {
                        continue;
                    };
                    let _ = tx.send(seen);
                    match reply {
                        Reply::Raw(bytes) => {
                            let _ = stream.write_all(&bytes);
                        }
                        Reply::Silent => held.push(stream),
                    }
                }
            })
        };
        Self {
            addr,
            stop,
            requests,
            thread: Some(thread),
        }
    }

    /// The base URL to give the exporter.
    pub(crate) fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Blocks until the next request arrives.
    pub(crate) fn recv(&self) -> Seen {
        self.requests
            .recv_timeout(Duration::from_secs(30))
            .expect("the collector saw a request")
    }
}

impl Drop for FakeCollector {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accept.
        let _ = TcpStream::connect(self.addr);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// A [`Transport`](super::http::Transport) that hands each body to a channel and
/// answers 200: no network.
pub(crate) struct Recording(pub(crate) mpsc::Sender<Vec<u8>>);

impl super::http::Transport for Recording {
    fn post(&mut self, body: &[u8]) -> Result<super::http::Response, super::http::TransportError> {
        let _ = self.0.send(body.to_vec());
        Ok(super::http::Response {
            status: 200,
            retry_after_secs: None,
            body: Vec::new(),
        })
    }
}
