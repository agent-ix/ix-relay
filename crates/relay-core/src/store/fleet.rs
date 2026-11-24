//! The per-host lane lists of the fleet registry (FR-069). Additive: one hash,
//! one field per machine, stamped with the relay clock.

use redis::Commands;

use super::{Relay, decode_bytes, encode};
use crate::{
    Error, MachineName, Result,
    fleet::{HostLaneList, MAX_HOST_LANES},
};

/// Every host's lane list, as read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaneLists {
    /// The lists that decoded.
    pub lists: Vec<HostLaneList>,
    /// Entries that did not decode and were left out.
    pub unreadable: usize,
    /// The relay clock at the read, Unix milliseconds.
    pub now_ms: i64,
}

impl Relay {
    fn relay_time_ms(&mut self) -> Result<i64> {
        let (seconds, micros): (u64, u64) = redis::cmd("TIME").query(&mut self.conn)?;
        seconds
            .checked_mul(1_000)
            .and_then(|ms| ms.checked_add(micros / 1_000))
            .and_then(|ms| i64::try_from(ms).ok())
            .ok_or_else(|| Error::InvalidName {
                name: "relay clock overflow".into(),
            })
    }

    /// Publishes `lanes` as `machine`'s lane list, stamped with the relay clock
    /// and cut to [`MAX_HOST_LANES`]. The record does not expire: its lease is
    /// read from the stamp, so a lapsed host's lanes stay listed as stale.
    ///
    /// # Errors
    ///
    /// [`Error::Redis`] when Redis fails or times out.
    pub fn publish_lane_list(
        &mut self,
        machine: &MachineName,
        lanes: &[String],
    ) -> Result<HostLaneList> {
        let list = HostLaneList {
            machine: machine.clone(),
            refreshed_ms: self.relay_time_ms()?,
            lanes: lanes.iter().take(MAX_HOST_LANES).cloned().collect(),
        };
        let () = self
            .conn
            .hset(self.keys.fleet_lanes_key(), machine.as_str(), encode(&list))?;
        self.host_refreshed(machine.as_str())?;
        Ok(list)
    }

    /// Every host's lane list and the relay clock.
    ///
    /// # Errors
    ///
    /// [`Error::Redis`] when Redis fails or times out.
    pub fn lane_lists(&mut self) -> Result<LaneLists> {
        let key = self.keys.fleet_lanes_key();
        let raw: Vec<(Vec<u8>, Vec<u8>)> = self.conn.hgetall(&key)?;
        let now_ms = self.relay_time_ms()?;
        let mut out = LaneLists {
            now_ms,
            ..LaneLists::default()
        };
        for (_, value) in raw {
            match decode_bytes::<HostLaneList>(&key, &value) {
                Ok(list) => out.lists.push(list),
                Err(_) => out.unreadable += 1,
            }
        }
        out.lists.sort_by(|a, b| a.machine.cmp(&b.machine));
        Ok(out)
    }
}
