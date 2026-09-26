//! Finds a Claude Code session's inbox socket in its own registry.
//!
//! Claude Code writes `~/.claude/sessions/<pid>.json` for every process. This
//! only ever reads those files. A record is trusted only when its process is the
//! one that wrote it (same pid, same start time, same pid namespace), because a
//! record can outlive its process and a pid can be reused. `/clear` changes the
//! `sessionId` inside a record and `--resume` starts a new process, so the lookup
//! is redone for every delivery.

use std::{
    fs,
    io::Read,
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

use claude_wire::{KNOWN_PEER_PROTOCOL, MIN_VERSION, RecordVerdict, SessionRecord};

/// The most bytes read from one registry file.
const MAX_RECORD_BYTES: u64 = 64 * 1024;

/// The most registry files looked at in one lookup.
const MAX_RECORDS: usize = 4096;

/// What the operating system says about a process; a seam so a test can say a
/// pid is dead or was reused.
pub trait Procs {
    /// The process's start time as Claude Code records it (`procStart`), `None`
    /// when there is no such process.
    fn start_time(&self, pid: u32) -> Option<String>;
    /// This machine's pid domain as Claude Code records it (`pidDomain`), `None`
    /// when it cannot be told.
    fn domain(&self) -> Option<String>;
    /// The user id this process runs as, `None` when it cannot be told.
    fn uid(&self) -> Option<u32>;
}

/// [`Procs`] over `/proc` (Linux).
#[derive(Debug, Default, Clone, Copy)]
pub struct ProcFs;

impl Procs for ProcFs {
    fn start_time(&self, pid: u32) -> Option<String> {
        relay_core::socket_status::proc_start_time(pid)
    }

    fn domain(&self) -> Option<String> {
        let machine = fs::read_to_string("/etc/machine-id").ok()?;
        let ns = fs::read_link("/proc/self/ns/pid").ok()?;
        Some(format!("linux:{}:{}", machine.trim(), ns.to_str()?))
    }

    fn uid(&self) -> Option<u32> {
        fs::metadata("/proc/self").ok().map(|m| m.uid())
    }
}

/// A live session's socket, ready to post to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// The Claude Code process.
    pub pid: u32,
    /// Its inbox socket.
    pub socket: PathBuf,
    /// Its version.
    pub version: String,
}

/// Why no socket could be used; every one of them means nothing was sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unresolved {
    /// The registry cannot be read.
    Unreadable(String),
    /// No record names the session.
    NoRecord,
    /// Records name it, and none is a live process that wrote it.
    Stale(usize),
    /// More than one live interactive (or more than one live non-interactive)
    /// process holds the session: the target is not knowable.
    Ambiguous(Vec<u32>),
    /// The live process is a version or protocol this code does not know.
    Unsupported {
        /// Its reported version.
        version: Option<String>,
        /// Its reported `peerProtocol`.
        peer_protocol: Option<u32>,
    },
    /// The record names no usable socket: not the process's own `<pid>.sock`, not
    /// in an expected directory, not owned by this user, or open to others.
    NoSocket(String),
}

impl std::fmt::Display for Unresolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable(why) => write!(f, "cannot read Claude Code's session registry: {why}"),
            Self::NoRecord => f.write_str("no Claude Code session record names this session"),
            Self::Stale(n) => write!(
                f,
                "{n} session record(s) name this session but none is a live process that wrote it"
            ),
            Self::Ambiguous(pids) => {
                write!(
                    f,
                    "several live Claude Code processes hold this session: {pids:?}"
                )
            }
            Self::Unsupported {
                version,
                peer_protocol,
            } => write!(
                f,
                "Claude Code {} / peerProtocol {} is not a known-good inbox socket \
                 (need peerProtocol {KNOWN_PEER_PROTOCOL}, version >= {}.{}.{})",
                version.as_deref().unwrap_or("?"),
                peer_protocol.map_or_else(|| "?".to_owned(), |p| p.to_string()),
                MIN_VERSION.0,
                MIN_VERSION.1,
                MIN_VERSION.2
            ),
            Self::NoSocket(why) => write!(f, "the session record has no usable socket: {why}"),
        }
    }
}

