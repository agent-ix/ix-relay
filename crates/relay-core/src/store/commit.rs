//! [`Relay::commit_identity`]: every write a hook makes about *who a session
//! is*, applied as one atomic step.
//!
//! A hook derives an identity from reads (registry, session cache, herdr) and
//! only then writes: the registration, the alias demotions a rename implies,
//! the session-to-name cache, or the "this session never joins the bus" mark.
//! Done as separate commands, a hook that ran out of time midway left half of
//! them behind. Done here, a caller that decides it has no time left simply
//! does not call, and nothing was written.

use super::{
    AgentName, Error, Registration, Relay, Result, SESSION_TTL_SECS, SessionId, decode, encode,
};
use crate::{MachineName, PaneId};

/// One name a session is giving up in favour of another: `old` becomes an
/// alias of `new` when it is safe to do so: `old` is unregistered, or registered to
/// this same session **and** pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Demotion {
    /// The name being demoted.
    pub old: AgentName,
    /// The canonical name it will point at.
    pub new: AgentName,
    /// The demoting session, when known. Ownership of `old` requires it.
    pub session_id: Option<SessionId>,
    /// The demoting session's herdr pane, when known. Ownership requires it.
    pub pane_id: Option<PaneId>,
    /// The machine that pane is on. Pane ids repeat across machines and the
    /// session cache does not know machines, so ownership requires the record to
    /// be on this machine too (compared normalised, see `lua_claim_alias`).
    pub machine: MachineName,
}

/// Everything one hook call writes about identity. The default writes nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdentityCommit {
    /// Alias demotions, applied in order before anything else. Each one that
    /// takes effect also repoints every alias that already pointed at `old`
    /// straight at `new`, so alias resolution never needs more than one hop.
    pub demotions: Vec<Demotion>,
    /// The registration to write. Also drops its name as anyone's alias, like
    /// [`Relay::register`].
    pub registration: Option<Registration>,
    /// `(session id, bus name)` for the session-to-name cache.
    pub session_name: Option<(SessionId, AgentName)>,
    /// A session id to mark suppressed: it never joins the bus.
    pub suppress_session: Option<SessionId>,
}

/// A demotion [`Relay::commit_identity`] declined because the name belongs to
/// a different live session or pane, or to a record nobody can read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefusedDemotion {
    /// The name that was not demoted.
    pub alias: AgentName,
    /// The registration's session id, when it decoded and had one.
    pub session_id: Option<SessionId>,
    /// The registration's herdr pane, when it decoded and had one.
    pub herdr_pane: Option<PaneId>,
}

/// What [`Relay::commit_identity`] applied.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct CommitOutcome {
    /// Demotions that were skipped, for the caller to report. Everything else
    /// in the commit was applied.
    pub refused: Vec<RefusedDemotion>,
}

/// Defines `claim_alias(agents, aliases, alias, canonical, session, pane, machine)`,
/// which sets `alias` -> `canonical` (dropping `alias`'s own registration) only
/// when `alias` is unregistered or registered to this same `session` **and**
/// `pane` **on** `machine` (an empty string is "unknown" and never matches, not even another
/// empty one), and otherwise changes nothing and returns the registration's
/// raw JSON. An undecodable record is never ours.
macro_rules! lua_claim_alias {
    () => {
        r"
        -- The machine rule of relay_core::MachineName::normalise: lowercase (ASCII),
        -- cut at the first '.'. A shared test vector list pins the two together.
        local function normalise_machine(raw)
            return string.match(string.lower(raw), '^[^.]*')
        end
        local function claim_alias(agents, aliases, alias, canonical, session, pane, machine)
            local raw = redis.call('HGET', agents, alias)
            if raw then
                local ok, reg = pcall(cjson.decode, raw)
                local ours = ok and type(reg) == 'table'
                    and session ~= '' and reg.session_id == session
                    and pane ~= '' and reg.herdr_pane == pane
                    and machine ~= '' and type(reg.machine) == 'string'
                    and normalise_machine(reg.machine) == machine
                if not ours then return raw end
            end
            redis.call('HDEL', agents, alias)
            redis.call('HSET', aliases, alias, canonical)
            return nil
        end
        "
    };
}

/// Defines `retarget_aliases(aliases, old, new)`: every alias currently
/// pointing at `old` points at `new` instead.
macro_rules! lua_retarget {
    () => {
        r"
        local function retarget_aliases(aliases, old, new)
            local entries = redis.call('HGETALL', aliases)
            for i = 1, #entries, 2 do
                if entries[i + 1] == old then redis.call('HSET', aliases, entries[i], new) end
            end
        end
        "
    };
}

