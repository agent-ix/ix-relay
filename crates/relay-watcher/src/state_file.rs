//! The on-disk [`WatcherFile`]: `~/.ix/relay/state/watcher.json`.

use std::{fs, io, path::PathBuf};

use crate::supervise::{StateSink, WatcherFile};

/// File name of the watcher's state inside [`relay_core::config::state_dir`].
pub const FILE_NAME: &str = relay_core::health::WATCHER_FILE_NAME;

/// Writes the state file atomically: a temp file beside it, then a rename, so a
/// reader never sees a half-written file.
pub struct StateFile {
    path: PathBuf,
}

impl StateFile {
    /// A sink writing to `path`; its directory is created on first write.
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// The temp file a write goes through: named for this process so two
    /// watchers sharing a state directory never rename each other's
    /// half-written file.
    fn tmp_path(&self) -> PathBuf {
        let mut tmp = self.path.clone().into_os_string();
        tmp.push(format!(".{}.tmp", std::process::id()));
        PathBuf::from(tmp)
    }

    /// The default location, honouring `IX_RELAY_CONFIG` (see
    /// [`relay_core::config::state_dir`]); `None` when neither it nor `HOME` is
    /// set.
    #[must_use]
    pub fn default_location() -> Option<Self> {
        relay_core::config::state_dir().map(|dir| Self::new(dir.join(FILE_NAME)))
    }
}

impl StateSink for StateFile {
    fn write(&mut self, file: &WatcherFile) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = self.tmp_path();
        fs::write(
            &tmp,
            serde_json::to_vec_pretty(file).map_err(io::Error::other)?,
        )?;
        fs::rename(&tmp, &self.path)
    }
}

/// With no resolvable location (no `HOME`, no `IX_RELAY_CONFIG`) the state
/// file is simply not kept; `main` says so once at startup.
impl StateSink for Option<StateFile> {
    fn write(&mut self, file: &WatcherFile) -> io::Result<()> {
        match self {
            Some(sink) => sink.write(file),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::supervise::LastError;

    fn sample(ok: Option<i64>) -> WatcherFile {
        WatcherFile {
            pid: 4242,
            started_at_ms: 1_000,
            last_ok_tick_ms: ok,
            last_error: Some(LastError {
                at_ms: 2_000,
                message: "redis: refused".into(),
            }),
        }
    }

    #[test]
    fn writes_atomically_and_replaces_the_previous_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state").join(FILE_NAME);
        let mut sink = StateFile::new(path.clone());
        sink.write(&sample(None)).unwrap();
        sink.write(&sample(Some(3_000))).unwrap();

        let read: WatcherFile = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(read, sample(Some(3_000)));
        let mut names: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        names.sort();
        assert_eq!(names, [FILE_NAME], "no temp file is left behind");
    }

    #[test]
    fn the_temp_file_is_named_for_this_process() {
        let sink = StateFile::new(PathBuf::from("/x/watcher.json"));
        assert_eq!(
            sink.tmp_path(),
            PathBuf::from(format!("/x/watcher.json.{}.tmp", std::process::id()))
        );
    }

    /// Every key of the file, nested ones included, spelled out.
    #[test]
    fn the_file_has_exactly_the_documented_fields() {
        assert_eq!(
            serde_json::to_value(sample(Some(5))).unwrap(),
            serde_json::json!({
                "pid": 4242,
                "started_at_ms": 1000,
                "last_ok_tick_ms": 5,
                "last_error": {"at_ms": 2000, "message": "redis: refused"},
            })
        );
        assert_eq!(
            serde_json::to_value(WatcherFile {
                last_error: None,
                ..sample(None)
            })
            .unwrap(),
            serde_json::json!({
                "pid": 4242,
                "started_at_ms": 1000,
                "last_ok_tick_ms": null,
                "last_error": null,
            })
        );
    }

    #[test]
    fn an_unwritable_location_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("file");
        fs::write(&blocker, b"x").unwrap();
        let mut sink = StateFile::new(blocker.join("sub").join(FILE_NAME));
        assert!(sink.write(&sample(None)).is_err());
    }
}
