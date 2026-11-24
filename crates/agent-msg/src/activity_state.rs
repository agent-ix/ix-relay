//! The hook's local activity-emit state: the backoff after a failed write and
//! the counts of skipped and dropped events (FR-068).
//!
//! One small JSON file beside the failure spool, per host. `relay-watcher`
//! reads the counts for `ix.relay.hook.activity_skipped`. Hooks race on the
//! file without a lock, so a count can lose an increment under concurrent
//! hooks; the backoff still converges because every failure rewrites it.

use std::{
    fs::{self, DirBuilder},
    io::{self, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::PathBuf,
};

use serde::{Deserialize, Serialize};

/// The first backoff after a failed write.
const BACKOFF_START_MS: u64 = 5_000;
/// The longest backoff.
const BACKOFF_CAP_MS: u64 = 5 * 60 * 1_000;

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
    /// Events dropped because the write failed or had no time.
    #[serde(default)]
    pub(crate) dropped: u64,
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
        let mut counts = self.counts();
        if now_ms >= counts.not_before_ms {
            return false;
        }
        counts.skipped = counts.skipped.saturating_add(1);
        let _ = self.store(&counts);
        true
    }

    /// Records a dropped event and starts or doubles the backoff.
    pub(crate) fn failed(&self, now_ms: i64) {
        let mut counts = self.counts();
        counts.dropped = counts.dropped.saturating_add(1);
        counts.backoff_ms = if counts.backoff_ms == 0 {
            BACKOFF_START_MS
        } else {
            counts.backoff_ms.saturating_mul(2).min(BACKOFF_CAP_MS)
        };
        counts.not_before_ms = now_ms.saturating_add(i64::try_from(counts.backoff_ms).unwrap_or(0));
        let _ = self.store(&counts);
    }

    /// Ends the backoff after a successful write.
    pub(crate) fn succeeded(&self) {
        let mut counts = self.counts();
        if counts.backoff_ms == 0 && counts.not_before_ms == 0 {
            return;
        }
        counts.backoff_ms = 0;
        counts.not_before_ms = 0;
        let _ = self.store(&counts);
    }

    fn store(&self, counts: &ActivityCounts) -> io::Result<()> {
        if fs::symlink_metadata(&self.path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "the activity state path is a symlink",
            ));
        }
        if let Some(dir) = self.path.parent() {
            DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        }
        let tmp = self
            .path
            .with_extension(format!("tmp{}", std::process::id()));
        let mut file = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(&serde_json::to_vec(counts).map_err(io::Error::other)?)?;
        fs::rename(&tmp, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trace: FR-068-AC-11
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
}
