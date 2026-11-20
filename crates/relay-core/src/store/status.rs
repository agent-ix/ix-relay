//! Redis persistence for bounded program-review status requests and replies.

use redis::Commands;

use super::keys::Keys;
use crate::{
    AgentName,
    status::{
        STATUS_REQUEST_TTL_SECS, StatusError, StatusReply, StatusRequest, StatusRequestId,
        validate_request,
    },
};

/// Redis-backed status request store. Keys are isolated under the supplied bus prefix.
pub(super) struct StatusStore<'a> {
    conn: &'a mut redis::Connection,
    keys: &'a Keys,
}

/// Outcome when a responder submits a reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyOutcome {
    /// This call stored the reply.
    Stored,
    /// An earlier reply from this agent was retained unchanged.
    AlreadySubmitted,
}

impl<'a> StatusStore<'a> {
    /// Creates a store view using the parent's centralized key builders.
    pub(super) fn new(conn: &'a mut redis::Connection, keys: &'a Keys) -> Self {
        Self { conn, keys }
    }

    /// Opens a request with a store-generated timestamp/counter id.
    pub fn create_new(
        &mut self,
        requester: AgentName,
        expected_agents: Vec<AgentName>,
    ) -> Result<StatusRequest, StatusError> {
        validate_request(&requester, &expected_agents)?;
        let created_at_ms = redis_now_ms(self.conn)?;
        let sequence: u64 = redis::cmd("INCR")
            .arg(self.keys.status_sequence_key())
            .query(self.conn)?;
        let id: StatusRequestId = format!("status-{created_at_ms}-{sequence}").parse()?;
        self.create_at(id, requester, expected_agents, created_at_ms)
    }

    fn create_at(
        &mut self,
        id: StatusRequestId,
        requester: AgentName,
        expected_agents: Vec<AgentName>,
        created_at_ms: u64,
    ) -> Result<StatusRequest, StatusError> {
        validate_request(&requester, &expected_agents)?;
        let request = StatusRequest {
            id: id.clone(),
            requester,
            expected_agents,
            created_at_ms,
        };
        let inserted: Option<String> = redis::cmd("SET")
            .arg(self.keys.status_request_key(&id))
            .arg(serde_json::to_vec(&request)?)
            .arg("NX")
            .arg("EX")
            .arg(STATUS_REQUEST_TTL_SECS)
            .query(self.conn)?;
        if inserted.is_none() {
            return Err(StatusError::RequestExists(id));
        }
        Ok(request)
    }

    /// Reads the request snapshot, or `None` after expiry / for an unknown id.
    pub fn get(&mut self, id: &StatusRequestId) -> Result<Option<StatusRequest>, StatusError> {
        let raw: Option<Vec<u8>> = self.conn.get(self.keys.status_request_key(id))?;
        raw.map(|bytes| serde_json::from_slice(&bytes).map_err(StatusError::from))
            .transpose()
    }

    /// Stores the first valid response from a requested agent. Duplicate delivery is idempotent.
    /// The submitted timestamp is assigned here, and cannot be selected by the caller.
    pub fn submit(&mut self, mut reply: StatusReply) -> Result<ReplyOutcome, StatusError> {
        reply.validate()?;
        let id = reply.request_id.clone();
        let agent = reply.agent.as_str();
        reply.submitted_at_ms = redis_now_ms(self.conn)?;
        let payload = serde_json::to_vec(&reply)?;
        let request_key = self.keys.status_request_key(&id);
        let reply_key = self.keys.status_reply_key(&id, &reply.agent);
        let script = redis::Script::new(
            r#"
            local raw = redis.call('GET', KEYS[1])
            if not raw then return -1 end
            local ttl = redis.call('PTTL', KEYS[1])
            if ttl <= 0 then return -1 end
            local req = cjson.decode(raw)
            local allowed = false
            for _, expected in ipairs(req.expected_agents) do
                if expected == ARGV[1] then allowed = true; break end
            end
            if not allowed then return -2 end
            local result = redis.call('SET', KEYS[2], ARGV[2], 'NX', 'PX', ttl)
            if result then return 1 end
            return 0
        "#,
        );
        let outcome: i64 = script
            .key(request_key)
            .key(reply_key)
            .arg(agent)
            .arg(payload)
            .invoke(self.conn)?;
        match outcome {
            1 => Ok(ReplyOutcome::Stored),
            0 => Ok(ReplyOutcome::AlreadySubmitted),
            -1 => Err(StatusError::RequestNotFound(id)),
            -2 => Err(StatusError::UnrequestedAgent(reply.agent)),
            _ => Err(StatusError::UnexpectedStoreResult(outcome)),
        }
    }

    /// Returns the immutable request and all replies received so far, in expected-agent order.
    pub fn snapshot(
        &mut self,
        id: &StatusRequestId,
    ) -> Result<Option<(StatusRequest, Vec<StatusReply>)>, StatusError> {
        let Some(request) = self.get(id)? else {
            return Ok(None);
        };
        let mut replies = Vec::new();
        for agent in &request.expected_agents {
            let raw: Option<Vec<u8>> = self.conn.get(self.keys.status_reply_key(id, agent))?;
            if let Some(raw) = raw {
                replies.push(serde_json::from_slice(&raw)?);
            }
        }
        Ok(Some((request, replies)))
    }
}

