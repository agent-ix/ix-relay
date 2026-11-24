// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! The hook activity stream and the per-lane last-activity record (FR-068).

use std::{collections::BTreeMap, sync::LazyLock};

use redis::{Commands, streams::StreamRangeReply};

use super::{Relay, Result};
use crate::{
    Address, MAX_LABEL_BYTES,
    activity::{ACTIVITY_LAST_TTL_SECS, ACTIVITY_MAX_AGE_MS, ACTIVITY_MAX_EVENTS, ActivityEvent},
};

static WRITE: LazyLock<String> = LazyLock::new(|| {
    include_str!("lua/activity.lua")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n")
});

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
        let lane = lane.to_string();
        if lane.len() > MAX_LABEL_BYTES {
            return Ok(false);
        }
        // `EVAL` with the body, never `EVALSHA`: a cold server would make the
        // write three sequential requests, and FR-068 allows the hook one.
        let reply: i64 = redis::cmd("EVAL")
            .arg(WRITE.as_str())
            .arg(1)
            .arg(self.keys.activity_key())
            .arg(&lane)
            .arg(event.kind.as_str())
            .arg(event.encode())
            .arg(self.keys.activity_last_stem())
            .arg(ACTIVITY_MAX_EVENTS)
            .arg(ACTIVITY_MAX_AGE_MS)
            .arg(ACTIVITY_LAST_TTL_SECS)
            .query(&mut self.conn)?;
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
