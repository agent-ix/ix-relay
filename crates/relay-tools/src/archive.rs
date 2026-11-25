//! Read-only, binary-safe Redis namespace archive. Never imports old
//! identities and never writes to or deletes from the source.

use std::{
    collections::HashSet,
    fs::OpenOptions,
    io::{BufWriter, Write},
    os::unix::fs::OpenOptionsExt,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Serialize;

/// The namespace an archive must not be taken of.
const NEW_NAMESPACE: &str = "ixr:v3:";

/// Why an archive failed. Callers print one fixed line; the detail is for
/// tests.
#[derive(Debug)]
pub enum Error {
    /// The prefix is empty or the new namespace.
    Prefix,
    /// The URL is not a `redis://` or `rediss://` URL.
    Url,
    /// A Redis command or connection failed.
    Redis(redis::RedisError),
    /// The output file could not be created (it may already exist) or written.
    Io(std::io::Error),
}

impl From<redis::RedisError> for Error {
    fn from(err: redis::RedisError) -> Self {
        Self::Redis(err)
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
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

/// Dumps every key under `prefix` (except the new namespace) to the new file
/// `output` as JSON lines and returns the number of keys written. The output
/// is created exclusively with mode 0600, so an existing archive is never
/// overwritten.
pub fn archive(url: &str, prefix: &str, output: &Path) -> Result<usize, Error> {
    if prefix.is_empty() || prefix == NEW_NAMESPACE {
        return Err(Error::Prefix);
    }
    if !(url.starts_with("redis://") || url.starts_with("rediss://")) {
        return Err(Error::Url);
    }
    let mut connection = redis::Client::open(url)?.get_connection()?;
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
