//! The local hook failure spool: `<config dir>/spool/hook.jsonl`.
//!
//! A hook must never fail the agent's turn, so every failure used to end as one
//! stderr line nobody reads. Now each one also appends a [`SpoolRecord`] here,
//! and the next hook that finds Redis healthy ships them to the
//! `<prefix>health:hooks` stream (see [`Spool::begin_flush`]) and removes what
//! it shipped. Nothing here can fail or block a hook: recording returns an
//! error the caller only logs, takes no lock that can wait, and costs a few
//! syscalls.
//!
//! # File format
//!
//! JSON Lines, one [`SpoolRecord`] per line, appended with `O_APPEND` in a
//! single `write`, so concurrent hooks never interleave inside a line.
//!
//! # Where it lives and who can read it
//!
//! Beside `config.toml` (so `IX_RELAY_CONFIG` redirects it), never in a
//! relative directory: a relative config path would put `spool/` in the
//! lead's working directory, which is usually a git repo, so
//! [`Spool::in_config_dir`] refuses one and the hook logs to stderr only. The
//! directory is created `0700` and the file kept `0600` (fixed if found
//! wider): a `requeue_failed` record holds message text. A spool path that is
//! a symlink is refused, never followed.
//!
//! # The bound, and what "dropped" means
//!
//! The file never stays above `max_bytes` (256 KiB in production). When an
//! append leaves it at or above that, [`compact_bytes`] rewrites it in place:
//! it keeps the newest records that fit in half the bound and drops the older
//! ones, then adds the number it dropped to the `dropped` field of the first
//! record it kept. Summing `dropped` over a file therefore always gives the
//! records lost from its front, across any number of compactions. Halving the
//! bound means the next compaction is at least `max_bytes / 2` of appends
//! away: the O(size) rewrite runs only when the bound is hit and is amortised
//! over many appends.
//!
//! # Flush
//!
//! [`Spool::begin_flush`] reads every record; the caller ships them in chunks
//! and calls [`Flush::finish`] with how many it shipped. Exactly those are
//! removed; the rest, and anything appended meanwhile, stay for the next
//! flush. So a flush that fails partway loses nothing, and one that crashes
//! after shipping but before `finish` ships the same records again
//! (at-least-once).
//!
//! # Concurrency, and the windows that lose an append
//!
//! Appenders never lock. Compaction and flush take a non-blocking exclusive
//! `flock`, so two of them never run at once, and skip their work when the
//! lock is held: a hook never waits. Both read the file and then rewrite it,
//! and a record another hook appends between the read and the rewrite is lost
//! (compaction) or kept (flush, which rewrites from the current end of file, so
//! it keeps appends made after its read). The compaction window is the time to
//! read at most one bound of bytes and truncate: microseconds, once per
//! `max_bytes / 2` of failures. Closing it would need appenders to wait for
//! that lock, which a spool that can never block cannot do.

use std::{
    fs::{self, DirBuilder, File, OpenOptions, TryLockError},
    io::{self, Read, Seek, SeekFrom, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::PathBuf,
};

use relay_core::{MachineName, Message, SessionId};
use serde::{Deserialize, Serialize};

use crate::hook::Event;

/// The production bound on the spool file, in bytes.
pub(crate) const DEFAULT_MAX_BYTES: u64 = 256 * 1024;

/// Longest session id a record keeps. Session ids come from the harness's
/// hook input; a hostile or broken one must not make a record unbounded.
const MAX_SESSION_ID_CHARS: usize = 64;

/// Where in a hook call something failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Stage {
    /// The command line did not parse.
    Args,
    /// `config.toml` was unreadable or malformed.
    Config,
    /// The hook input on stdin was unreadable, too large, late or not JSON, or an inbox entry
    /// this binary cannot decode was popped.
    Decode,
    /// Connecting to Redis.
    Connect,
    /// Working out who this session is.
    Derive,
    /// Writing the registration, aliases and session cache.
    Register,
    /// Popping the inbox.
    Pop,
    /// Writing the hook's output. The popped messages were requeued.
    Output,
    /// The time budget ran out before a step could run, and delivery is
    /// delayed for it (a skipped `UserPromptSubmit` pop is an expected
    /// deferral and is not recorded). Nothing was lost: a skipped pop leaves
    /// the messages queued for the next hook.
    Budget,
    /// The output failed and putting the popped messages back failed too. They
    /// are still in flight and return when the lease expires; the record carries
    /// a copy.
    RequeueFailed,
    /// The output was delivered but acknowledging it failed or had no time:
    /// the batch stays in flight and is delivered again, marked, when its lease
    /// expires.
    Ack,
    /// The hook panicked. It still exited 0.
    Panic,
    /// A stage a newer binary wrote that this one does not know (see
    /// [`ErrorKind::Unknown`]).
    #[serde(other)]
    Unknown,
}

