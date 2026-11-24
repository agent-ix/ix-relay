//! The hook's local activity-emit state: the backoff after a failed write and
//! the counts of skipped, dropped and unregistered events (FR-068).
//!
//! One small JSON file beside the failure spool, per host. `relay-watcher`
//! reads the counts for `ix.relay.hook.activity_skipped`. Updates are a
//! read-modify-write under an advisory lock on a sidecar file, taken without
//! ever waiting longer than [`LOCK_WAIT`]: a hook that cannot get the lock
//! skips the update (an uncounted event) rather than delay the agent.

use std::{
    fs::{self, DirBuilder, File, OpenOptions, TryLockError},
    io::{self, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::PathBuf,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

/// The first backoff after a failed write.
const BACKOFF_START_MS: u64 = 5_000;
/// The longest backoff.
const BACKOFF_CAP_MS: u64 = 5 * 60 * 1_000;
/// The longest an update waits for the lock.
const LOCK_WAIT: Duration = Duration::from_millis(50);

/// What the file holds.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ActivityCounts {
    /// The current backoff period, zero when the last write succeeded.
    #[serde(default)]
    pub(crate) backoff_ms: u64,
    /// Writes are skipped until this Unix millisecond.
    #[serde(default)]
    pub(crate) not_before_ms: i64,
    /// Events skipped because of the backoff.
    #[serde(default)]
    pub(crate) skipped: u64,
    /// Events dropped because the write, or the call carrying it, failed.
    #[serde(default)]
    pub(crate) dropped: u64,
    /// Events not written because the lead has no verified lane address.
    #[serde(default)]
    pub(crate) unregistered: u64,
}

/// The state file.
#[derive(Debug, Clone)]
pub(crate) struct ActivityState {
    path: PathBuf,
}

impl ActivityState {
    /// `spool/activity.json` beside `config.toml`; `None` when the config
    /// directory is unknown or relative (same rule as the spool).
    pub(crate) fn in_config_dir() -> Option<Self> {
        let dir = relay_core::config::config_dir()?;
        dir.is_absolute().then(|| Self {
            path: dir.join("spool").join("activity.json"),
        })
    }

    /// A state file at `path`.
    #[cfg(test)]
    pub(crate) fn at(path: PathBuf) -> Self {
        Self { path }
    }

    /// What the file holds; empty when it is missing or unreadable.
    pub(crate) fn counts(&self) -> ActivityCounts {
        fs::read(&self.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// Whether a write at `now_ms` must be skipped; counts the skip when so.
    pub(crate) fn skip(&self, now_ms: i64) -> bool {
        if now_ms >= self.counts().not_before_ms {
            return false;
        }
        self.update(|c| c.skipped = c.skipped.saturating_add(1));
        true
    }

    /// Records a dropped event and starts or doubles the backoff.
    pub(crate) fn failed(&self, now_ms: i64) {
        self.update(|c| {
            c.dropped = c.dropped.saturating_add(1);
            c.backoff_ms = if c.backoff_ms == 0 {
                BACKOFF_START_MS
            } else {
                c.backoff_ms.saturating_mul(2).min(BACKOFF_CAP_MS)
            };
            c.not_before_ms = now_ms.saturating_add(i64::try_from(c.backoff_ms).unwrap_or(0));
        });
    }

    /// Records a dropped event without touching the backoff: the call failed for
    /// a reason that says nothing about the activity write.
    pub(crate) fn dropped(&self) {
        self.update(|c| c.dropped = c.dropped.saturating_add(1));
    }

    /// Records an event not written for want of a verified lane address. The
    /// backoff is untouched: nothing was attempted.
    pub(crate) fn unregistered(&self) {
        self.update(|c| c.unregistered = c.unregistered.saturating_add(1));
    }

    /// Ends the backoff after a successful write.
    pub(crate) fn succeeded(&self) {
        let counts = self.counts();
        if counts.backoff_ms == 0 && counts.not_before_ms == 0 {
            return;
        }
        self.update(|c| {
            c.backoff_ms = 0;
            c.not_before_ms = 0;
        });
    }

    /// Applies `change` to the file under the lock; skipped when the lock is
    /// not free within [`LOCK_WAIT`] or the file cannot be written.
    fn update(&self, change: impl FnOnce(&mut ActivityCounts)) {
        let _ = self.try_update(change);
    }

    fn try_update(&self, change: impl FnOnce(&mut ActivityCounts)) -> io::Result<()> {
        if fs::symlink_metadata(&self.path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "the activity state path is a symlink",
            ));
        }
        if let Some(dir) = self.path.parent() {
            DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        }
        let lock = self.lock()?;
        let mut counts = self.counts();
        change(&mut counts);
        let tmp = self
            .path
            .with_extension(format!("tmp{}", std::process::id()));
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(&serde_json::to_vec(&counts).map_err(io::Error::other)?)?;
        fs::rename(&tmp, &self.path)?;
        drop(lock);
        Ok(())
    }

    /// The advisory lock on the sidecar file, polled for at most [`LOCK_WAIT`].
    fn lock(&self) -> io::Result<File> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .open(self.path.with_extension("lock"))?;
        let started = Instant::now();
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(file),
                Err(TryLockError::Error(e)) => return Err(e),
                Err(TryLockError::WouldBlock) if started.elapsed() >= LOCK_WAIT => {
                    return Err(io::Error::from(io::ErrorKind::WouldBlock));
                }
                Err(TryLockError::WouldBlock) => std::thread::sleep(Duration::from_micros(200)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use ix_trace_rs::trace;

    use super::*;

    #[trace("FR-068-AC-11")]
    #[test]
    fn the_backoff_starts_at_five_seconds_doubles_to_the_cap_and_ends_on_success() {
        let dir = tempfile::tempdir().unwrap();
        let state = ActivityState::at(dir.path().join("spool").join("activity.json"));
        assert!(!state.skip(1_000));
        let mut now = 1_000;
        let mut periods = Vec::new();
        for _ in 0..9 {
            state.failed(now);
            let c = state.counts();
            periods.push(c.backoff_ms);
            assert!(state.skip(now + 1), "skipped inside the period");
            assert!(!state.skip(c.not_before_ms), "writes resume after it");
            now = c.not_before_ms;
        }
        assert_eq!(
            periods,
            [
                5_000, 10_000, 20_000, 40_000, 80_000, 160_000, 300_000, 300_000, 300_000
            ]
        );
        state.succeeded();
        assert_eq!(state.counts().backoff_ms, 0);
        assert!(!state.skip(now + 1));
        let c = state.counts();
        assert_eq!((c.dropped, c.skipped), (9, 9));
        state.failed(now);
        assert_eq!(state.counts().backoff_ms, 5_000, "starts over");
    }

    /// An unregistered lead is counted and neither starts nor ends a backoff.
    #[trace("FR-068-AC-11")]
    #[test]
    fn an_unregistered_event_is_counted_and_leaves_the_backoff_alone() {
        let dir = tempfile::tempdir().unwrap();
        let state = ActivityState::at(dir.path().join("activity.json"));
        state.failed(1_000);
        state.unregistered();
        state.unregistered();
        state.dropped();
        let c = state.counts();
        assert_eq!((c.unregistered, c.dropped, c.backoff_ms), (2, 2, 5_000));
        state.unregistered();
        assert_eq!(state.counts().backoff_ms, 5_000);
    }

    /// Concurrent hooks lose no increment, and a held lock skips the update
    /// without waiting past the bound.
    #[trace("FR-068-AC-11")]
    #[test]
    fn concurrent_updates_are_exact_and_a_held_lock_never_blocks() {
        let dir = tempfile::tempdir().unwrap();
        let state = ActivityState::at(dir.path().join("activity.json"));
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    for _ in 0..10 {
                        state.unregistered();
                    }
                });
            }
        });
        assert_eq!(state.counts().unregistered, 40);
        let held = state.lock().unwrap();
        let started = Instant::now();
        state.unregistered();
        assert!(started.elapsed() < Duration::from_millis(500));
        assert_eq!(state.counts().unregistered, 40, "skipped, not counted");
        drop(held);
        state.unregistered();
        assert_eq!(state.counts().unregistered, 41);
    }
}