fn redis_now_ms(conn: &mut redis::Connection) -> Result<u64, StatusError> {
    let (seconds, microseconds): (u64, u64) = redis::cmd("TIME").query(conn)?;
    seconds
        .checked_mul(1_000)
        .and_then(|millis| millis.checked_add(microseconds / 1_000))
        .ok_or(StatusError::Clock)
}

#[cfg(test)]
mod tests {
    use std::{num::NonZeroUsize, time::Duration};

    use super::*;
    use crate::{LaneState, Message, Registration, Relay, StatusItem, Target};

    fn relay(test: &str) -> Option<Relay> {
        let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
            eprintln!("skipping {test}: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return None;
        };
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        Some(
            Relay::connect(&url, Duration::from_secs(5))
                .expect("connect to test redis")
                .with_prefix(format!("test:{}:{test}:{nonce}:", std::process::id())),
        )
    }

    fn registration(name: &str) -> Registration {
        Registration {
            name: name.parse().expect("valid agent name"),
            machine: "node-a".parse().expect("valid machine"),
            kind: "claude".to_owned(),
            session_id: None,
            cwd: None,
            herdr_pane: None,
            place: None,
            last_seen_ms: 1,
        }
    }

    fn item(text: &str) -> StatusItem {
        StatusItem::new(text).expect("valid status item")
    }

    #[test]
    fn notification_reaches_requested_lanes_and_partial_replies_are_idempotent() {
        let Some(mut relay) = relay("status-request-partial-redelivery") else {
            return;
        };
        let requester = registration("requester").name;
        let first = registration("lane-a").name;
        let second = registration("lane-b").name;
        for name in [&requester, &first, &second] {
            relay
                .register(&registration(name.as_str()))
                .expect("register lane");
        }
        let request = relay
            .create_status_request(requester.clone(), vec![first.clone(), second.clone()])
            .expect("create request");

        for recipient in [&first, &second] {
            let recipients = relay
                .send(&Message {
                    from: requester.clone(),
                    to: Target::Agent(recipient.clone()),
                    text: format!("Status request {}", request.id),
                    sent_at_ms: 1,
                    forwarded: false,
                })
                .expect("enqueue request notice");
            assert_eq!(recipients, vec![recipient.clone()]);
            let popped = relay
                .pop_inbox(recipient, NonZeroUsize::new(1).expect("positive batch"))
                .expect("read request notice");
            assert_eq!(popped.messages.len(), 1);
            assert!(popped.messages[0].text.contains(request.id.as_str()));
        }

        let first_reply = StatusReply {
            request_id: request.id.clone(),
            agent: first.clone(),
            tickets: vec![item("SWM-107")],
            pull_requests: vec![],
            blocker_ticket_ids: vec![],
            lane_state: LaneState::Active,
            spare_coder_slots: 1,
            owner_decisions: vec![],
            submitted_at_ms: 0,
        };
        assert_eq!(
            relay.submit_status_reply(first_reply.clone()).unwrap(),
            ReplyOutcome::Stored
        );
        let mut redelivery = first_reply;
        redelivery.tickets = vec![item("changed retry data")];
        assert_eq!(
            relay.submit_status_reply(redelivery).unwrap(),
            ReplyOutcome::AlreadySubmitted
        );

        let (saved_request, replies) = relay
            .status_snapshot(&request.id)
            .expect("snapshot")
            .expect("request still present");
        assert_eq!(
            saved_request.expected_agents,
            vec![first.clone(), second.clone()]
        );
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0].tickets, vec![item("SWM-107")]);
        let nonresponders = saved_request
            .expected_agents
            .iter()
            .filter(|agent| !replies.iter().any(|reply| &reply.agent == *agent))
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(nonresponders, vec![second]);
    }

    #[test]
    fn focus_can_be_cleared_and_server_capacity_is_explicit() {
        let Some(mut relay) = relay("lane-registry-focus-capacity") else {
            return;
        };
        let lead = registration("lane-a").name;
        relay
            .set_focus(&lead, Some("SWM-108: program review"))
            .expect("set focus");
        let focus = relay.focus_records().expect("read focus");
        assert_eq!(focus.len(), 1);
        assert_eq!(focus[0].agent, lead);
        assert_eq!(focus[0].focus, "SWM-108: program review");
        assert!(focus[0].set_at_ms > 0);
        relay.set_focus(&lead, None).expect("clear focus");
        assert!(
            relay
                .focus_records()
                .expect("read cleared focus")
                .is_empty()
        );

        let machine = "titan".parse().expect("valid machine");
        let capacity = relay
            .set_server_capacity(
                &machine,
                64,
                65_536,
                Some(16),
                vec!["Kani team only".to_owned()],
            )
            .expect("store explicit capacity");
        assert_eq!(capacity.ram_mib, 65_536);
        assert_eq!(capacity.max_agents, Some(16));
        assert_eq!(capacity.owner_rules, vec!["Kani team only"]);
        assert_eq!(relay.server_capacities().unwrap(), vec![capacity]);
        relay
            .clear_server_capacity(&machine)
            .expect("clear capacity");
        assert!(relay.server_capacities().unwrap().is_empty());
    }
}
