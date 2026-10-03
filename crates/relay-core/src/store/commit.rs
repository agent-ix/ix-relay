//! Atomic session enrollment and hook cache updates.
use super::{AgentName, Registration, Relay, Result, SESSION_TTL_SECS, SessionId, encode};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// Verified enrollment changes applied together.
pub struct IdentityCommit {
    /// Current live session placement.
    pub registration: Option<Registration>,
    /// Genuine session to immutable queue identity.
    pub session_name: Option<(SessionId, AgentName)>,
    /// Non-lead session excluded from enrollment.
    pub suppress_session: Option<SessionId>,
}
#[derive(Debug, Default, PartialEq, Eq)]
/// Successful atomic enrollment outcome.
pub struct CommitOutcome;
impl Relay {
    /// Commit registration and session cache together without partial writes.
    pub fn commit_identity(&mut self, commit: &IdentityCommit) -> Result<CommitOutcome> {
        let mut pipe = redis::pipe();
        pipe.atomic();
        if let Some(reg) = &commit.registration {
            pipe.hset(self.keys.agents_key(), reg.name.as_str(), encode(reg))
                .ignore();
        }
        if let Some((sid, id)) = &commit.session_name {
            pipe.cmd("SET")
                .arg(self.keys.session_key(sid))
                .arg(id.as_str())
                .arg("EX")
                .arg(SESSION_TTL_SECS)
                .ignore();
        }
        if let Some(sid) = &commit.suppress_session {
            pipe.cmd("SET")
                .arg(self.keys.suppressed_key(sid))
                .arg("1")
                .arg("EX")
                .arg(SESSION_TTL_SECS)
                .ignore();
        }
        let (): () = pipe.query(&mut self.conn)?;
        Ok(CommitOutcome)
    }
}
