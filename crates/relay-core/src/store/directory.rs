//! Leased authoritative local topology snapshots, resolved through Redis only.
use std::time::Duration;

use redis::Commands;
use serde::{Deserialize, Serialize};

use super::{LOG_CAP, MAX_MESSAGE_BYTES, Relay, encode};
use crate::{Address, AgentName, Error, MachineName, Message, Registration, Result, Target};

/// A failed observer cannot extend this lease; successful sweeps renew it.
pub const DIRECTORY_LEASE: Duration = Duration::from_secs(90);

#[derive(Debug, Serialize, Deserialize)]
struct Publication {
    revision: u64,
    registrations: Vec<Registration>,
}

fn validate_entries(machine: &MachineName, registrations: &[Registration]) -> Result<()> {
    for registration in registrations {
        let valid = registration.session_id.as_ref().is_some_and(|session| {
            let Some(address) = registration.full_address() else {
                return false;
            };
            let text = address.to_string();
            registration.machine == *machine
                && registration.name == AgentName::for_session(machine, session)
                && text.len() <= crate::MAX_LABEL_BYTES
                && text.split(':').count() == 3
                && text
                    .split(':')
                    .all(|segment| !segment.is_empty() && !segment.chars().any(char::is_control))
        });
        if !valid {
            return Err(Error::UnverifiedEnrollment {
                reason: format!("invalid genuine session ownership in directory for {machine}"),
            });
        }
    }
    Ok(())
}

/// Resolution from a fresh complete snapshot, never from legacy registrations.
#[derive(Debug)]
pub enum DirectoryRead {
    /// No unexpired snapshot; request the owning local watcher to refresh.
    Unavailable,
    /// Complete snapshot contains no verified lead at this address.
    Absent,
    /// Multiple live session associations prohibit routing.
    Ambiguous,
    /// Unique session with a proof checked again at enqueue time.
    Unique(Box<DirectoryRecipient>),
}

/// A recipient whose snapshot must still exist unchanged when enqueued.
#[derive(Debug)]
pub struct DirectoryRecipient {
    /// Verified session metadata for user-facing confirmation.
    pub registration: Registration,
    machine: MachineName,
    publication: String,
}

impl Relay {
    /// Allocate a globally ordered observation fence BEFORE querying Herdr.
    pub fn begin_directory_observation(&mut self, machine: &MachineName) -> Result<u64> {
        Ok(self
            .conn
            .incr(self.keys.directory_sequence_key(machine), 1)?)
    }

    /// Publish one complete topology observation. Ambiguous bindings are retained
    /// so resolution rejects them. Expiry uses Redis time, not host wall clocks.
    pub fn publish_directory(
        &mut self,
        machine: &MachineName,
        revision: u64,
        registrations: &[Registration],
    ) -> Result<bool> {
        validate_entries(machine, registrations)?;
        let publication = Publication {
            revision,
            registrations: registrations.to_vec(),
        };
        // Persistent sequence also fences a delayed observation after the previous
        // leased snapshot expires. Reject any observation overtaken by a newer one.
        let accepted: bool = redis::cmd("EVAL")
            .arg(
                r#"
                if redis.call('GET', KEYS[2]) ~= ARGV[1] then return 0 end
                local committed = redis.call('GET', KEYS[3])
                if committed and tonumber(committed) >= tonumber(ARGV[1]) then return 0 end
                redis.call('SET', KEYS[1], ARGV[2], 'EX', ARGV[3])
                redis.call('SET', KEYS[3], ARGV[1])
                return 1
            "#,
            )
            .arg(3)
            .arg(self.keys.directory_key(machine))
            .arg(self.keys.directory_sequence_key(machine))
            .arg(self.keys.directory_committed_key(machine))
            .arg(revision)
            .arg(encode(&publication))
            .arg(DIRECTORY_LEASE.as_secs())
            .query(&mut self.conn)?;
        Ok(accepted)
    }

    /// Read a leased machine snapshot. No SSH or Herdr process is invoked.
    pub fn directory_recipient(&mut self, address: &Address) -> Result<DirectoryRead> {
        let machine: MachineName = address.server().parse()?;
        let Some(raw): Option<String> = self.conn.get(self.keys.directory_key(&machine))? else {
            return Ok(DirectoryRead::Unavailable);
        };
        let publication: Publication =
            serde_json::from_str(&raw).map_err(|error| Error::UnverifiedEnrollment {
                reason: format!("unreadable directory for {machine}: {error}"),
            })?;
        validate_entries(&machine, &publication.registrations)?;
        let matches: Vec<_> = publication
            .registrations
            .iter()
            .filter(|registration| registration.full_address().as_ref() == Some(address))
            .collect();
        let [registration] = matches.as_slice() else {
            return Ok(if matches.is_empty() {
                DirectoryRead::Absent
            } else {
                DirectoryRead::Ambiguous
            });
        };
        if publication
            .registrations
            .iter()
            .filter(|other| other.session_id == registration.session_id)
            .count()
            != 1
        {
            return Ok(DirectoryRead::Ambiguous);
        }
        Ok(DirectoryRead::Unique(Box::new(DirectoryRecipient {
            registration: (*registration).clone(),
            machine,
            publication: raw,
        })))
    }

