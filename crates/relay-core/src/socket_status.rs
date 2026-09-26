//! What the watcher's Claude Code socket delivery reports about itself (FR-044).
//!
//! Written by `relay-watcher` after every socket delivery attempt to
//! `~/.ix/relay/state/socket/socket-<watcher pid>.json` and read by `agent-msg doctor`,
//! which cannot see the watcher's memory. One file per watcher process, so two
//! watchers on one machine never overwrite each other and doctor reports each
//! live one. The files exist only when `[watcher] delivery = "socket"`; with the
//! default (herdr) nothing writes them.

use std::{
    fs, io,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

/// The start of a status file's name, inside [`crate::config::state_dir`].
const FILE_PREFIX: &str = "socket-";

/// The end of a status file's name.
const FILE_SUFFIX: &str = ".json";

/// The most status files [`read_all`] reads: one per watcher, and a machine
/// runs one or two.
const MAX_FILES: usize = 64;

/// The directory the status files live in, inside the machine's state directory: a
/// directory of their own, so that its mode can be private whatever the shared state
/// directory's is.
#[must_use]
pub fn dir_in(state_dir: &Path) -> PathBuf {
    state_dir.join("socket")
}

/// The user id this process runs as, `None` where `/proc` cannot say.
fn current_uid() -> Option<u32> {
    fs::metadata("/proc/self").ok().map(|m| m.uid())
}

/// Whether `meta` (from `symlink_metadata`, so a symlink is not a directory) is a
/// directory owned by `uid` that nobody else can enter or write: the only kind of
/// directory that may hold status files. A pure function of its inputs.
fn is_private_dir_of(meta: &fs::Metadata, uid: Option<u32>) -> bool {
    meta.is_dir() && uid == Some(meta.uid()) && meta.permissions().mode() & 0o077 == 0
}

/// Why a directory that exists may not hold status files, `None` when it may.
fn untrusted_dir(dir: &Path) -> Option<String> {
    match fs::symlink_metadata(dir) {
        Ok(meta) if is_private_dir_of(&meta, current_uid()) => None,
        Ok(_) => Some(format!(
            "{} is not a directory of this user closed to others (0700), or is a symlink; it is \
             not trusted and not repaired",
            dir.display()
        )),
        Err(e) => Some(e.to_string()),
    }
}

/// Whether the file at `path` may be read as a status: a regular file (not a symlink)
/// owned by this user.
fn file_is_ours(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .is_ok_and(|m| m.file_type().is_file() && current_uid() == Some(m.uid()))
}

/// The status file of the watcher with process id `pid` in `dir`.
#[must_use]
pub fn path_for(state_dir: &Path, pid: u32) -> PathBuf {
    state_dir.join(format!("{FILE_PREFIX}{pid}{FILE_SUFFIX}"))
}

/// Every status file in `state_dir`, with what reading it gave; an absent
/// directory is none.
#[must_use]
pub fn read_all(state_dir: &Path) -> Vec<(PathBuf, io::Result<SocketStatus>)> {
    if fs::symlink_metadata(state_dir).is_err() {
        return Vec::new();
    }
    if let Some(why) = untrusted_dir(state_dir) {
        // Anyone could have written any file in it: none is read.
        return vec![(
            state_dir.to_path_buf(),
            Err(io::Error::new(io::ErrorKind::PermissionDenied, why)),
        )];
    }
    let Ok(entries) = fs::read_dir(state_dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                n.starts_with(FILE_PREFIX)
                    && n.ends_with(FILE_SUFFIX)
                    && n[FILE_PREFIX.len()..n.len() - FILE_SUFFIX.len()]
                        .bytes()
                        .all(|b| b.is_ascii_digit())
            })
        })
        .take(MAX_FILES)
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|p| {
            let read = SocketStatus::read(&p);
            (p, read)
        })
        .collect()
}

/// The most bytes read from the file: it holds a few short records.
const MAX_READ_BYTES: u64 = 256 * 1024;

/// One lead's Claude Code socket as the watcher last resolved it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeadSocket {
    /// The lead's bus name.
    pub lead: String,
    /// The Claude Code process that owns the socket.
    pub pid: u32,
    /// The socket path.
    pub socket: String,
    /// The Claude Code version that process reported.
    pub version: String,
}

