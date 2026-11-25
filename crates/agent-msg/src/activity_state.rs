//! The hook's local activity-emit state: the backoff after a failed write and
//! the counts of skipped, dropped and unregistered events (FR-068).
//!
//! One small JSON file beside the failure spool, per host. `relay-watcher`
//! reads the counts for `ix.relay.hook.activity_skipped`. Updates are a
//! read-modify-write under an advisory lock on a sidecar file, taken without
//! ever waiting longer than [`LOCK_WAIT`]: a hook that cannot get the lock
//! skips the update rather than delay the agent. A skipped update cannot be counted
//! in the file it could not lock, so it appends one byte to a sibling file with
//! `O_APPEND`, which needs no lock, and the count is that file's length.

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
/// The most lock skips the sidecar file records, one byte each.
const MAX_LOCK_SKIPS: u64 = 1 << 20;

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
    /// Updates that could not take the lock or write the file, counted in the
    /// sidecar file and never stored in the JSON.
    #[serde(skip)]
    pub(crate) lock_skipped: u64,
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

    /// The sidecar file whose length is the count of dropped updates.
    pub(crate) fn skips_file(&self) -> PathBuf {
        skips_path(&self.path)
    }

    /// What the file holds, with the lock skips; empty when both are missing or
    /// unreadable.
    pub(crate) fn counts(&self) -> ActivityCounts {
        let mut counts: ActivityCounts = fs::read(&self.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        counts.lock_skipped = lock_skips_at(&self.path);
        counts
    }

    /// Counts one update that was dropped. It takes no lock and never waits: a
    /// single `O_APPEND` write of one byte.
    fn note_lock_skip(&self) {
        let sidecar = skips_path(&self.path);
        if fs::symlink_metadata(&sidecar).is_ok_and(|m| !m.file_type().is_file()) {
            return;
        }
        if lock_skips_at(&self.path) >= MAX_LOCK_SKIPS {
            return;
        }
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&sidecar)
        {
            let _ = file.write_all(b"1");
        }
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

    /// Applies `change` to the file under the lock; when the lock is not free
    /// within [`LOCK_WAIT`] or the file cannot be written, the update is dropped
    /// and counted as a lock skip.
    fn update(&self, change: impl FnOnce(&mut ActivityCounts)) {
        if self.try_update(change).is_err() {
            self.note_lock_skip();
        }
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

/// The sidecar file that counts dropped updates, beside the state file.
fn skips_path(state: &std::path::Path) -> PathBuf {
    state.with_extension("lockskips")
}

/// The lock skips recorded beside the state file at `state`: the sidecar's length.
fn lock_skips_at(state: &std::path::Path) -> u64 {
    fs::symlink_metadata(skips_path(state))
        .ok()
        .filter(|m| m.file_type().is_file())
        .map_or(0, |m| m.len())
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

    /// Concurrent hooks lose no applied update: the file holds exactly the updates that
    /// took the lock, and an update that finds the lock held is dropped after the
    /// 50 ms bound, not after the holder lets go, and is counted as a lock skip.
    #[trace("FR-068-AC-14", "FR-068-AC-15")]
    #[test]
    fn concurrent_updates_apply_exactly_the_counted_ones_and_a_held_lock_drops_within_the_bound() {
        let dir = tempfile::tempdir().unwrap();
        let state = ActivityState::at(dir.path().join("activity.json"));
        let counted = std::sync::atomic::AtomicU64::new(0);
        let skipped = std::sync::atomic::AtomicU64::new(0);
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    for _ in 0..10 {
                        let outcome = state.try_update(|c| c.unregistered += 1);
                        let tally = if outcome.is_ok() { &counted } else { &skipped };
                        tally.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                });
            }
        });
        // A loaded host can keep the lock past the bound, and that update is skipped by
        // design; what must hold is that every update that got the lock is in the file.
        let (counted, skipped) = (counted.into_inner(), skipped.into_inner());
        assert_eq!(counted + skipped, 40);
        assert!(counted > 0, "every update skipping would pass the sum");
        assert_eq!(state.counts().unregistered, counted);
        let base = counted;
        assert_eq!(
            state.counts().lock_skipped,
            skipped,
            "each concurrent skip is counted"
        );
        let held = state.lock().unwrap();
        let started = Instant::now();
        state.unregistered();
        let waited = started.elapsed();
        assert!(
            waited >= LOCK_WAIT,
            "it waited the bound before giving up: {waited:?}"
        );
        // The margin is scheduling slack on a loaded host, not part of the bound.
        assert!(
            waited < LOCK_WAIT + Duration::from_millis(450),
            "{waited:?}"
        );
        assert_eq!(state.counts().unregistered, base, "not applied");
        assert_eq!(
            state.counts().lock_skipped,
            skipped + 1,
            "counted as a skip"
        );
        drop(held);
        state.unregistered();
        assert_eq!(state.counts().unregistered, base + 1);
        assert_eq!(state.counts().lock_skipped, skipped + 1, "no further skip");
    }

    /// Separate hook processes share the count through the sidecar file, and a
    /// skipped update leaves the JSON untouched.
    #[trace("FR-068-AC-15")]
    #[test]
    fn lock_skips_from_separate_processes_add_up_in_the_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("activity.json");
        let (a, b) = (ActivityState::at(path.clone()), ActivityState::at(path));
        let held = a.lock().unwrap();
        a.unregistered();
        b.unregistered();
        b.dropped();
        assert_eq!(a.counts().lock_skipped, 3);
        assert_eq!(b.counts().lock_skipped, 3);
        drop(held);
        assert_eq!((a.counts().unregistered, a.counts().dropped), (0, 0));
    }
}