    /// Read verified directory entries for discovery. Expired machines return
    /// no entries; stale registration records never become discovery evidence.
    pub fn directory_entries(
        &mut self,
        machine: &MachineName,
    ) -> Result<Option<Vec<Registration>>> {
        let Some(raw): Option<String> = self.conn.get(self.keys.directory_key(machine))? else {
            return Ok(None);
        };
        let publication: Publication =
            serde_json::from_str(&raw).map_err(|error| Error::UnverifiedEnrollment {
                reason: format!("unreadable directory for {machine}: {error}"),
            })?;
        validate_entries(machine, &publication.registrations)?;
        Ok(Some(publication.registrations))
    }

    /// Whether a leased snapshot exists for this machine; payload validation
    /// still occurs on resolution or broadcast, so corruption never enables routing.
    pub fn directory_available(&mut self, machine: &MachineName) -> Result<bool> {
        Ok(self.conn.exists(self.keys.directory_key(machine))?)
    }

    /// Resolve this caller's genuine session solely from fresh relay evidence.
    /// No pane claim or mutable nickname can select a caller identity.
    pub fn directory_session(
        &mut self,
        machine: &MachineName,
        session: &crate::SessionId,
    ) -> Result<Option<Registration>> {
        let Some(raw): Option<String> = self.conn.get(self.keys.directory_key(machine))? else {
            return Ok(None);
        };
        let publication: Publication =
            serde_json::from_str(&raw).map_err(|error| Error::UnverifiedEnrollment {
                reason: format!("unreadable directory for {machine}: {error}"),
            })?;
        validate_entries(machine, &publication.registrations)?;
        let matches: Vec<_> = publication
            .registrations
            .iter()
            .filter(|registration| registration.session_id.as_ref() == Some(session))
            .collect();
        let [registration] = matches.as_slice() else {
            return Ok(None);
        };
        if registration.machine != *machine
            || registration.name != AgentName::for_session(machine, session)
            || registration.full_address().is_none()
            || publication
                .registrations
                .iter()
                .filter(|other| other.full_address() == registration.full_address())
                .count()
                != 1
        {
            return Ok(None);
        }
        Ok(Some((*registration).clone()))
    }

    /// Coalesce a cold resolution refresh request; the destination watcher reads
    /// its own local Herdr and publishes the result through this same bus.
    pub fn request_directory_refresh(&mut self, machine: &MachineName) -> Result<()> {
        let _: Option<String> = redis::cmd("SET")
            .arg(self.keys.directory_refresh_key(machine))
            .arg("1")
            .arg("EX")
            .arg(30)
            .arg("NX")
            .query(&mut self.conn)?;
        Ok(())
    }

    /// Consume a refresh request at the next eligible topology observation.
    pub fn take_directory_refresh(&mut self, machine: &MachineName) -> Result<bool> {
        let value: Option<String> = redis::cmd("GETDEL")
            .arg(self.keys.directory_refresh_key(machine))
            .query(&mut self.conn)?;
        Ok(value.is_some())
    }

