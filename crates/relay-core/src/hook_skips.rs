//! The count of hook activity updates that could not be recorded in
//! `spool/activity.json` (FR-068).
//!
//! A dropped update cannot be counted in the file it failed to update, so each
//! drop appends one byte to a sidecar file beside it with a single `O_APPEND`
//! write, which takes no lock and never waits, and the count is the file's
//! length. There is one sidecar per [`Reason`]. The count stops at
//! [`MAX_SKIPS`] bytes so the file stays bounded, and a count at that bound is
//! reported as saturated, never as exact. Deleting a sidecar resets its count.

use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

/// The most skips a sidecar records, one byte each.
pub const MAX_SKIPS: u64 = 1 << 20;

/// Why an update was dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// The state file's lock was not free within the bound.
    Lock,
    /// The lock was free, but the state file, its directory or its lock file
    /// could not be written.
    WriteFailed,
}

impl Reason {
    /// The sidecar file's extension.
    const fn extension(self) -> &'static str {
        match self {
            Self::Lock => "lockskips",
            Self::WriteFailed => "writefails",
        }
    }
}

/// The sidecar for `reason` beside the state file at `state`.
#[must_use]
pub fn sidecar_path(state: &Path, reason: Reason) -> PathBuf {
    state.with_extension(reason.extension())
}

/// The skips recorded for `reason` beside the state file at `state`: the
/// sidecar's length, `0` when it does not exist.
///
/// # Errors
///
/// The sidecar exists but is a symlink or not a regular file, or cannot be
/// inspected.
pub fn count(state: &Path, reason: Reason) -> io::Result<u64> {
    match fs::symlink_metadata(sidecar_path(state, reason)) {
        Ok(m) if m.file_type().is_file() => Ok(m.len()),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the skip count is not a regular file",
        )),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(e),
    }
}

/// When the sidecar for `reason` last grew, Unix milliseconds; `None` when it
/// is missing or unreadable.
#[must_use]
pub fn last_skip_ms(state: &Path, reason: Reason) -> Option<i64> {
    let modified = fs::symlink_metadata(sidecar_path(state, reason))
        .ok()?
        .modified()
        .ok()?;
    let since = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    i64::try_from(since.as_millis()).ok()
}

/// Whether a count has reached [`MAX_SKIPS`] and so reads "at least".
#[must_use]
pub const fn saturated(count: u64) -> bool {
    count >= MAX_SKIPS
}

/// Records one dropped update. Takes no lock, never waits, and does nothing
/// when the sidecar is at the bound, is not a regular file, or cannot be
/// opened (an unwritable spool directory cannot record anything).
#[cfg(unix)]
pub fn record(state: &Path, reason: Reason) {
    use std::os::unix::fs::OpenOptionsExt;

    if count(state, reason).map_or(true, saturated) {
        return;
    }
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(sidecar_path(state, reason))
    {
        let _ = file.write_all(b"1");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_skip_is_one_byte_per_reason_and_a_missing_sidecar_is_zero() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("activity.json");
        assert_eq!(count(&state, Reason::Lock).unwrap(), 0);
        record(&state, Reason::Lock);
        record(&state, Reason::Lock);
        record(&state, Reason::WriteFailed);
        assert_eq!(count(&state, Reason::Lock).unwrap(), 2);
        assert_eq!(count(&state, Reason::WriteFailed).unwrap(), 1);
        assert!(last_skip_ms(&state, Reason::Lock).is_some());
    }

    #[test]
    fn a_sidecar_that_is_not_a_regular_file_is_an_error_and_is_never_followed() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("activity.json");
        let target = dir.path().join("target");
        fs::write(&target, "xxxx").unwrap();
        std::os::unix::fs::symlink(&target, sidecar_path(&state, Reason::Lock)).unwrap();
        assert!(count(&state, Reason::Lock).is_err());
        record(&state, Reason::Lock);
        assert_eq!(fs::read(&target).unwrap(), b"xxxx");
        fs::create_dir(sidecar_path(&state, Reason::WriteFailed)).unwrap();
        assert!(count(&state, Reason::WriteFailed).is_err());
    }

    #[test]
    fn a_full_sidecar_stops_growing_and_reads_as_saturated() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("activity.json");
        fs::write(
            sidecar_path(&state, Reason::Lock),
            vec![b'1'; usize::try_from(MAX_SKIPS).unwrap()],
        )
        .unwrap();
        record(&state, Reason::Lock);
        let n = count(&state, Reason::Lock).unwrap();
        assert_eq!(n, MAX_SKIPS);
        assert!(saturated(n));
        assert!(!saturated(n - 1));
    }
}