fn read_record(path: &Path) -> Option<SessionRecord> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(MAX_RECORD_BYTES)
        .read_to_end(&mut bytes)
        .ok()?;
    SessionRecord::parse(&bytes)
}

/// The socket of the live Claude Code process holding `session`, looked up in
/// `sessions_dir` (`~/.claude/sessions`).
///
/// # Errors
///
/// [`Unresolved`], always meaning nothing has been sent.
pub fn resolve(
    sessions_dir: &Path,
    session: &str,
    procs: &dyn Procs,
    socket_dirs: &[PathBuf],
) -> Result<Resolved, Unresolved> {
    let entries = fs::read_dir(sessions_dir).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Unresolved::NoRecord
        } else {
            Unresolved::Unreadable(e.to_string())
        }
    })?;
    let named: Vec<SessionRecord> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .take(MAX_RECORDS)
        .filter_map(|p| read_record(&p))
        .filter(|r| r.session_id == session)
        .collect();
    if named.is_empty() {
        return Err(Unresolved::NoRecord);
    }
    let domain = procs.domain();
    let total = named.len();
    let mut live: Vec<SessionRecord> = named
        .into_iter()
        .filter(|r| {
            let same_process = r.proc_start.is_some() && procs.start_time(r.pid) == r.proc_start;
            let same_domain = domain.is_some() && r.pid_domain == domain;
            same_process && same_domain
        })
        .collect();
    if live.is_empty() {
        return Err(Unresolved::Stale(total));
    }
    if live
        .iter()
        .any(|r| r.kind.as_deref() == Some("interactive"))
    {
        live.retain(|r| r.kind.as_deref() == Some("interactive"));
    }
    if live.len() > 1 {
        let mut pids: Vec<u32> = live.iter().map(|r| r.pid).collect();
        pids.sort_unstable();
        return Err(Unresolved::Ambiguous(pids));
    }
    let Some(record) = live.pop() else {
        return Err(Unresolved::Stale(total));
    };
    match record.verdict() {
        RecordVerdict::Supported => {}
        RecordVerdict::UnsupportedProtocol {
            peer_protocol,
            version,
        }
        | RecordVerdict::UnsupportedVersion {
            version,
            peer_protocol,
        } => {
            return Err(Unresolved::Unsupported {
                version,
                peer_protocol,
            });
        }
        RecordVerdict::Malformed { missing } => {
            return Err(Unresolved::NoSocket(format!("{missing} is absent")));
        }
    }
    let socket = record
        .messaging_socket_path
        .ok_or_else(|| Unresolved::NoSocket("messagingSocketPath is absent".to_owned()))?;
    let is_own_name = socket
        .file_name()
        .is_some_and(|n| n == format!("{}.sock", record.pid).as_str());
    if !socket.is_absolute() || !is_own_name {
        return Err(Unresolved::NoSocket(format!(
            "{} is not this process's <pid>.sock",
            socket.display()
        )));
    }
    if !socket
        .parent()
        .is_some_and(|dir| socket_dirs.iter().any(|d| d == dir))
    {
        return Err(Unresolved::NoSocket(format!(
            "{} is not in a Claude Code socket directory",
            socket.display()
        )));
    }
    // `symlink_metadata`: a symlink named `<pid>.sock` is not the socket.
    let meta = fs::symlink_metadata(&socket).map_err(|e| Unresolved::NoSocket(e.to_string()))?;
    if !meta.file_type().is_socket() {
        return Err(Unresolved::NoSocket("the path is not a socket".to_owned()));
    }
    if procs.uid() != Some(meta.uid()) {
        return Err(Unresolved::NoSocket(
            "the socket is not owned by this user".to_owned(),
        ));
    }
    if meta.permissions().mode() & 0o077 != 0 {
        return Err(Unresolved::NoSocket(
            "the socket is open to other users".to_owned(),
        ));
    }
    Ok(Resolved {
        pid: record.pid,
        socket,
        version: record.version.unwrap_or_default(),
    })
}
