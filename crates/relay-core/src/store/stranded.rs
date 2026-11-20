//! Mail stranded in the inbox of a registration whose session left its pane
//! (FR-053): handed to the registration that now holds the address, or
//! dead-lettered with the registration when nothing does.
//!
//! Both moves are one script each, a compare-and-set against the registrations
//! the caller read (as a relink is, FR-043) that follows the store's script
//! discipline (`lua/common.lua`, FR-020): it checks the type of every key it may
//! write before its first write and writes the destination before it deletes the
//! source, so a script that fails part-way leaves a message twice, never in none.

use std::sync::LazyLock;

use redis::Commands;

use super::{DEAD_LETTER_CAP, Relay, Result, decode_bytes, inflight::script};
use crate::{error::Error, message::Registration};

pub(super) static FORWARD: LazyLock<redis::Script> = LazyLock::new(|| {
    redis::Script::new(&script(
        include_str!("lua/common.lua"),
        include_str!("lua/forward.lua"),
    ))
});

pub(super) static BURY: LazyLock<redis::Script> = LazyLock::new(|| {
    redis::Script::new(&script(
        include_str!("lua/common.lua"),
        include_str!("lua/bury.lua"),
    ))
});

/// What [`Relay::dead_letter_stranded`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Buried {
    /// The mail is in the dead-letter list (this many messages) and the
    /// registration is pruned.
    Done(usize),
    /// The registration was pruned or registered again since it was read: nothing
    /// changed.
    Changed,
    /// A batch of the lead is in flight: nothing changed, and the lease will return
    /// it to the inbox.
    InFlight,
}

impl Relay {
    /// Reads the stored record of `seen`, and returns its bytes only if it still
    /// decodes to exactly `seen`: the first half of the compare-and-set, the script
    /// being the second.
    fn unchanged_record(&mut self, seen: &Registration) -> Result<Option<Vec<u8>>> {
        let key = self.keys.agents_key();
        let raw: Option<Vec<u8>> = self.conn.hget(&key, seen.name.as_str())?;
        let Some(raw) = raw else {
            return Ok(None);
        };
        let stored: Registration = decode_bytes(&key, &raw)?;
        Ok((stored == *seen).then_some(raw))
    }

    /// Moves every queued message of `from` to the front of `to`'s inbox, in
    /// order, each marked as sent to a prior session of this address
    /// ([`crate::Message::forwarded`]), and returns how many moved. Messages in
    /// flight are not queued and stay (their lease returns them to `from`'s inbox).
    ///
    /// Compare-and-set against both registrations as the caller read them:
    /// `None`, with nothing changed, when either was pruned or changed since.
    ///
    /// # Errors
    ///
    /// [`Error::Redis`] when Redis fails or times out; [`Error::Decode`] when a stored
    /// record is not a registration; [`Error::MoveUnreadable`] when a key of either
    /// lead holds the wrong type (nothing changed).
    pub fn forward_stranded(
        &mut self,
        from: &Registration,
        to: &Registration,
    ) -> Result<Option<usize>> {
        self.forward_stranded_between(from, to, |_| Ok(()))
    }

    /// [`Relay::forward_stranded`] running `between` after the read and before the
    /// script: the seam a test uses to change a registration in exactly the window
    /// the script guards.
    pub(super) fn forward_stranded_between(
        &mut self,
        from: &Registration,
        to: &Registration,
        between: impl FnOnce(&mut Self) -> Result<()>,
    ) -> Result<Option<usize>> {
        if from.name == to.name {
            return Ok(None);
        }
        let (Some(from_raw), Some(to_raw)) =
            (self.unchanged_record(from)?, self.unchanged_record(to)?)
        else {
            return Ok(None);
        };
        between(self)?;
        let (status, moved): (i64, usize) = FORWARD
            .key(self.keys.agents_key())
            .key(self.keys.inbox_key(&from.name))
            .key(self.keys.inbox_key(&to.name))
            .arg(from.name.as_str())
            .arg(from_raw)
            .arg(to.name.as_str())
            .arg(to_raw)
            .invoke(&mut self.conn)?;
        match status {
            1 => Ok(Some(moved)),
            0 => Ok(None),
            -1 => Err(Error::MoveUnreadable {
                lead: from.name.clone(),
            }),
            other => Err(Error::UnexpectedScriptReply {
                script: "forward_stranded",
                reply: other.to_string(),
            }),
        }
    }

    /// Moves every queued message of `seen` to its dead-letter list, byte for byte
    /// (the reason, `recipient-pruned`, is the caller's to record: the list has no
    /// reason field), and prunes the registration, as one compare-and-set against
    /// the registration the caller read.
    ///
    /// # Errors
    ///
    /// [`Error::Redis`] when Redis fails or times out; [`Error::Decode`] when the
    /// stored record is not a registration; [`Error::MoveUnreadable`] when a key of
    /// the lead holds the wrong type: nothing is moved and nothing pruned.
    pub fn dead_letter_stranded(&mut self, seen: &Registration) -> Result<Buried> {
        self.dead_letter_stranded_between(seen, |_| Ok(()))
    }

    /// [`Relay::dead_letter_stranded`] running `between` after the read and before
    /// the script.
    pub(super) fn dead_letter_stranded_between(
        &mut self,
        seen: &Registration,
        between: impl FnOnce(&mut Self) -> Result<()>,
    ) -> Result<Buried> {
        let Some(raw) = self.unchanged_record(seen)? else {
            return Ok(Buried::Changed);
        };
        between(self)?;
        let (status, moved): (i64, usize) = BURY
            .key(self.keys.agents_key())
            .key(self.keys.inbox_key(&seen.name))
            .key(self.keys.inflight_key(&seen.name))
            .key(self.keys.dead_letter_key(&seen.name))
            .arg(seen.name.as_str())
            .arg(raw)
            .arg(DEAD_LETTER_CAP)
            .invoke(&mut self.conn)?;
        match status {
            1 => Ok(Buried::Done(moved)),
            0 => Ok(Buried::Changed),
            2 => Ok(Buried::InFlight),
            -1 => Err(Error::MoveUnreadable {
                lead: seen.name.clone(),
            }),
            other => Err(Error::UnexpectedScriptReply {
                script: "dead_letter_stranded",
                reply: other.to_string(),
            }),
        }
    }
}