/// A batch whose message Claude Code is holding behind an approval prompt, kept
/// so a restarted watcher goes on renewing its lease instead of posting it again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeldBatch {
    /// The lead whose batch it is.
    pub lead: String,
    /// The in-flight batch id (global to the bus, so it means the same after a restart).
    pub batch: u64,
    /// The `msg_id` posted.
    pub msg_id: String,
    /// The session it was posted to.
    pub session: String,
    /// When the hold began, Unix milliseconds; the hold ends at the maximum hold after it.
    pub since_ms: i64,
    /// When Claude Code reported the message delivered (the user approved it), Unix
    /// milliseconds, if it has: the grace for its transcript line goes on across a
    /// restart, and an approved message is never dead-lettered as never approved.
    #[serde(default)]
    pub delivered_since_ms: Option<i64>,
}

/// A decision about a batch whose write to the bus has not succeeded yet, kept so a
/// restarted watcher goes on with it instead of losing it (an acknowledgement) or turning
/// it into something else (a denial that becomes `hold_expired`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwedSettlement {
    /// The lead whose batch it is.
    pub lead: String,
    /// The in-flight batch id.
    pub batch: u64,
    /// `ack`, `requeue` or `dead_letter`.
    pub action: String,
    /// For a dead letter, the reason to record once it is written.
    pub reason: Option<String>,
    /// When the decision was made, Unix milliseconds.
    pub since_ms: i64,
}

/// A batch the watcher dead-lettered on its own account, and why: a dead letter is
/// a bare message with no reason, so the reason is kept here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeadLetterNote {
    /// The lead whose batch it was.
    pub lead: String,
    /// The batch id.
    pub batch: u64,
    /// `denied` (the receiver's user denied it) or `hold_expired` (nobody answered).
    pub reason: String,
    /// When, Unix milliseconds.
    pub at_ms: i64,
}

/// The most [`DeadLetterNote`]s kept (the newest).
pub const MAX_DEAD_LETTER_NOTES: usize = 32;

/// The start time of process `pid` as `/proc/<pid>/stat` gives it (field 22), `None`
/// when there is no such process. With the pid it identifies a process: a pid that a
/// later process reuses has another start time.
#[must_use]
pub fn proc_start_time(pid: u32) -> Option<String> {
    start_time_from_stat(&fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)
}

/// The start time (field 22) of a `/proc/<pid>/stat` line. The command name is
/// parenthesised and may hold spaces and parentheses, so the fields that matter
/// are counted from the last `)`: the start time is the 20th after the state.
#[must_use]
pub fn start_time_from_stat(stat: &str) -> Option<String> {
    let after = stat.rsplit_once(')')?.1;
    after.split_whitespace().nth(19).map(str::to_owned)
}

/// How many times Claude Code sent something this watcher's typed wire formats
/// (`claude-wire`) do not know, since its reply socket was bound. A drift signal:
/// Claude Code's cross-session formats are undocumented and can change in any
/// release. Counts saturate; nothing here fails a delivery.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireDrift {
    /// Reply-socket lines that are valid JSON in a shape not known (renamed `type` or
    /// `action`, missing `status`, wrong field types).
    #[serde(default)]
    pub unrecognised_lines: u64,
    /// Reply-socket lines that are not JSON at all.
    #[serde(default)]
    pub not_json_lines: u64,
    /// Receipts whose `status` (with its detail) is not one known.
    #[serde(default)]
    pub unknown_status_receipts: u64,
    /// Transcript looks that found a line naming a message id without a structured
    /// `origin.msg_id`.
    #[serde(default)]
    pub transcript_shape_unrecognised: u64,
}

impl WireDrift {
    /// The drift on the reply socket: unrecognised or non-JSON lines and unknown
    /// receipt statuses.
    #[must_use]
    pub fn control_total(&self) -> u64 {
        self.unrecognised_lines
            .saturating_add(self.not_json_lines)
            .saturating_add(self.unknown_status_receipts)
    }
}

/// The socket delivery state of one watcher process.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SocketStatus {
    /// The watcher's process id; a status whose process is gone is stale.
    pub pid: u32,
    /// The watcher's start time ([`proc_start_time`]), so a pid that a later
    /// process reuses is not taken for this watcher.
    #[serde(default)]
    pub start_time: Option<String>,
    /// When it was written, Unix milliseconds.
    pub updated_at_ms: i64,
    /// The watcher's own reply socket, `None` when it could not be bound (every
    /// batch then goes to herdr).
    pub reply_socket: Option<String>,
    /// The leads whose socket was resolved by their last delivery.
    pub leads: Vec<LeadSocket>,
    /// Leads whose Claude Code holds relay mail behind an approval prompt
    /// (bypass mode without `crossSessionInbound: "accept"`).
    pub held: Vec<String>,
    /// Batches held behind an approval prompt (restored by a restarted watcher).
    #[serde(default)]
    pub held_batches: Vec<HeldBatch>,
    /// Decisions about batches that are still owed a write to the bus.
    #[serde(default)]
    pub owed: Vec<OwedSettlement>,
    /// Batches dead-lettered by the watcher, and why, newest last.
    #[serde(default)]
    pub dead_lettered: Vec<DeadLetterNote>,
    /// What Claude Code sent that the typed wire formats do not know.
    #[serde(default)]
    pub drift: WireDrift,
    /// One line per lead the socket cannot be used for (no record, unsupported
    /// version, ambiguous), and why; these batches go to herdr.
    pub notes: Vec<String>,
}

