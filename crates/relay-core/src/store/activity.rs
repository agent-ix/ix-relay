// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! The hook activity stream and the per-lane last-activity record (FR-068).

use std::{collections::BTreeMap, sync::LazyLock};

use redis::{Commands, streams::StreamRangeReply};

use super::{
    Relay, Result,
    inflight::{CachedScript, slim_script},
};
use crate::{
    Address, MAX_LABEL_BYTES,
    activity::{ACTIVITY_LAST_TTL_SECS, ACTIVITY_MAX_AGE_MS, ACTIVITY_MAX_EVENTS, ActivityEvent},
};

pub(super) static WRITE: LazyLock<CachedScript> =
    LazyLock::new(|| CachedScript::new(slim_script(include_str!("lua/activity.lua"))));

impl Relay {
    /// Appends `event` for the lane at `lane` to the activity stream and updates
    /// the lane's last-activity record, in one request. `lane` is the lead's
    /// verified address ([`crate::Registration::verified_place`]); the relay adds
    /// its own server time, trims the stream to its count and age caps in the
    /// same request, and renews the record's expiry.
    ///
    /// Returns `false`, writing nothing, when `lane` is longer than
    /// [`crate::MAX_LABEL_BYTES`], the longest address the bus prints.
    ///
    /// # Errors
    /// Fails when Redis cannot be reached or answers with an error.
    pub fn record_activity(&mut self, lane: &Address, event: &ActivityEvent) -> Result<bool> {
        let machine = lane.server().to_owned();
        let lane = lane.to_string();
        if lane.len() > MAX_LABEL_BYTES {
            return Ok(false);
        }
        // By SHA: the body goes only to a server that does not know it yet, so the
        // write is one small request, and two only on the first call after a server start.
        let (activity, last_stem) = (self.keys.activity_key(), self.keys.activity_last_stem());
        let (stream, meta) = (self.keys.receipt_stream_key(), self.keys.receipt_meta_key());
        let encoded = event.encode();
        let queue = queue_hex(&machine, &event.session);
        let acts = if event.delivered_mail { "0" } else { "1" };
        let reply: i64 = WRITE.run(&mut self.conn, |cmd| {
            cmd.arg(1)
                .arg(&activity)
                .arg(&lane)
                .arg(event.kind.as_str())
                .arg(&encoded)
                .arg(&last_stem)
                .arg(ACTIVITY_MAX_EVENTS)
                .arg(ACTIVITY_MAX_AGE_MS)
                .arg(ACTIVITY_LAST_TTL_SECS)
                .arg(&queue)
                .arg(&event.session)
                .arg(acts)
                .arg(&stream)
                .arg(&meta);
        })?;
        Ok(reply == 1)
    }

    /// The newest `count` activity events, oldest first, as stored JSON.
    ///
    /// # Errors
    /// Fails when Redis cannot be reached or answers with an error.
    pub fn activity_events(&mut self, count: usize) -> Result<Vec<String>> {
        let reply: StreamRangeReply =
            self.conn
                .xrevrange_count(self.keys.activity_key(), "+", "-", count)?;
        Ok(reply
            .ids
            .into_iter()
            .rev()
            .filter_map(|entry| entry.get::<String>("event"))
            .collect())
    }

    /// How many events the activity stream holds.
    ///
    /// # Errors
    /// Fails when Redis cannot be reached or answers with an error.
    pub fn activity_len(&mut self) -> Result<usize> {
        Ok(self.conn.xlen(self.keys.activity_key())?)
    }

    /// The lane's last-activity record: the newest event of each kind under
    /// `kind:<kind>` and the newest overall under `any`, as stored JSON.
    ///
    /// # Errors
    /// Fails when Redis cannot be reached or answers with an error.
    pub fn activity_last(&mut self, lane: &Address) -> Result<BTreeMap<String, String>> {
        Ok(self.conn.hgetall(self.keys.activity_last_key(lane))?)
    }
}

/// Most lanes one pipelined read of last-activity records carries.
const ACTIVITY_READ_BATCH: usize = 128;

impl Relay {
    /// The last-activity records of `lanes`, in order, read in pipelined batches
    /// of at most 128 lanes (one round trip each) under one total `budget`: the
    /// connection's timeout is re-armed to what is left before each batch and put
    /// back afterwards, and a budget that runs out is a timeout error.
    ///
    /// # Errors
    /// Fails when Redis cannot be reached, answers with an error, or the budget
    /// runs out.
    pub fn activity_last_many(
        &mut self,
        lanes: &[Address],
        budget: std::time::Duration,
    ) -> Result<Vec<BTreeMap<String, String>>> {
        let armed = self.timeout.get();
        let began = std::time::Instant::now();
        let read = (|| {
            let mut records = Vec::with_capacity(lanes.len());
            for batch in lanes.chunks(ACTIVITY_READ_BATCH) {
                let left = budget.saturating_sub(began.elapsed());
                if left.is_zero() {
                    return Err(crate::Error::Redis(redis::RedisError::from(
                        std::io::Error::new(std::io::ErrorKind::TimedOut, "activity read budget"),
                    )));
                }
                self.set_timeout(left)?;
                let mut pipe = redis::pipe();
                for lane in batch {
                    pipe.hgetall(self.keys.activity_last_key(lane));
                }
                let replies: Vec<BTreeMap<String, String>> = pipe.query(&mut self.conn)?;
                records.extend(replies);
            }
            Ok(records)
        })();
        let restored = self.set_timeout(armed);
        let records = read?;
        restored?;
        Ok(records)
    }
}

/// The machine and the session as the hex a session queue's name is made of
/// ([`crate::AgentName::for_session`]), joined by a colon; empty for a session id too
/// long to be one.
fn queue_hex(machine: &str, session: &str) -> String {
    if session.len() > crate::activity::ACTIVITY_MAX_FIELD_BYTES {
        return String::new();
    }
    let hex = |text: &str| text.bytes().map(|b| format!("{b:02x}")).collect::<String>();
    format!("{}:{}", hex(machine), hex(session))
}