    /// Broadcast only to unique owners in fresh directory snapshots, excluding
    /// the sender. All snapshot fences are checked before any queue is written.
    pub fn send_directory_broadcast(&mut self, msg: &Message) -> Result<Vec<AgentName>> {
        if msg.to != Target::All {
            return Err(Error::UnverifiedEnrollment {
                reason: "broadcast requires all target".into(),
            });
        }
        if msg.text.len() > MAX_MESSAGE_BYTES {
            return Err(Error::MessageTooLarge {
                len: msg.text.len(),
                max: MAX_MESSAGE_BYTES,
            });
        }
        if let Some((byte_offset, c)) = crate::text::first_unsafe(&msg.text) {
            return Err(Error::UnsafeMessageText {
                codepoint: u32::from(c),
                byte_offset,
            });
        }
        let machines: std::collections::BTreeSet<_> = self
            .agents()?
            .registrations
            .into_iter()
            .map(|registration| registration.machine)
            .collect();
        let mut proofs = Vec::new();
        let mut recipients = std::collections::BTreeSet::new();
        for machine in machines {
            let Some(raw): Option<String> = self.conn.get(self.keys.directory_key(&machine))?
            else {
                continue;
            };
            let publication: Publication =
                serde_json::from_str(&raw).map_err(|error| Error::UnverifiedEnrollment {
                    reason: format!("unreadable directory for {machine}: {error}"),
                })?;
            validate_entries(&machine, &publication.registrations)?;
            for registration in &publication.registrations {
                let Some(address) = registration.full_address() else {
                    continue;
                };
                let Some(session) = &registration.session_id else {
                    continue;
                };
                if registration.machine == machine
                    && registration.name == AgentName::for_session(&machine, session)
                    && !address.to_string().eq_ignore_ascii_case(msg.from.as_str())
                    && publication
                        .registrations
                        .iter()
                        .filter(|other| {
                            other.full_address().as_ref() == Some(&address)
                                || other.session_id == registration.session_id
                        })
                        .count()
                        == 1
                {
                    recipients.insert(registration.name.clone());
                }
            }
            proofs.push((machine, raw));
        }
        let recipients: Vec<_> = recipients.into_iter().collect();
        let mut command = redis::cmd("EVAL");
        command.arg(r#"
            local n = tonumber(ARGV[1])
            local count = tonumber(ARGV[2])
            for i=1,n do if redis.call('GET', KEYS[i]) ~= ARGV[i+2] then return 0 end end
            for i=n+1,n+count do
                local kind=redis.call('TYPE', KEYS[i]).ok
                if kind ~= 'none' and kind ~= 'list' then return redis.error_reply('broadcast inbox has wrong type') end
            end
            local kind=redis.call('TYPE', KEYS[n+count+1]).ok
            if kind ~= 'none' and kind ~= 'stream' then return redis.error_reply('broadcast log has wrong type') end
            for i=n+1,n+count do redis.call('RPUSH', KEYS[i], ARGV[n+3]) end
            redis.call('XADD', KEYS[n+count+1], 'MAXLEN', '~', ARGV[n+4], '*', 'msg', ARGV[n+3])
            return 1
        "#).arg(proofs.len() + recipients.len() + 1);
        for (machine, _) in &proofs {
            command.arg(self.keys.directory_key(machine));
        }
        for recipient in &recipients {
            command.arg(self.keys.inbox_key(recipient));
        }
        command
            .arg(self.keys.log_key())
            .arg(proofs.len())
            .arg(recipients.len());
        for (_, raw) in &proofs {
            command.arg(raw);
        }
        let accepted: bool = command
            .arg(encode(msg))
            .arg(LOG_CAP)
            .query(&mut self.conn)?;
        if !accepted {
            return Err(Error::DirectoryChanged);
        }
        Ok(recipients)
    }

    /// Atomically recheck the snapshot lease/fence and enqueue to its immutable
    /// session queue. Address replacement/expiry races fail before writing mail.
    pub fn send_directory(
        &mut self,
        msg: &Message,
        recipient: &DirectoryRecipient,
    ) -> Result<Vec<AgentName>> {
        if msg.to != Target::Agent(recipient.registration.name.clone()) {
            return Err(Error::UnverifiedEnrollment {
                reason: "message target differs from resolved session".into(),
            });
        }
        if msg.text.len() > MAX_MESSAGE_BYTES {
            return Err(Error::MessageTooLarge {
                len: msg.text.len(),
                max: MAX_MESSAGE_BYTES,
            });
        }
        if let Some((byte_offset, c)) = crate::text::first_unsafe(&msg.text) {
            return Err(Error::UnsafeMessageText {
                codepoint: u32::from(c),
                byte_offset,
            });
        }
        let address =
            recipient
                .registration
                .full_address()
                .ok_or_else(|| Error::UnverifiedEnrollment {
                    reason: "recipient has no canonical address".into(),
                })?;
        let mut stored = msg.clone();
        stored.to = Target::Agent(address.to_string().parse()?);
        let accepted: bool = redis::cmd("EVAL")
            .arg(r#"
                if redis.call('GET', KEYS[1]) ~= ARGV[1] then return 0 end
                local inbox = redis.call('TYPE', KEYS[2]).ok
                local log = redis.call('TYPE', KEYS[3]).ok
                if inbox ~= 'none' and inbox ~= 'list' then return redis.error_reply('directory inbox has wrong type') end
                if log ~= 'none' and log ~= 'stream' then return redis.error_reply('directory log has wrong type') end
                redis.call('RPUSH', KEYS[2], ARGV[2])
                redis.call('XADD', KEYS[3], 'MAXLEN', '~', ARGV[3], '*', 'msg', ARGV[2])
                return 1
            "#)
            .arg(3)
            .arg(self.keys.directory_key(&recipient.machine))
            .arg(self.keys.inbox_key(&recipient.registration.name))
            .arg(self.keys.log_key())
            .arg(&recipient.publication)
            .arg(encode(&stored))
            .arg(LOG_CAP)
            .query(&mut self.conn)?;
        if !accepted {
            return Err(Error::DirectoryChanged);
        }
        Ok(vec![recipient.registration.name.clone()])
    }
}