/// Why it failed, as a stable code: never the text of an error message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ErrorKind {
    /// A Redis call timed out.
    Timeout,
    /// Redis refused, dropped or could not be reached.
    Connection,
    /// Redis answered with an error.
    Redis,
    /// A script replied with something this binary does not understand.
    Protocol,
    /// A stored record did not decode.
    Corrupt,
    /// `AGENT_MSG_NAME` is not a valid bus name.
    InvalidName,
    /// The bus refused the request as invalid.
    Rejected,
    /// The derived name is held by another session and every widened or
    /// numbered variant is taken too, so the lead was not registered.
    NoFreeName,
    /// The derived name is held by a same-machine session and herdr could not
    /// say whether it is still live, so the lead was not registered.
    LivenessUnknown,
    /// herdr, asked who this session is, failed or timed out and nothing else
    /// named the session, so the lead was not registered.
    HerdrFailed,
    /// The hook's time budget was spent.
    BudgetExhausted,
    /// A local file or pipe failed (stdout, stdin, the spool).
    Io,
    /// The hook input was not the JSON this binary reads.
    Malformed,
    /// The hook input was larger than the hook accepts.
    TooLarge,
    /// The hook input did not arrive before the deadline: stdin stayed open
    /// without a whole payload.
    StdinTimeout,
    /// `config.toml` did not load.
    Config,
    /// The command line did not parse.
    Args,
    /// A panic.
    Panic,
    /// The lead already has as much in flight as one pop may add to: nothing
    /// was popped, the mail waits for a lease to free room.
    InflightFull,
    /// An acknowledgement found its batch no longer in flight: its lease had run
    /// out first, so the messages were, or will be, delivered again.
    LeaseExpired,
    /// A code a newer binary wrote that this one does not know. It lets a
    /// record with a new code decode, and be shipped as written, instead of
    /// being skipped and then removed by [`Flush::finish`].
    #[serde(other)]
    Unknown,
}

impl ErrorKind {
    /// The code for a bus error. Exhaustive on purpose: a new
    /// [`relay_core::Error`] variant must be classified here, not fall
    /// through to a default.
    pub(crate) fn of(error: &relay_core::Error) -> Self {
        use relay_core::Error as E;
        match error {
            E::Redis(e) if e.is_timeout() => Self::Timeout,
            E::Redis(e)
                if e.is_connection_refusal() || e.is_connection_dropped() || e.is_io_error() =>
            {
                Self::Connection
            }
            E::Redis(_) => Self::Redis,
            E::UnexpectedScriptReply { .. } => Self::Protocol,
            E::Decode { .. }
            | E::CorruptEscalation { .. }
            | E::InflightUnreadable { .. }
            | E::MoveUnreadable { .. } => Self::Corrupt,
            E::InvalidName { .. } => Self::InvalidName,
            E::Work(_) => Self::Rejected,
            E::DirectoryChanged
            | E::UnverifiedEnrollment { .. }
            | E::UnknownAgent(_)
            | E::InvalidAddress { .. }
            | E::RelativeAddressNeedsLocation { .. }
            | E::AmbiguousTarget { .. }
            | E::UnknownTarget { .. }
            | E::MessageTooLarge { .. }
            | E::UnsafeMessageText { .. }
            | E::InvalidTicket { .. }
            | E::InvalidEscalationStatus { .. }
            | E::InvalidEscalationId { .. }
            | E::EscalationNotFound(_)
            | E::EscalationAlreadyResolved(_)
            | E::EmptyDecision { .. }
            | E::InvalidSessionId { .. }
            | E::InvalidId { .. }
            | E::UnsafeEscalationText { .. } => Self::Rejected,
        }
    }
}

/// One hook failure: a line of the spool and an entry of `health:hooks`.
///
/// `messages` (set only for [`Stage::RequeueFailed`]) is message text written
/// by another lead. Whatever renders this record for a person or an agent must
/// neutralise it first ([`relay_core::text::neutralise`], FR-016).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct SpoolRecord {
    /// When it failed, Unix milliseconds.
    pub(crate) at_ms: i64,
    /// The machine it happened on, normalised.
    pub(crate) machine: MachineName,
    /// The lead's bus name, when the hook knew it by then.
    pub(crate) lead: Option<String>,
    /// Which hook event, when the input got far enough to say.
    pub(crate) event: Option<Event>,
    /// The harness session, cut to a bounded length. A `String`, not a
    /// `SessionId`: the cut is an excerpt for a person to read, and a record
    /// written by any binary must decode whatever the harness sent.
    pub(crate) session_id: Option<String>,
    /// Where it failed.
    pub(crate) stage: Stage,
    /// Why, as a code.
    pub(crate) error_kind: ErrorKind,
    /// Records the spool discarded just before this one to stay inside its
    /// bound. Zero, and absent from the JSON, almost always.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub(crate) dropped: u64,
    /// The derived name that collided, for a failure of name derivation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) candidate: Option<String>,
    /// The messages a failed requeue lost from Redis: set only for
    /// [`Stage::RequeueFailed`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) messages: Vec<Message>,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// Who and what a failure belongs to, apart from what failed.
