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
        // One request and one reply: a MULTI pipeline can wait out the socket
        // timeout once per queued reply when Redis accepts but never answers.
        let script = r#"
            if ARGV[1] ~= '' then
                redis.call('HSET', KEYS[1], ARGV[1], ARGV[2])
            end
            if ARGV[3] ~= '' then
                redis.call('SET', KEYS[2], ARGV[3], 'EX', ARGV[4])
            end
            if ARGV[5] == '1' then
                redis.call('SET', KEYS[3], '1', 'EX', ARGV[4])
            end
            return 1
        "#;
        let (name, registration) = commit
            .registration
            .as_ref()
            .map(|r| (r.name.as_str(), encode(r)))
            .unwrap_or(("", String::new()));
        let (session_key, session_name) = commit
            .session_name
            .as_ref()
            .map(|(sid, id)| (self.keys.session_key(sid), id.as_str()))
            .unwrap_or((self.keys.agents_key(), ""));
        let suppressed_key = commit
            .suppress_session
            .as_ref()
            .map(|sid| self.keys.suppressed_key(sid))
            .unwrap_or_else(|| self.keys.agents_key());
        let _: i64 = redis::cmd("EVAL")
            .arg(script)
            .arg(3)
            .arg(self.keys.agents_key())
            .arg(session_key)
            .arg(suppressed_key)
            .arg(name)
            .arg(registration)
            .arg(session_name)
            .arg(SESSION_TTL_SECS)
            .arg(if commit.suppress_session.is_some() {
                "1"
            } else {
                "0"
            })
            .query(&mut self.conn)?;
        Ok(CommitOutcome)
    }
}