impl Relay {
    /// Applies `commit` as one `EVAL` (one round trip, never `EVALSHA`): Redis
    /// runs a script without interleaving any other command, so a reader never
    /// sees half of it, and a caller that gives up before the reply either
    /// applied all of it (the script ran) or none (it never arrived). Redis does
    /// not roll a script back if it fails at run time, so that guarantee holds
    /// for a script that runs to the end, which this one does for any input the
    /// types allow. Order inside it: demotions, then the registration, then the
    /// session cache, then the suppression mark.
    ///
    /// A demotion whose `old` name belongs to someone else is skipped, not
    /// fatal; it comes back in [`CommitOutcome::refused`]. A demotion of a name
    /// to itself is a no-op.
    ///
    /// # Errors
    ///
    /// [`Error::Redis`] when Redis fails or times out;
    /// [`Error::UnexpectedScriptReply`] when the script answers with an odd
    /// number of entries.
    pub fn commit_identity(&mut self, commit: &IdentityCommit) -> Result<CommitOutcome> {
        // KEYS: agents, aliases, session key, suppressed key.
        // ARGV: registration name ('' for none), registration JSON, session
        // name ('' for none), ttl, suppress flag ('1'/'0'), demotion count,
        // then per demotion: alias, canonical, session ('' unknown), pane, machine.
        // Returns a flat list: alias, raw registration JSON, alias, ...
        const SCRIPT: &str = concat!(
            lua_retarget!(),
            lua_claim_alias!(),
            r"
            local blocked = {}
            local n = tonumber(ARGV[6])
            for i = 0, n - 1 do
                local b = 7 + i * 5
                local raw = claim_alias(KEYS[1], KEYS[2], ARGV[b], ARGV[b + 1],
                                        ARGV[b + 2], ARGV[b + 3], ARGV[b + 4])
                if raw then
                    blocked[#blocked + 1] = ARGV[b]
                    blocked[#blocked + 1] = raw
                else
                    retarget_aliases(KEYS[2], ARGV[b], ARGV[b + 1])
                end
            end
            if ARGV[1] ~= '' then
                redis.call('HDEL', KEYS[2], ARGV[1])
                redis.call('HSET', KEYS[1], ARGV[1], ARGV[2])
            end
            if ARGV[3] ~= '' then redis.call('SET', KEYS[3], ARGV[3], 'EX', ARGV[4]) end
            if ARGV[5] == '1' then redis.call('SET', KEYS[4], '1', 'EX', ARGV[4]) end
            return blocked"
        );
        let agents_key = self.keys.agents_key();
        // KEYS[3] and KEYS[4] are only touched when ARGV says so; with nothing
        // to write, `agents_key` stands in so no builder is handed an id it
        // does not have.
        let session_key = commit
            .session_name
            .as_ref()
            .map_or_else(|| agents_key.clone(), |(sid, _)| self.keys.session_key(sid));
        let suppressed_key = commit
            .suppress_session
            .as_ref()
            .map_or_else(|| agents_key.clone(), |sid| self.keys.suppressed_key(sid));
        let (reg_name, reg_json) = commit
            .registration
            .as_ref()
            .map_or((String::new(), String::new()), |reg| {
                (reg.name.as_str().to_owned(), encode(reg))
            });
        let demotions: Vec<&Demotion> =
            commit.demotions.iter().filter(|d| d.old != d.new).collect();
        let mut call = redis::cmd("EVAL");
        call.arg(SCRIPT)
            .arg(4)
            .arg(&agents_key)
            .arg(self.keys.aliases_key())
            .arg(&session_key)
            .arg(&suppressed_key)
            .arg(reg_name)
            .arg(reg_json)
            .arg(
                commit
                    .session_name
                    .as_ref()
                    .map_or("", |(_, name)| name.as_str()),
            )
            .arg(SESSION_TTL_SECS)
            .arg(if commit.suppress_session.is_some() {
                "1"
            } else {
                "0"
            })
            .arg(demotions.len());
        for d in demotions {
            call.arg(d.old.as_str())
                .arg(d.new.as_str())
                .arg(d.session_id.as_ref().map_or("", SessionId::as_str))
                .arg(d.pane_id.as_ref().map_or("", PaneId::as_str))
                .arg(d.machine.as_str());
        }
        let reply: Vec<String> = call.query(&mut self.conn)?;
        refused_from(&agents_key, &reply)
    }
}

/// Decodes the script's flat `alias, raw, alias, raw, ...` reply.
fn refused_from(agents_key: &str, reply: &[String]) -> Result<CommitOutcome> {
    let mut refused = Vec::with_capacity(reply.len() / 2);
    for pair in reply.chunks(2) {
        let [alias, raw] = pair else {
            return Err(Error::UnexpectedScriptReply {
                script: "commit_identity",
                reply: format!("{reply:?}"),
            });
        };
        let alias: AgentName = alias.parse()?;
        // An undecodable record is refused too; it just has nothing to report.
        let existing: Option<Registration> = decode(agents_key, raw).ok();
        refused.push(RefusedDemotion {
            alias,
            session_id: existing.as_ref().and_then(|r| r.session_id.clone()),
            herdr_pane: existing.and_then(|r| r.herdr_pane),
        });
    }
    Ok(CommitOutcome { refused })
}