#[derive(Debug, Clone)]
pub(crate) struct Subject {
    /// The machine, normalised.
    pub(crate) machine: MachineName,
    /// The lead's bus name, when known.
    pub(crate) lead: Option<String>,
    /// The hook event, when known.
    pub(crate) event: Option<Event>,
    /// The harness session id, when known.
    pub(crate) session_id: Option<SessionId>,
}

impl Subject {
    /// A failure on `machine` that knows nothing else yet.
    pub(crate) fn on(machine: MachineName) -> Self {
        Self {
            machine,
            lead: None,
            event: None,
            session_id: None,
        }
    }
}

impl SpoolRecord {
    /// A record for one failure of `subject`. The session id is cut to a
    /// bounded length.
    pub(crate) fn new(at_ms: i64, subject: &Subject, stage: Stage, error_kind: ErrorKind) -> Self {
        Self {
            at_ms,
            machine: subject.machine.clone(),
            lead: subject.lead.clone(),
            event: subject.event,
            session_id: subject
                .session_id
                .as_ref()
                .map(|s| s.as_str().chars().take(MAX_SESSION_ID_CHARS).collect()),
            stage,
            error_kind,
            dropped: 0,
            candidate: None,
            messages: Vec::new(),
        }
    }
}

/// A bounded, append-only failure file.
#[derive(Debug, Clone)]
pub(crate) struct Spool {
    path: PathBuf,
    max_bytes: u64,
}

impl Spool {
    /// The production spool: `spool/hook.jsonl` beside `config.toml`, wherever
    /// `IX_RELAY_CONFIG` or `$HOME` put it. `None` when neither is set, or the
    /// config path is relative (see the module docs).
    pub(crate) fn in_config_dir() -> Option<Self> {
        let dir = relay_core::config::config_dir()?;
        dir.is_absolute()
            .then(|| Self::new(dir.join("spool").join("hook.jsonl"), DEFAULT_MAX_BYTES))
    }

    /// Where the spool file is.
    pub(crate) fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// A spool at `path` bounded to `max_bytes`.
    pub(crate) fn new(path: PathBuf, max_bytes: u64) -> Self {
        Self { path, max_bytes }
    }

    /// Appends `record` as one line, then compacts if the file reached its
    /// bound.
    ///
    /// # Errors
    ///
    /// Any I/O failure, or a spool path that is a symlink. The caller logs it
    /// and carries on: a spool that cannot be written must not fail the hook.
    pub(crate) fn record(&self, record: &SpoolRecord) -> io::Result<()> {
        let mut line = serde_json::to_vec(record).map_err(io::Error::other)?;
        line.push(b'\n');
        self.refuse_symlink()?;
        let mut file = self.open_append()?;
        file.write_all(&line)?;
        let meta = file.metadata()?;
        if meta.permissions().mode() & 0o777 != 0o600 {
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        if meta.len() >= self.max_bytes {
            self.compact()?;
        }
        Ok(())
    }

    /// Refuses a spool path that is a symlink: appending to it would write
    /// wherever it points.
    fn refuse_symlink(&self) -> io::Result<()> {
        match fs::symlink_metadata(&self.path) {
            Ok(meta) if meta.file_type().is_symlink() => Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "the spool path is a symlink",
            )),
            Ok(_) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    fn open_append(&self) -> io::Result<File> {
        let open = || {
            OpenOptions::new()
                .create(true)
                .append(true)
                .mode(0o600)
                .open(&self.path)
        };
        match open() {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                if let Some(dir) = self.path.parent() {
                    DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
                }
                open()
            }
            other => other,
        }
    }

    /// Whether anything is waiting to be flushed: one `stat`.
    pub(crate) fn has_records(&self) -> bool {
        fs::symlink_metadata(&self.path).is_ok_and(|m| m.is_file() && m.len() > 0)
    }

    /// Rewrites the file to its newest records (see the module docs). A no-op
    /// while another process holds the file.
    fn compact(&self) -> io::Result<()> {
        // Append mode keeps a record another hook appends during the rewrite
        // from being overwritten: our write goes to the end too.
        let mut file = OpenOptions::new()
            .read(true)
            .append(true)
            .open(&self.path)?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Ok(()),
            Err(TryLockError::Error(e)) => return Err(e),
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        let kept = compact_bytes(&bytes, self.max_bytes / 2);
        file.set_len(0)?;
        file.write_all(&kept)
    }

