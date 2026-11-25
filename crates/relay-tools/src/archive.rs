//! Read-only, binary-safe Redis namespace archive. Never imports old
//! identities and never writes to or deletes from the source.

use std::{
    collections::HashSet,
    fs::OpenOptions,
    io::{BufWriter, Write},
    os::unix::fs::OpenOptionsExt,
    path::Path,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Serialize;

/// The namespace an archive must not be taken of.
const NEW_NAMESPACE: &str = "ixr:v3:";

/// How long a connect, read or write may take before the archive fails.
pub const IO_TIMEOUT: Duration = Duration::from_secs(10);

/// Why an archive failed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The prefix is empty or the new namespace.
    #[error("choose the explicit prior namespace, not the new v3 namespace")]
    Prefix,
    /// The URL is not a `redis://` URL.
    #[error("the Redis URL must start with redis://")]
    Url,
    /// The URL asks for TLS, which no ix-relay build carries.
    #[error(
        "rediss:// is not supported: ix-relay builds carry no TLS library and the relay is reached over WireGuard; use a redis:// URL"
    )]
    Tls,
    /// A Redis command or connection failed.
    #[error("Redis rejected the archive read: {0}")]
    Redis(#[from] redis::RedisError),
    /// The output file could not be created (it may already exist) or written.
    #[error("cannot write the archive: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Serialize)]
struct Header<'a> {
    format: &'static str,
    prefix: &'a str,
    created_at_ms: u128,
    consistent_snapshot: bool,
}

#[derive(Serialize)]
struct Entry {
    key_base64: String,
    dump_base64: String,
    ttl_ms: i64,
    captured_at_ms: u128,
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
}

fn line(out: &mut impl Write, value: &impl Serialize) -> Result<(), Error> {
    serde_json::to_writer(&mut *out, value).map_err(std::io::Error::from)?;
    out.write_all(b"\n")?;
    Ok(())
}

/// Connects, giving up when `timeout` has passed since `started`. The redis
/// crate's own connect timeout applies to each round trip of its setup
/// handshake, so a silent server would otherwise cost it several times over;
/// the handshake runs on a thread that is abandoned at the deadline (its own
/// per-round-trip timeouts end it shortly after).
fn connect_within(
    url: &str,
    started: Instant,
    timeout: Duration,
) -> Result<redis::Connection, Error> {
    let client = redis::Client::open(url)?;
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(client.get_connection_with_timeout(timeout));
    });
    match receiver.recv_timeout(timeout.saturating_sub(started.elapsed())) {
        Ok(connected) => Ok(connected?),
        Err(_) => Err(Error::Redis(redis::RedisError::from(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "connecting to Redis timed out",
        )))),
    }
}

/// [`archive_within`] with [`IO_TIMEOUT`].
pub fn archive(url: &str, prefix: &str, output: &Path) -> Result<usize, Error> {
    archive_within(url, prefix, output, IO_TIMEOUT)
}

/// Dumps every key under `prefix` (except the new namespace) to the new file
/// `output` as JSON lines and returns the number of keys written. The output
/// is created exclusively with mode 0600, so an existing archive is never
/// overwritten. A connect plus the first reply together fail after `timeout`, and each
/// later read or write fails after `timeout`.
pub fn archive_within(
    url: &str,
    prefix: &str,
    output: &Path,
    timeout: Duration,
) -> Result<usize, Error> {
    if prefix.is_empty() || prefix == NEW_NAMESPACE {
        return Err(Error::Prefix);
    }
    if url.starts_with("rediss://") {
        return Err(Error::Tls);
    }
    if !url.starts_with("redis://") {
        return Err(Error::Url);
    }
    // One deadline covers the connect and the first reply, so a server that
    // accepts and never answers costs `timeout` in all, not twice that.
    let started = Instant::now();
    let mut connection = connect_within(url, started, timeout)?;
    let remaining = timeout
        .saturating_sub(started.elapsed())
        .max(Duration::from_millis(1));
    connection.set_read_timeout(Some(remaining))?;
    connection.set_write_timeout(Some(timeout))?;
    let mut first_reply = true;
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(output)?;
    let mut out = BufWriter::new(file);
    line(
        &mut out,
        &Header {
            format: "redis-dump-jsonl-v1",
            prefix,
            created_at_ms: now_ms(),
            consistent_snapshot: false,
        },
    )?;
    let mut count = 0;
    let mut seen: HashSet<Vec<u8>> = HashSet::new();
    let mut cursor: Vec<u8> = b"0".to_vec();
    loop {
        let (next, keys): (Vec<u8>, Vec<Vec<u8>>) = redis::cmd("SCAN")
            .arg(&cursor)
            .arg("MATCH")
            .arg(format!("{prefix}*"))
            .arg("COUNT")
            .arg(100)
            .query(&mut connection)?;
        if std::mem::take(&mut first_reply) {
            connection.set_read_timeout(Some(timeout))?;
        }
        for key in keys {
            if key.starts_with(NEW_NAMESPACE.as_bytes()) || !seen.insert(key.clone()) {
                continue;
            }
            let ttl: i64 = redis::cmd("PTTL").arg(&key).query(&mut connection)?;
            let value: Option<Vec<u8>> = redis::cmd("DUMP").arg(&key).query(&mut connection)?;
            let Some(value) = value.filter(|_| ttl != -2) else {
                continue;
            };
            line(
                &mut out,
                &Entry {
                    key_base64: STANDARD.encode(&key),
                    dump_base64: STANDARD.encode(&value),
                    ttl_ms: ttl,
                    captured_at_ms: now_ms(),
                },
            )?;
            count += 1;
        }
        cursor = next;
        if cursor == b"0" {
            break;
        }
    }
    out.flush()?;
    out.get_ref().sync_all()?;
    Ok(count)
}
