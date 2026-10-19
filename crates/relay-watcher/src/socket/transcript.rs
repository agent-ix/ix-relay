//! The strong acknowledgement: our `msg_id` in the lead's transcript.
//!
//! When Claude Code consumes a peer message it records
//! `origin.msg_id` in `~/.claude/projects/<slug>/<sessionId>.jsonl` (only when the
//! id is a UUID, measured on 2.1.283). This only ever reads that file, and only
//! its end: a transcript can be hundreds of megabytes.

use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use claude_wire::{Scan, TranscriptMatch, scan_detail};

/// The most bytes read from the end of a transcript in one read: the message was
/// just consumed, so it is near the end.
pub const TAIL_BYTES: u64 = 512 * 1024;

/// Whether `session` is safe as a file name: Claude Code's ids are UUIDs, and
/// anything else could reach outside the projects directory.
#[must_use]
pub fn is_plain_id(session: &str) -> bool {
    !session.is_empty()
        && session
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The transcript of `session` under `projects_dir`, whichever project
/// directory Claude Code filed it in (the directory name is derived from the
/// working directory in ways not worth reproducing).
#[must_use]
pub fn find(projects_dir: &Path, session: &str) -> Option<PathBuf> {
    if !is_plain_id(session) {
        return None;
    }
    let name = format!("{session}.jsonl");
    fs::read_dir(projects_dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path().join(&name))
        .find(|p| p.is_file())
}

/// A poll of one transcript for one id that reads only what was appended since the
/// last look, in windows of at most [`TAIL_BYTES`], and never re-reads a line it has
/// read whole: a transcript that grows by more than a window between two looks is
/// caught up over several looks, so an acknowledgement is not lost to a long gap.
#[derive(Debug)]
pub struct Watch {
    path: PathBuf,
    /// Where the next look starts: the start of the first line not yet read whole.
    seen_to: Option<u64>,
    /// Whether `seen_to` is inside a line (after a line longer than a window was skipped),
    /// so the next window starts with the rest of that line, which is not scanned.
    mid_line: bool,
}

impl Watch {
    /// A watch that starts at the transcript's end as it is now: for a message about
    /// to be posted, which Claude Code records after this.
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        let seen_to = Some(fs::metadata(&path).map_or(0, |m| m.len()));
        Self {
            path,
            seen_to,
            mid_line: false,
        }
    }

    /// A watch of a transcript that did not exist when the message was posted: whatever
    /// it holds was written since.
    #[must_use]
    pub fn from_start(path: PathBuf) -> Self {
        Self {
            path,
            seen_to: Some(0),
            mid_line: false,
        }
    }

    /// A watch that starts at the transcript's tail: for a message posted earlier
    /// (by a watcher that has since restarted), whose place in the file is unknown.
    #[must_use]
    pub fn from_tail(path: PathBuf) -> Self {
        Self {
            path,
            seen_to: None,
            mid_line: false,
        }
    }

    /// What the next window of the transcript says about `msg_id`. A read
    /// error is [`TranscriptMatch::Absent`] ("not yet").
    #[cfg(test)]
    pub fn records(&mut self, msg_id: &str) -> TranscriptMatch {
        self.look(msg_id).found
    }

    /// [`Watch::records`], and the `type` of a line that named the id in a shape not known.
    pub fn look(&mut self, msg_id: &str) -> Scan {
        let absent = || Scan {
            found: TranscriptMatch::Absent,
            line_type: None,
        };
        let Ok(mut file) = fs::File::open(&self.path) else {
            return absent();
        };
        let Ok(len) = file.metadata().map(|m| m.len()) else {
            return absent();
        };
        // A transcript that shrank was replaced: read it from its tail, where a line
        // boundary is not known.
        let (start, known_boundary) = match self.seen_to {
            Some(n) if n <= len => (n, !self.mid_line),
            _ => (len.saturating_sub(TAIL_BYTES), false),
        };
        // Without a known boundary, one byte before the window says whether it starts on a
        // line: a window that starts inside a line drops that partial line (half a line
        // naming the id would read as a shape change), one that starts on a line keeps it.
        let lookback = u64::from(!known_boundary && start > 0);
        if file.seek(SeekFrom::Start(start - lookback)).is_err() {
            return absent();
        }
        let mut bytes = Vec::new();
        if file
            .take(TAIL_BYTES + lookback)
            .read_to_end(&mut bytes)
            .is_err()
        {
            return absent();
        }
        let at_line_start = if lookback == 1 {
            let before = bytes.remove(0);
            before == b'\n'
        } else {
            true
        };
        let read = u64::try_from(bytes.len()).unwrap_or(0);
        // Only whole lines are read; a line still being written waits for the next look.
        let whole = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
        let first = if at_line_start {
            0
        } else {
            bytes
                .iter()
                .position(|b| *b == b'\n')
                .map_or(whole, |i| (i + 1).min(whole))
        };
        let found = scan_detail(&bytes[first..whole], msg_id);
        // Progress: past the whole lines, or past a window that holds none (a line
        // longer than a window is skipped rather than stalled on, and the next window
        // starts inside it).
        if whole > 0 {
            self.seen_to = Some(start + u64::try_from(whole).unwrap_or(read));
            self.mid_line = false;
        } else if read >= TAIL_BYTES {
            self.seen_to = Some(start + read);
            self.mid_line = true;
        } else {
            self.seen_to = Some(start);
            self.mid_line = !at_line_start;
        }
        found
    }
}