    /// Takes the file's records for shipping. `None` when there is nothing to
    /// ship, or another process is already flushing or compacting.
    ///
    /// The returned [`Flush`] holds the file's lock. Ship
    /// [`Flush::entries`] (in as many pieces as the time allows), then call
    /// [`Flush::finish`] with how many were shipped. If none were, drop the
    /// [`Flush`]: the file is untouched.
    ///
    /// # Errors
    ///
    /// Any I/O failure other than "the file is not there".
    pub(crate) fn begin_flush(&self) -> io::Result<Option<Flush>> {
        if !self.has_records() {
            return Ok(None);
        }
        let mut file = match OpenOptions::new().read(true).append(true).open(&self.path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Error(e)) => return Err(e),
        }
        // Reads only the tail if something has grown the file far past its bound.
        let len = file.metadata()?.len();
        let start = len.saturating_sub(self.max_bytes.saturating_mul(2));
        file.seek(SeekFrom::Start(start))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        let read_len = start + u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        // A window that starts mid-file begins mid-line: skip to the first whole one.
        let skipped = if start > 0 {
            bytes
                .iter()
                .position(|&b| b == b'\n')
                .map_or(bytes.len(), |i| i + 1)
        } else {
            0
        };
        let mut offset = start + u64::try_from(skipped).unwrap_or(0);
        let mut entries = Vec::new();
        let mut refused = Vec::new();
        for line in bytes[skipped..].split_inclusive(|&b| b == b'\n') {
            offset += u64::try_from(line.len()).unwrap_or(0);
            let json = line.strip_suffix(b"\n").unwrap_or(line);
            let Ok(record) = serde_json::from_slice::<SpoolRecord>(json) else {
                refused.push(offset);
                continue;
            };
            let Ok(text) = String::from_utf8(json.to_vec()) else {
                refused.push(offset);
                continue;
            };
            entries.push(FlushEntry {
                json: text,
                machine: record.machine,
                lead: record.lead,
                end: offset,
            });
        }
        Ok(Some(Flush {
            file,
            read_len,
            entries,
            refused,
        }))
    }
}

/// One record ready to ship.
#[derive(Debug)]
pub(crate) struct FlushEntry {
    /// The record, as its JSON line.
    pub(crate) json: String,
    /// The machine it came from.
    pub(crate) machine: MachineName,
    /// The lead's bus name, when known.
    pub(crate) lead: Option<String>,
    /// The file offset just past this record's line.
    end: u64,
}

/// The records [`Spool::begin_flush`] took, and the lock on their file.
#[derive(Debug)]
pub(crate) struct Flush {
    file: File,
    read_len: u64,
    entries: Vec<FlushEntry>,
    /// The end offset of each line that is not a record this binary can decode
    /// (not JSON, not UTF-8, or a record outside the types' rules): they are
    /// removed with the shipped ones, and `finish` says how many.
    refused: Vec<u64>,
}

impl Flush {
    /// The records to ship, oldest first.
    pub(crate) fn entries(&self) -> &[FlushEntry] {
        &self.entries
    }

    /// Removes the first `shipped` records from the file, now that they are
    /// shipped. Everything after them, including anything a hook appended in the
    /// meantime, stays for the next flush.
    ///
    /// # Errors
    ///
    /// Any I/O failure. The file is then left as it was, or with the shipped
    /// records still in it: they are shipped again (at-least-once).
    ///
    /// Returns how many lines it removed that were not records (never silently:
    /// the caller records them as dropped).
    pub(crate) fn finish(mut self, shipped: usize) -> io::Result<usize> {
        if shipped == 0 && !self.entries.is_empty() {
            return Ok(0);
        }
        let cut = match shipped.checked_sub(1).and_then(|i| self.entries.get(i)) {
            Some(last) if shipped < self.entries.len() => last.end,
            // Everything read is shipped, junk lines after the last record too.
            _ => self.read_len,
        };
        let removed = self.refused.iter().filter(|end| **end <= cut).count();
        let len = self.file.metadata()?.len();
        if cut >= len {
            self.file.set_len(0)?;
            return Ok(removed);
        }
        self.file.seek(SeekFrom::Start(cut))?;
        let mut rest = Vec::new();
        self.file.read_to_end(&mut rest)?;
        self.file.set_len(0)?;
        self.file.write_all(&rest)?;
        Ok(removed)
    }
}