impl SocketStatus {
    /// Writes the status atomically (a temp file beside it, then a rename).
    ///
    /// # Errors
    ///
    /// Any I/O error creating the directory, writing or renaming.
    pub fn write(&self, path: &Path) -> io::Result<()> {
        use std::io::Write;
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        if let Some(dir) = path.parent() {
            // Created private in one step (the file names sessions, message ids and
            // sockets). A directory that already exists is used only if it is ours and
            // private; it is never repaired, because whatever it holds may have been
            // planted while it was open.
            if fs::symlink_metadata(dir).is_err() {
                fs::DirBuilder::new()
                    .recursive(true)
                    .mode(0o700)
                    .create(dir)?;
            }
            if let Some(why) = untrusted_dir(dir) {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, why));
            }
        }
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(format!(".{}.tmp", std::process::id()));
        let tmp = std::path::PathBuf::from(tmp);
        let bytes = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        // Never through a link someone left at the temporary name.
        let _ = fs::remove_file(&tmp);
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(&bytes)?;
        // On disk before the rename makes it the file: a crash leaves the old one or
        // the new one, never an empty one.
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)
    }

    /// Reads the status at `path`.
    ///
    /// # Errors
    ///
    /// The I/O error (`NotFound` when there is none), or `InvalidData` when it
    /// does not decode.
    pub fn read(path: &Path) -> io::Result<Self> {
        use io::Read;
        if fs::symlink_metadata(path).is_ok() && !file_is_ours(path) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "the status file is not a regular file of this user",
            ));
        }
        let mut bytes = Vec::new();
        fs::File::open(path)?
            .take(MAX_READ_BYTES)
            .read_to_end(&mut bytes)?;
        serde_json::from_slice(&bytes).map_err(io::Error::other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::DirBuilderExt;

    #[test]
    fn a_status_round_trips_through_its_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_for(&dir.path().join("state"), 7);
        let status = SocketStatus {
            pid: 7,
            start_time: Some("123".into()),
            updated_at_ms: 9,
            reply_socket: Some("/run/x/7.sock".into()),
            leads: vec![LeadSocket {
                lead: "a".into(),
                pid: 3,
                socket: "/run/x/3.sock".into(),
                version: "2.1.283".into(),
            }],
            held: vec!["b".into()],
            held_batches: vec![HeldBatch {
                lead: "b".into(),
                batch: 4,
                msg_id: "m".into(),
                session: "s".into(),
                since_ms: 5,
                delivered_since_ms: Some(6),
            }],
            owed: vec![OwedSettlement {
                lead: "b".into(),
                batch: 4,
                action: "ack".into(),
                reason: None,
                since_ms: 7,
            }],
            dead_lettered: vec![DeadLetterNote {
                lead: "b".into(),
                batch: 3,
                reason: "denied".into(),
                at_ms: 6,
            }],
            drift: WireDrift {
                unrecognised_lines: 1,
                not_json_lines: 2,
                unknown_status_receipts: 3,
                transcript_shape_unrecognised: 4,
            },
            notes: vec!["c: no record".into()],
        };
        status.write(&path).unwrap();
        assert_eq!(SocketStatus::read(&path).unwrap(), status);
    }

    /// Tracing: TC-044-15
    /// ACs: FR-044-AC-15
    ///
    /// A directory that already exists and is open (0755, 0777) is neither written nor read
    /// and never repaired: a file planted in it while it was open must not become trusted
    /// by a later `chmod`.
    #[test]
    fn an_existing_open_status_directory_is_refused_and_never_repaired() {
        for mode in [0o755, 0o777, 0o770] {
            let dir = tempfile::tempdir().unwrap();
            let status_dir = dir_in(dir.path());
            fs::create_dir(&status_dir).unwrap();
            fs::set_permissions(&status_dir, fs::Permissions::from_mode(mode)).unwrap();
            // A forged file planted while it was open.
            let forged = path_for(&status_dir, 7);
            fs::write(&forged, r#"{"pid":7,"held_batches":[{"lead":"ea","batch":7,"msg_id":"m","session":"s","since_ms":1}]}"#).unwrap();
            let err = SocketStatus::default()
                .write(&path_for(&status_dir, 4))
                .unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::PermissionDenied, "{mode:o}");
            let after = fs::metadata(&status_dir).unwrap().permissions().mode() & 0o777;
            assert_eq!(after, mode, "not repaired");
            let read = read_all(&status_dir);
            assert_eq!(read.len(), 1, "{mode:o}: nothing read");
            assert_eq!(
                read[0].1.as_ref().unwrap_err().kind(),
                io::ErrorKind::PermissionDenied
            );
            assert!(forged.exists(), "left as it was");
        }
    }

    /// Tracing: TC-044-15
    /// ACs: FR-044-AC-15
    ///
    /// A symlink in place of the directory, or of a file, is not followed; a directory or
    /// file of another user is not trusted (checked as a pure function of the owner, since
    /// the test cannot be another user).
    #[test]
    fn a_symlinked_directory_a_symlinked_file_and_another_owner_are_not_trusted() {
        let dir = tempfile::tempdir().unwrap();
        // A symlinked directory.
        let real = dir.path().join("real");
        fs::DirBuilder::new().mode(0o700).create(&real).unwrap();
        SocketStatus::default().write(&path_for(&real, 5)).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert_eq!(
            SocketStatus::default()
                .write(&path_for(&link, 6))
                .unwrap_err()
                .kind(),
            io::ErrorKind::PermissionDenied
        );
        let read = read_all(&link);
        assert_eq!(read.len(), 1);
        assert!(read[0].1.is_err());
        // A symlinked file in a proper directory.
        let elsewhere = dir.path().join("other.json");
        fs::write(&elsewhere, "{}").unwrap();
        std::os::unix::fs::symlink(&elsewhere, path_for(&real, 8)).unwrap();
        let read = read_all(&real);
        let planted = read
            .iter()
            .find(|(p, _)| p == &path_for(&real, 8))
            .expect("listed");
        assert_eq!(
            planted.1.as_ref().unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert!(
            read.iter()
                .any(|(p, r)| p == &path_for(&real, 5) && r.is_ok())
        );
        // Another owner: the same directory, judged as if this process were someone else.
        let meta = fs::symlink_metadata(&real).unwrap();
        assert!(is_private_dir_of(&meta, current_uid()));
        assert!(!is_private_dir_of(&meta, current_uid().map(|u| u + 1)));
        assert!(!is_private_dir_of(&meta, None));
    }

    /// Tracing: TC-044-15
    /// ACs: FR-044-AC-15
    ///
    /// The file is private from its creation, whatever the umask.
    #[test]
    fn the_status_file_is_created_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = path_for(&dir.path().join("state"), 9);
        SocketStatus::default().write(&path).unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path) & 0o077, 0, "{:o}", mode(&path));
        assert_eq!(
            mode(path.parent().unwrap()) & 0o077,
            0,
            "the state directory too"
        );
    }

    #[test]
    fn a_missing_file_is_not_found_and_garbage_is_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_for(dir.path(), 1);
        assert_eq!(
            SocketStatus::read(&path).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        fs::write(&path, b"not json").unwrap();
        assert!(SocketStatus::read(&path).is_err());
    }

    /// Tracing: TC-044-09
    /// ACs: FR-044-AC-9
    ///
    /// Two watchers write two files; only status files are read, in order.
    #[test]
    fn two_watchers_keep_two_files_and_read_all_finds_only_them() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("socket");
        for pid in [20, 3] {
            SocketStatus {
                pid,
                ..SocketStatus::default()
            }
            .write(&path_for(&dir, pid))
            .unwrap();
        }
        fs::write(dir.join("watcher.json"), "{}").unwrap();
        fs::write(dir.join("socket-x.json"), "{}").unwrap();
        fs::write(dir.join("socket-5.json.4.tmp"), "{}").unwrap();
        let pids: Vec<u32> = read_all(&dir)
            .into_iter()
            .map(|(_, s)| s.unwrap().pid)
            .collect();
        assert_eq!(pids, [20, 3], "sorted by file name, nothing else read");
        assert!(read_all(&dir.join("absent")).is_empty());
    }
}
