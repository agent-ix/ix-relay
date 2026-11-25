//! Whether a Claude Code session's mail belongs to its inbox socket (FR-044).
//!
//! A session that has an inbox socket receives relay mail only through it, so its
//! hooks register the lead and leave the mail to the watcher. A session with no
//! socket keeps hook delivery.

use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use claude_wire::SessionRecord;
use relay_core::SessionId;

/// The most bytes read from one registry file.
const MAX_RECORD_BYTES: u64 = 64 * 1024;

/// The most registry files looked at in one lookup.
const MAX_RECORDS: usize = 4096;

/// What a Claude Code hook can see about its session's inbox socket.
#[derive(Debug, Clone, Default)]
pub(crate) struct ClaudeInbox {
    /// Claude Code gave the hook's process a socket address
    /// (`CLAUDE_CODE_MESSAGING_SOCKET`), which it does once the socket is bound.
    pub(crate) env_socket: bool,
    /// Claude Code's session registry, read only: it covers a hook that runs before
    /// the address is in the environment.
    pub(crate) sessions_dir: Option<PathBuf>,
}

impl ClaudeInbox {
    /// This process's view: the environment, and the registry under
    /// `$IX_RELAY_CLAUDE_DIR` (else `~/.claude`).
    pub(crate) fn from_env() -> Self {
        let claude = std::env::var_os("IX_RELAY_CLAUDE_DIR")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".claude")));
        Self {
            env_socket: std::env::var_os("CLAUDE_CODE_MESSAGING_SOCKET")
                .is_some_and(|v| !v.is_empty()),
            sessions_dir: claude.map(|dir| dir.join("sessions")),
        }
    }

    /// Whether `session` has an inbox socket.
    pub(crate) fn has_socket(&self, session: Option<&SessionId>) -> bool {
        self.env_socket
            || session.is_some_and(|session| {
                self.sessions_dir
                    .as_deref()
                    .is_some_and(|dir| advertises(dir, session.as_str()))
            })
    }
}

/// Whether any record in `dir` names `session` together with a socket.
fn advertises(dir: &Path, session: &str) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|x| x == "json"))
        .take(MAX_RECORDS)
        .filter_map(|path| read_record(&path))
        .any(|record| record.session_id == session && record.messaging_socket_path.is_some())
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Tracing: TC-044-17
    /// ACs: FR-044-AC-17
    #[test]
    fn tc_044_17_a_session_has_a_socket_by_its_environment_or_its_own_record() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, body: &str| fs::write(dir.path().join(name), body).unwrap();
        write(
            "10.json",
            r#"{"pid":10,"sessionId":"with","messagingSocketPath":"/tmp/cc-socks/10.sock"}"#,
        );
        write("11.json", r#"{"pid":11,"sessionId":"without"}"#);
        write("12.json", "not json");
        let session = |s: &str| s.parse::<SessionId>().unwrap();
        let inbox = ClaudeInbox {
            env_socket: false,
            sessions_dir: Some(dir.path().to_path_buf()),
        };
        assert!(inbox.has_socket(Some(&session("with"))));
        assert!(!inbox.has_socket(Some(&session("without"))));
        assert!(!inbox.has_socket(Some(&session("unknown"))));
        assert!(!inbox.has_socket(None));
        let env = ClaudeInbox {
            env_socket: true,
            sessions_dir: None,
        };
        assert!(env.has_socket(Some(&session("without"))));
        assert!(env.has_socket(None));
        let nothing = ClaudeInbox::default();
        assert!(!nothing.has_socket(Some(&session("with"))));
    }
}