/// `bytes` (a whole spool file) cut to its newest records totalling at most
/// `target` bytes (but always at least one), with the count of everything
/// removed added to the first survivor's `dropped`. A line that is not a
/// record counts as one dropped record.
fn compact_bytes(bytes: &[u8], target: u64) -> Vec<u8> {
    let mut lines: Vec<(usize, Option<SpoolRecord>)> = bytes
        .split(|&b| b == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| (line.len() + 1, serde_json::from_slice(line).ok()))
        .collect();
    let mut kept_bytes = 0u64;
    let mut first_kept = lines.len();
    while first_kept > 0 {
        let size = u64::try_from(lines[first_kept - 1].0).unwrap_or(u64::MAX);
        if kept_bytes.saturating_add(size) > target && first_kept < lines.len() {
            break;
        }
        kept_bytes += size;
        first_kept -= 1;
    }
    let mut removed: u64 = lines
        .drain(..first_kept)
        .map(|(_, record)| 1 + record.map_or(0, |r| r.dropped))
        .sum();
    // An unreadable line among the survivors is one more lost record.
    let survivors: Vec<SpoolRecord> = lines
        .into_iter()
        .filter_map(|(_, record)| {
            if record.is_none() {
                removed = removed.saturating_add(1);
            }
            record
        })
        .collect();
    let mut out = Vec::new();
    for (i, mut record) in survivors.into_iter().enumerate() {
        if i == 0 {
            record.dropped = record.dropped.saturating_add(removed);
        }
        if let Ok(line) = serde_json::to_vec(&record) {
            out.extend_from_slice(&line);
            out.push(b'\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use proptest::prelude::*;

    use super::*;

    fn subject() -> Subject {
        Subject {
            machine: "hub".parse().unwrap(),
            lead: Some("qsl-lead".into()),
            event: Some(Event::Stop),
            session_id: Some("s1".parse().unwrap()),
        }
    }

    /// A minimal spool record is exactly these members; the optional ones
    /// (`dropped`, `candidate`, `messages`) are absent until set.
    #[test]
    fn a_spool_record_pins_its_field_list() {
        let value = serde_json::to_value(record(3)).unwrap();
        let mut keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "at_ms",
                "error_kind",
                "event",
                "lead",
                "machine",
                "session_id",
                "stage"
            ]
        );
    }

    fn record(n: i64) -> SpoolRecord {
        SpoolRecord::new(n, &subject(), Stage::Pop, ErrorKind::Timeout)
    }

    fn spool(dir: &tempfile::TempDir, max_bytes: u64) -> Spool {
        Spool::new(dir.path().join("spool").join("hook.jsonl"), max_bytes)
    }

    fn read(spool: &Spool) -> Vec<SpoolRecord> {
        std::fs::read_to_string(&spool.path)
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    /// The line size of `record`, so a bound can be set in whole records.
    fn line_len() -> u64 {
        u64::try_from(serde_json::to_vec(&record(0)).unwrap().len() + 1).unwrap()
    }

    /// Ships the first `n` records of a flush and removes them.
    fn ship(spool: &Spool, n: usize) -> Vec<SpoolRecord> {
        let Some(flush) = spool.begin_flush().unwrap() else {
            return Vec::new();
        };
        let n = n.min(flush.entries().len());
        let shipped = flush.entries()[..n]
            .iter()
            .map(|e| serde_json::from_str(&e.json).unwrap())
            .collect();
        flush.finish(n).unwrap();
        shipped
    }

    #[test]
    fn a_record_is_one_json_line_with_the_documented_fields() {
        let dir = tempfile::tempdir().unwrap();
        let spool = spool(&dir, DEFAULT_MAX_BYTES);
        spool.record(&record(7)).unwrap();
        let text = std::fs::read_to_string(&spool.path).unwrap();
        assert_eq!(
            text,
            "{\"at_ms\":7,\"machine\":\"hub\",\"lead\":\"qsl-lead\",\"event\":\"Stop\",\
             \"session_id\":\"s1\",\"stage\":\"pop\",\"error_kind\":\"timeout\"}\n"
        );
    }

    #[test]
    fn the_spool_directory_is_private_and_the_file_is_private() {
        let dir = tempfile::tempdir().unwrap();
        let spool = spool(&dir, DEFAULT_MAX_BYTES);
        assert!(!spool.has_records());
        spool.record(&record(1)).unwrap();
        assert!(spool.has_records());
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&spool.path), 0o600);
        assert_eq!(mode(spool.path.parent().unwrap()), 0o700);
    }

    #[test]
    fn a_file_with_wider_permissions_is_tightened() {
        let dir = tempfile::tempdir().unwrap();
        let spool = spool(&dir, DEFAULT_MAX_BYTES);
        spool.record(&record(1)).unwrap();
        std::fs::set_permissions(&spool.path, fs::Permissions::from_mode(0o644)).unwrap();
        spool.record(&record(2)).unwrap();
        let mode = std::fs::metadata(&spool.path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn a_symlinked_spool_is_refused_and_never_followed() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("elsewhere");
        std::fs::write(&target, b"precious\n").unwrap();
        let spool = spool(&dir, DEFAULT_MAX_BYTES);
        std::fs::create_dir_all(spool.path.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&target, &spool.path).unwrap();
        let err = spool.record(&record(1)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(std::fs::read(&target).unwrap(), b"precious\n");
        assert!(!spool.has_records(), "and it is never flushed either");
        assert!(spool.begin_flush().unwrap().is_none());
    }

    #[test]
    fn a_long_session_id_is_cut() {
        let long = Subject {
            session_id: Some("x".repeat(relay_core::MAX_ID_BYTES).parse().unwrap()),
            ..subject()
        };
        let r = SpoolRecord::new(0, &long, Stage::Decode, ErrorKind::Malformed);
        assert_eq!(r.session_id.unwrap().len(), MAX_SESSION_ID_CHARS);
    }

    #[test]
    fn a_full_spool_drops_the_oldest_and_counts_them() {
        let dir = tempfile::tempdir().unwrap();
        // Room for ten records: the tenth append reaches the bound.
        let spool = spool(&dir, line_len() * 10);
        for n in 0..10 {
            spool.record(&record(n)).unwrap();
        }
        let kept = read(&spool);
        // Compaction keeps the newest half.
        assert_eq!(
            kept.iter().map(|r| r.at_ms).collect::<Vec<_>>(),
            [5, 6, 7, 8, 9]
        );
        assert_eq!(kept[0].dropped, 5);
        assert!(kept[1..].iter().all(|r| r.dropped == 0));
    }

    #[test]
    fn dropped_counts_accumulate_across_compactions() {
        let dir = tempfile::tempdir().unwrap();
        let spool = spool(&dir, line_len() * 10);
        for n in 0..40 {
            spool.record(&record(n)).unwrap();
        }
        let kept = read(&spool);
        let dropped: u64 = kept.iter().map(|r| r.dropped).sum();
        assert_eq!(dropped + kept.len() as u64, 40);
        assert_eq!(
            kept.last().unwrap().at_ms,
            39,
            "the newest is never dropped"
        );
    }

    #[test]
    fn a_record_larger_than_the_bound_is_still_kept() {
        let dir = tempfile::tempdir().unwrap();
        let spool = spool(&dir, 10);
        spool.record(&record(1)).unwrap();
        spool.record(&record(2)).unwrap();
        let kept = read(&spool);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].at_ms, 2);
        assert_eq!(kept[0].dropped, 1);
    }

    /// A spool that holds far more than a stream keeps is shipped whole, not
    /// cut to the newest 1000.
    #[test]
    fn a_flush_offers_every_record_not_just_the_newest_thousand() {
        let dir = tempfile::tempdir().unwrap();
        let spool = spool(&dir, DEFAULT_MAX_BYTES * 8);
        for n in 0..1_500 {
            spool.record(&record(n)).unwrap();
        }
        let flush = spool.begin_flush().unwrap().unwrap();
        assert_eq!(flush.entries().len(), 1_500);
        assert_eq!(flush.entries()[0].machine.as_str(), "hub");
        assert_eq!(flush.entries()[0].lead.as_deref(), Some("qsl-lead"));
    }

    #[test]
    fn flushing_ships_every_record_then_empties_the_file_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let spool = spool(&dir, DEFAULT_MAX_BYTES);
        assert!(spool.begin_flush().unwrap().is_none(), "nothing to ship");
        for n in 0..3 {
            spool.record(&record(n)).unwrap();
        }
        assert_eq!(ship(&spool, usize::MAX).len(), 3);
        assert!(!spool.has_records());
        assert!(
            spool.begin_flush().unwrap().is_none(),
            "second flush is empty"
        );
    }

    #[test]
    fn a_partial_flush_removes_exactly_what_was_shipped() {
        let dir = tempfile::tempdir().unwrap();
        let spool = spool(&dir, DEFAULT_MAX_BYTES);
        for n in 0..5 {
            spool.record(&record(n)).unwrap();
        }
        let first = ship(&spool, 2);
        assert_eq!(first.iter().map(|r| r.at_ms).collect::<Vec<_>>(), [0, 1]);
        assert_eq!(
            read(&spool).iter().map(|r| r.at_ms).collect::<Vec<_>>(),
            [2, 3, 4]
        );
    }

    #[test]
    fn dropping_a_flush_without_finishing_leaves_the_file_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let spool = spool(&dir, DEFAULT_MAX_BYTES);
        spool.record(&record(1)).unwrap();
        let before = std::fs::read(&spool.path).unwrap();
        drop(spool.begin_flush().unwrap().unwrap());
        assert_eq!(std::fs::read(&spool.path).unwrap(), before);
    }

    #[test]
    fn a_hook_that_appends_during_a_flush_keeps_its_record_for_the_next_one() {
        let dir = tempfile::tempdir().unwrap();
        let spool = spool(&dir, DEFAULT_MAX_BYTES);
        spool.record(&record(1)).unwrap();
        let flush = spool.begin_flush().unwrap().unwrap();
        spool.record(&record(2)).unwrap();
        flush.finish(1).unwrap();
        // Exactly the shipped record is gone; the newcomer is not lost.
        assert_eq!(
            read(&spool).iter().map(|r| r.at_ms).collect::<Vec<_>>(),
            [2]
        );
    }

    #[test]
    fn a_second_flusher_is_turned_away_while_one_holds_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let spool = spool(&dir, DEFAULT_MAX_BYTES);
        spool.record(&record(1)).unwrap();
        let first = spool.begin_flush().unwrap().unwrap();
        assert!(spool.begin_flush().unwrap().is_none());
        drop(first);
        assert!(spool.begin_flush().unwrap().is_some());
    }

    #[test]
    fn unreadable_lines_are_not_shipped() {
        let dir = tempfile::tempdir().unwrap();
        let spool = spool(&dir, DEFAULT_MAX_BYTES);
        spool.record(&record(1)).unwrap();
        let mut file = OpenOptions::new().append(true).open(&spool.path).unwrap();
        file.write_all(b"{half a record\n").unwrap();
        let flush = spool.begin_flush().unwrap().unwrap();
        assert_eq!(flush.entries().len(), 1);
        flush.finish(1).unwrap();
        assert!(!spool.has_records(), "the junk went with what was shipped");
    }

    /// A record whose stage and code a newer binary invented still decodes,
    /// and is shipped as written, rather than being skipped and then removed
    /// with the records around it (a mixed-version machine shares one spool).
    /// Binaries older than this rule cannot be fixed retroactively.
    #[test]
    fn a_record_with_a_code_from_a_newer_binary_is_shipped_verbatim() {
        let dir = tempfile::tempdir().unwrap();
        let spool = spool(&dir, DEFAULT_MAX_BYTES);
        spool.record(&record(1)).unwrap();
        let future = r#"{"at_ms":2,"machine":"m","lead":null,"event":null,"session_id":null,"stage":"from_the_future","error_kind":"also_new","extra":1}"#;
        let mut file = OpenOptions::new().append(true).open(&spool.path).unwrap();
        writeln!(file, "{future}").unwrap();
        spool.record(&record(3)).unwrap();
        let flush = spool.begin_flush().unwrap().unwrap();
        let json: Vec<&str> = flush.entries().iter().map(|e| e.json.as_str()).collect();
        assert_eq!(json.len(), 3);
        assert_eq!(json[1], future, "shipped exactly as written");
        let decoded: SpoolRecord = serde_json::from_str(future).unwrap();
        assert_eq!(
            (decoded.stage, decoded.error_kind),
            (Stage::Unknown, ErrorKind::Unknown)
        );
    }

    #[test]
    fn a_spool_that_cannot_be_written_reports_it_instead_of_failing_the_hook() {
        let dir = tempfile::tempdir().unwrap();
        // The parent "directory" is a file: the spool can never be created.
        let blocker = dir.path().join("spool");
        std::fs::write(&blocker, b"").unwrap();
        let spool = Spool::new(blocker.join("hook.jsonl"), DEFAULT_MAX_BYTES);
        assert!(spool.record(&record(1)).is_err());
        assert!(!spool.has_records());
    }

    #[test]
    fn every_relay_error_has_a_code() {
        let timed_out = relay_core::Error::Redis(redis::RedisError::from(std::io::Error::from(
            std::io::ErrorKind::TimedOut,
        )));
        let refused = relay_core::Error::Redis(redis::RedisError::from(std::io::Error::from(
            std::io::ErrorKind::ConnectionRefused,
        )));
        let bad_name = relay_core::Error::InvalidName { name: "X".into() };
        assert_eq!(ErrorKind::of(&timed_out), ErrorKind::Timeout);
        assert_eq!(ErrorKind::of(&refused), ErrorKind::Connection);
        assert_eq!(ErrorKind::of(&bad_name), ErrorKind::InvalidName);
    }

    /// One step of the spool model.
    #[derive(Debug, Clone)]
    enum Op {
        Append,
        /// Ship this many of what the flush offers, then remove them.
        Flush(usize),
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(48))]

        /// Tracing: TC-030-03
        /// ACs: FR-030-AC-3
        ///
        /// P8: after N failures the spool holds the newest records that fit,
        /// `dropped` accounts for every other one, and the file never stays
        /// above its bound.
        #[test]
        fn tc_030_03_after_n_failures_the_spool_holds_what_fits_and_counts_the_rest(
            n in 0usize..120,
            capacity in 4u64..40,
        ) {
            let dir = tempfile::tempdir().unwrap();
            let max = line_len() * capacity;
            let spool = spool(&dir, max);
            for i in 0..n {
                spool.record(&record(i64::try_from(i).unwrap())).unwrap();
                let size = std::fs::metadata(&spool.path).unwrap().len();
                prop_assert!(size < max + line_len(), "size {size} over bound {max}");
            }
            let kept = read(&spool);
            let dropped: u64 = kept.iter().map(|r| r.dropped).sum();
            prop_assert_eq!(dropped + kept.len() as u64, n as u64);
            // The survivors are the newest, in order.
            let times: Vec<i64> = kept.iter().map(|r| r.at_ms).collect();
            let expected: Vec<i64> = ((n - kept.len())..n).map(|i| i64::try_from(i).unwrap()).collect();
            prop_assert_eq!(times, expected);
        }

        /// Tracing: TC-030-04
        /// ACs: FR-030-AC-4
        ///
        /// Over any interleaving of appends and (partial) flushes,
        /// shipped + remaining + dropped == appended, nothing is shipped
        /// twice, and shipped records keep their order.
        #[test]
        fn tc_030_04_shipped_plus_remaining_plus_dropped_is_everything_appended(
            ops in prop::collection::vec(
                prop_oneof![
                    3 => Just(Op::Append),
                    1 => (0usize..8).prop_map(Op::Flush),
                ],
                0..80,
            ),
            capacity in 4u64..24,
        ) {
            let dir = tempfile::tempdir().unwrap();
            let spool = spool(&dir, line_len() * capacity);
            let mut appended = 0i64;
            let mut shipped: Vec<SpoolRecord> = Vec::new();
            for op in ops {
                match op {
                    Op::Append => {
                        spool.record(&record(appended)).unwrap();
                        appended += 1;
                    }
                    Op::Flush(n) => shipped.extend(ship(&spool, n)),
                }
            }
            let remaining = read(&spool);
            let dropped: u64 = shipped
                .iter()
                .chain(remaining.iter())
                .map(|r| r.dropped)
                .sum();
            let total = shipped.len() as u64 + remaining.len() as u64 + dropped;
            prop_assert_eq!(total, u64::try_from(appended).unwrap());
            // No record is shipped twice, and order is preserved.
            let times: Vec<i64> = shipped.iter().chain(remaining.iter()).map(|r| r.at_ms).collect();
            let mut sorted = times.clone();
            sorted.sort_unstable();
            sorted.dedup();
            prop_assert_eq!(times, sorted);
        }
    }

    /// Tracing: TC-008-07
    /// ACs: FR-008-AC-7
    ///
    /// A spool record is byte-identical to what `main` wrote when the machine and
    /// session were bare strings: the literals were captured from `main`'s output
    /// for these inputs (AGE-2027 phase B), and an older binary reads what this
    /// one writes.
    #[test]
    fn a_spool_record_is_byte_identical_to_main() {
        let minimal = SpoolRecord::new(3, &subject(), Stage::Pop, ErrorKind::Timeout);
        assert_eq!(
            serde_json::to_string(&minimal).unwrap(),
            r#"{"at_ms":3,"machine":"hub","lead":"qsl-lead","event":"Stop","session_id":"s1","stage":"pop","error_kind":"timeout"}"#
        );
        let mut full = SpoolRecord::new(
            9,
            &Subject::on("node-a".parse().unwrap()),
            Stage::RequeueFailed,
            ErrorKind::NoFreeName,
        );
        full.dropped = 2;
        full.candidate = Some("qsl-lead".into());
        full.messages = vec![Message {
            from: "a".parse().unwrap(),
            to: "b".parse().unwrap(),
            text: "t".into(),
            sent_at_ms: 1,
        }];
        assert_eq!(
            serde_json::to_string(&full).unwrap(),
            r#"{"at_ms":9,"machine":"node-a","lead":null,"event":null,"session_id":null,"stage":"requeue_failed","error_kind":"no_free_name","dropped":2,"candidate":"qsl-lead","messages":[{"from":"a","to":"b","text":"t","sent_at_ms":1}]}"#
        );
        for golden in [
            r#"{"at_ms":3,"machine":"hub","lead":"qsl-lead","event":"Stop","session_id":"s1","stage":"pop","error_kind":"timeout"}"#,
            r#"{"at_ms":9,"machine":"node-a","lead":null,"event":null,"session_id":null,"stage":"requeue_failed","error_kind":"no_free_name","dropped":2,"candidate":"qsl-lead","messages":[{"from":"a","to":"b","text":"t","sent_at_ms":1}]}"#,
        ] {
            let decoded: SpoolRecord = serde_json::from_str(golden).unwrap();
            assert_eq!(serde_json::to_string(&decoded).unwrap(), golden);
        }
    }

    /// Tracing: TC-008-06
    /// ACs: FR-008-AC-6
    ///
    /// A record whose machine is not a machine name does not decode, so the
    /// flush leaves it out rather than shipping an attribution nobody can read.
    #[test]
    fn a_record_with_a_malformed_machine_does_not_decode() {
        let bad = r#"{"at_ms":3,"machine":"hub","lead":"qsl-lead","event":"Stop","session_id":"s1","stage":"pop","error_kind":"timeout"}"#.replace(r#""machine":"hub""#, r#""machine":"""#);
        assert!(serde_json::from_str::<SpoolRecord>(&bad).is_err());
    }
}
