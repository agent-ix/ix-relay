//! Atomic receipt writes and side-effect-free bounded exports.

use std::{collections::BTreeMap, sync::LazyLock};

use ix_trace_rs::implements;
use redis::streams::StreamRangeReply;
use serde::Deserialize;

use super::{
    LOG_CAP, Relay, decode, encode,
    inflight::{CachedScript, script, slim_script},
};
use crate::{
    AgentName, BatchId, Error, Message, Now, Popper, Result, Target,
    receipts::{
        DeliveryEvidence, EXPIRED_ORDERS_CAP, EXPIRED_ORDERS_SCAN_CAP, ExpiredOrder,
        ExpiredOrderState, ExpiredOrders, ORDER_RETENTION_MS, OrderCursor, OrderEscalationState,
        OrderLatency, OrderListRequest, OrderPage, OrderSpec, OrderState, OrderStats, OrderWindow,
        OrderedReceipt, RECEIPT_EVENT_CAP, RECEIPT_PAGE_CAP, RECEIPT_WINDOW_MS,
        ReceiptCoverageReason, ReceiptEvent, ReceiptExport, ReceiptRequest,
    },
};

static SEND_SCRIPT: LazyLock<String> = LazyLock::new(|| {
    script(
        include_str!("lua/common.lua"),
        include_str!("lua/receipt_send.lua"),
    )
});

static OBSERVE_SCRIPT: LazyLock<String> = LazyLock::new(|| {
    script(
        include_str!("lua/common.lua"),
        include_str!("lua/receipt_observe.lua"),
    )
});

static LIST_ORDERS_SCRIPT: LazyLock<CachedScript> = LazyLock::new(|| {
    let body = script(
        include_str!("lua/common.lua"),
        include_str!("lua/order_list_all.lua"),
    );
    CachedScript::new(format!("#!lua flags=no-writes\n{body}"))
});

static ORDER_COVERAGE_SCRIPT: LazyLock<CachedScript> = LazyLock::new(|| {
    CachedScript::new(script(
        include_str!("lua/common.lua"),
        include_str!("lua/order_coverage.lua"),
    ))
});

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOrderState {
    order_id: String,
    sender: String,
    recipient: String,
    receipt_target: String,
    expect_ticket: Option<String>,
    ack_ms: i64,
    enqueued_at: i64,
    delivered_at: Option<i64>,
    acted_at: Option<i64>,
    state: String,
    expired: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOrderPage {
    orders: Vec<RawOrderState>,
    oldest_available_at: Option<i64>,
    complete: bool,
    incomplete_reason: Option<String>,
    skipped: usize,
}

/// The stored envelope is distinct from the public message/history shape.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredMessage {
    from: AgentName,
    to: Target,
    text: String,
    sent_at_ms: i64,
    #[serde(default)]
    forwarded: bool,
    #[serde(default, rename = "_relay")]
    metadata: Option<StoredMetadata>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredMetadata {
    original: String,
    recipient: String,
    #[serde(default)]
    attempt: Option<String>,
    #[serde(default)]
    delivered: Option<DeliveryEvidence>,
    #[serde(default)]
    order: Option<i64>,
    #[serde(default)]
    expect: Option<String>,
}

pub(crate) fn decode_message(bytes: &[u8]) -> std::result::Result<Message, serde_json::Error> {
    let stored: StoredMessage = serde_json::from_slice(bytes)?;
    // Metadata belongs to persistence, never to the recipient's public payload.
    if let Some(meta) = stored.metadata {
        let _ = (
            meta.original,
            meta.recipient,
            meta.attempt,
            meta.delivered,
            meta.order,
            meta.expect,
        );
    }
    Ok(Message {
        from: stored.from,
        to: stored.to,
        text: stored.text,
        sent_at_ms: stored.sent_at_ms,
        forwarded: stored.forwarded,
    })
}

impl Relay {
    #[implements("FR-071-AC-12")]
    pub(super) fn enqueue_receipted(
        &mut self,
        message: &Message,
        recipients: &[AgentName],
        publications: &[(String, String)],
        order: Option<&OrderSpec>,
    ) -> Result<bool> {
        let mut command = redis::cmd("EVAL");
        command
            .arg(SEND_SCRIPT.as_str())
            .arg(publications.len() + recipients.len() + 1);
        for (key, _) in publications {
            command.arg(key);
        }
        for recipient in recipients {
            command.arg(self.keys.inbox_key(recipient));
        }
        command
            .arg(self.keys.log_key())
            .arg(publications.len())
            .arg(recipients.len());
        for (_, publication) in publications {
            command.arg(publication);
        }
        command
            .arg(encode(message))
            .arg(LOG_CAP)
            .arg("")
            .arg(order.map_or_else(String::new, |o| o.ack_deadline_ms().to_string()))
            .arg(order.and_then(OrderSpec::expect_ticket).unwrap_or(""));
        for recipient in recipients {
            command.arg(recipient.as_str());
        }
        Ok(command
            .arg(ORDER_RETENTION_MS)
            .arg(self.keys.receipt_stream_key())
            .arg(self.keys.receipt_meta_key())
            .query(&mut self.conn)?)
    }

    /// Persists a transport's successful observation for an existing attempt.
    /// Repeating the observation or naming an expired attempt creates no duplicate.
    /// A failure leaves the pending-observation marker visible to receipt readers.
    pub fn observe_delivery(
        &mut self,
        agent: &AgentName,
        batch: BatchId,
        popper: Popper,
        evidence: DeliveryEvidence,
        now: Now,
    ) -> Result<usize> {
        let count: i64 = redis::cmd("EVAL")
            .arg(OBSERVE_SCRIPT.as_str())
            .arg(1)
            .arg(self.keys.inflight_key(agent))
            .arg(batch.get())
            .arg(popper.letter().to_string())
            .arg(evidence.code())
            .arg(now.arg())
            .arg(self.keys.receipt_stream_key())
            .arg(self.keys.receipt_meta_key())
            .query(&mut self.conn)?;
        usize::try_from(count).map_err(|_| Error::InflightUnreadable {
            lead: agent.clone(),
        })
    }

    /// Reads receipt history without changing any inbox, lease or receipt state.
    /// Unreadable source evidence is returned as explicitly unavailable coverage.
    /// The request's interval is measured by observation time, including when the
    /// source event time is unknown or moves backwards.
    pub fn receipt_export(&mut self, request: &ReceiptRequest) -> Result<ReceiptExport> {
        validate_request(request)?;
        let mut export = empty_export(request);
        let result = self.read_receipts();
        let Ok((metadata, records)) = result else {
            export
                .reasons
                .push(ReceiptCoverageReason::SourceUnavailable);
            return Ok(export);
        };
        populate_export(request, &metadata, records, &mut export)?;
        Ok(export)
    }

    fn read_receipts(&mut self) -> Result<(BTreeMap<String, String>, Vec<OrderedReceipt>)> {
        let mut pipeline = redis::pipe();
        pipeline
            .atomic()
            .cmd("HGETALL")
            .arg(self.keys.receipt_meta_key())
            .cmd("XRANGE")
            .arg(self.keys.receipt_stream_key())
            .arg("-")
            .arg("+")
            .arg("COUNT")
            .arg(RECEIPT_EVENT_CAP + 1);
        let (metadata, stream): (BTreeMap<String, String>, StreamRangeReply) =
            pipeline.query(&mut self.conn)?;
        if metadata.get("last_cursor") != stream.ids.last().map(|row| &row.id) {
            return Err(Error::InvalidName {
                name: "receipt stream does not match its persisted observation boundary".into(),
            });
        }
        if stream.ids.len() > RECEIPT_EVENT_CAP {
            return Err(Error::InvalidName {
                name: "receipt event bound exceeded".into(),
            });
        }
        let key = self.keys.receipt_stream_key();
        let mut events = Vec::with_capacity(stream.ids.len());
        for row in stream.ids {
            let json = row
                .get::<String>("event")
                .ok_or_else(|| Error::InvalidName {
                    name: "receipt event is missing".into(),
                })?;
            let event: ReceiptEvent = decode(&key, &json)?;
            events.push(OrderedReceipt {
                cursor: row.id,
                event,
            });
        }
        Ok((metadata, events))
    }
}

pub(crate) fn validate_request(request: &ReceiptRequest) -> Result<()> {
    if request.since_ms > request.until_ms
        || request.until_ms > request.now_ms
        || !(1..=RECEIPT_PAGE_CAP).contains(&request.page_size)
        || request
            .after
            .as_ref()
            .is_some_and(|cursor| cursor_parts(cursor).is_none())
    {
        return Err(Error::InvalidName {
            name: "invalid receipt interval, page size or cursor".into(),
        });
    }
    Ok(())
}

fn cursor_parts(cursor: &str) -> Option<(u64, u64)> {
    let (time, sequence) = cursor.split_once('-')?;
    Some((time.parse().ok()?, sequence.parse().ok()?))
}

pub(crate) fn empty_export(request: &ReceiptRequest) -> ReceiptExport {
    ReceiptExport {
        requested_since_ms: request.since_ms,
        requested_until_ms: request.until_ms,
        covered_since_ms: None,
        covered_until_ms: None,
        instrumentation_start_ms: None,
        retention_boundary_ms: request.now_ms.saturating_sub(RECEIPT_WINDOW_MS),
        complete: false,
        reasons: Vec::new(),
        events: Vec::new(),
        next_cursor: None,
        has_more: false,
    }
}

fn populate_export(
    request: &ReceiptRequest,
    metadata: &BTreeMap<String, String>,
    records: Vec<OrderedReceipt>,
    export: &mut ReceiptExport,
) -> Result<()> {
    let number = |key: &str| metadata.get(key).and_then(|v| v.parse::<i64>().ok());
    let Some(start) = number("instrumentation_start_ms") else {
        export
            .reasons
            .push(ReceiptCoverageReason::BeforeInstrumentation);
        return Ok(());
    };
    export.instrumentation_start_ms = Some(start);
    if request.since_ms < start {
        export
            .reasons
            .push(ReceiptCoverageReason::BeforeInstrumentation);
    }
    if let Some(evicted) = number("evicted_through_ms") {
        export.retention_boundary_ms = export.retention_boundary_ms.max(evicted.saturating_add(1));
    }
    if request.since_ms < export.retention_boundary_ms {
        export.reasons.push(ReceiptCoverageReason::Retention);
    }
    if let (Some(first), Some(last)) = (number("gap_start"), number("gap_end"))
        && first <= request.until_ms
        && last >= request.since_ms
    {
        export.reasons.push(ReceiptCoverageReason::RecordingGap);
    }
    if metadata.iter().any(|(key, value)| {
        key.starts_with("pending:")
            && value
                .parse::<i64>()
                .is_ok_and(|start| start <= request.until_ms)
    }) {
        export
            .reasons
            .push(ReceiptCoverageReason::ObservationPending);
    }
    let after = request.after.as_deref().and_then(cursor_parts);
    let first_cursor = records.first().and_then(|row| cursor_parts(&row.cursor));
    if after
        .zip(first_cursor)
        .is_some_and(|(after, first)| after < first)
    {
        export.reasons.push(ReceiptCoverageReason::CursorEvicted);
    } else if request
        .after
        .as_ref()
        .is_some_and(|cursor| !records.iter().any(|record| &record.cursor == cursor))
    {
        export
            .reasons
            .push(ReceiptCoverageReason::CursorUnavailable);
    }
    let covered_start = request
        .since_ms
        .max(start)
        .max(export.retention_boundary_ms);
    if covered_start <= request.until_ms {
        export.covered_since_ms = Some(covered_start);
        export.covered_until_ms = Some(request.until_ms);
    }
    for record in records {
        let Some(position) = cursor_parts(&record.cursor) else {
            return Err(Error::InvalidName {
                name: "invalid stored receipt cursor".into(),
            });
        };
        let time = record.event.observed_at_ms;
        if time < request.since_ms
            || time > request.until_ms
            || time < request.now_ms.saturating_sub(RECEIPT_WINDOW_MS)
            || after.is_some_and(|after| position <= after)
        {
            continue;
        }
        if export.events.len() == request.page_size {
            export.has_more = true;
            export.next_cursor = export.events.last().map(|row| row.cursor.clone());
            break;
        }
        export.events.push(record);
    }
    export.complete = export.reasons.is_empty();
    Ok(())
}

static EXPIRE_SCRIPT: LazyLock<CachedScript> =
    LazyLock::new(|| CachedScript::new(slim_script(include_str!("lua/order_expire.lua"))));

static ESCALATE_SCRIPT: LazyLock<CachedScript> =
    LazyLock::new(|| CachedScript::new(slim_script(include_str!("lua/order_escalate.lua"))));

static ESCALATE_FAILED_SCRIPT: LazyLock<CachedScript> =
    LazyLock::new(|| CachedScript::new(slim_script(include_str!("lua/order_escalate_failed.lua"))));

static LIST_EXPIRED_SCRIPT: LazyLock<CachedScript> = LazyLock::new(|| {
    // `no-writes` makes Redis refuse any write the body might one day contain.
    let body = slim_script(include_str!("lua/order_expired_list.lua"));
    CachedScript::new(format!("#!lua flags=no-writes\n{body}"))
});

/// Most orders one scan expires.
const ORDER_SCAN_LIMIT: usize = 100;

impl Relay {
    /// Records `AckExpired` for every order whose ack deadline has passed and
    /// returns the orders whose escalation is not filed yet (at most 20; an order
    /// whose escalation kept failing is not among them), in one request and without a
    /// key scan. Safe to repeat: an order expires once.
    ///
    /// # Errors
    /// Fails when Redis cannot be reached or a receipt key has the wrong type.
    pub fn expire_orders(&mut self, now: Now) -> Result<Vec<ExpiredOrder>> {
        let (stream, meta) = (self.keys.receipt_stream_key(), self.keys.receipt_meta_key());
        let rows: Vec<Vec<String>> = EXPIRE_SCRIPT.run(&mut self.conn, |cmd| {
            cmd.arg(0)
                .arg(ORDER_SCAN_LIMIT)
                .arg(now.arg())
                .arg(&stream)
                .arg(&meta);
        })?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let [original_id, sender, recipient, queue, last_state, expect] =
                    <[String; 6]>::try_from(row).ok()?;
                Some(ExpiredOrder {
                    original_id,
                    sender,
                    recipient,
                    queue,
                    last_state,
                    expect_ticket: (!expect.is_empty()).then_some(expect),
                })
            })
            .collect())
    }

    /// Every order whose `AckExpired` the relay recorded and that was not `Acted`,
    /// newest expiry first, in one request and without a key scan. At most
    /// [`EXPIRED_ORDERS_CAP`] orders come back; `truncated` says more exist. Changes
    /// nothing. Orders that expired before the relay indexed them, and orders whose
    /// record aged out (eight days), are not listed.
    ///
    /// # Errors
    /// Fails when Redis cannot be reached or a receipt key has the wrong type.
    pub fn expired_orders(&mut self) -> Result<ExpiredOrders> {
        let lanes_key = self.keys.fleet_lanes_key();
        let (stream, meta) = (self.keys.receipt_stream_key(), self.keys.receipt_meta_key());
        let rows: Vec<String> = LIST_EXPIRED_SCRIPT.run(&mut self.conn, |cmd| {
            cmd.arg(0)
                .arg(EXPIRED_ORDERS_CAP)
                .arg(EXPIRED_ORDERS_SCAN_CAP)
                .arg(&lanes_key)
                .arg(&stream)
                .arg(&meta);
        })?;
        let mut rows = rows.into_iter();
        let truncated = rows.next().is_some_and(|flag| flag == "1");
        let indexed_since_ms = rows.next().and_then(|since| since.parse().ok());
        rows.next();
        let list_count = rows.next().and_then(|n| n.parse().ok()).unwrap_or(0);
        let lists: Vec<crate::fleet::HostLaneList> = rows
            .by_ref()
            .take(list_count)
            .filter_map(|raw| serde_json::from_str(&raw).ok())
            .collect();
        let fields: Vec<String> = rows.collect();
        let (chunks, rest) = fields.as_chunks::<9>();
        let mut skipped = u64::try_from(rest.len()).unwrap_or(u64::MAX);
        let mut orders = Vec::with_capacity(chunks.len());
        for row in chunks {
            match expired_order_of(row) {
                Some(order) => orders.push(order),
                None => skipped += 1,
            }
        }
        {
            let lanes: Vec<(crate::Address, &str)> = lists
                .iter()
                .flat_map(|list| list.lanes.iter())
                .filter_map(|lane| Some((lane.parse().ok()?, lane.as_str())))
                .collect();
            for order in &mut orders {
                if let Ok(stored) = order.recipient.parse::<crate::Address>()
                    && let Some((_, spelling)) =
                        lanes.iter().find(|(address, _)| *address == stored)
                {
                    order.recipient = (*spelling).to_owned();
                }
            }
        }
        Ok(ExpiredOrders {
            orders,
            truncated,
            skipped,
            indexed_since_ms,
        })
    }

    /// Confirms all-outcome coverage from Redis server time. This is an operator
    /// assertion that every order-sending host uses an index-writing `agent-msg`.
    #[implements("FR-071-AC-12")]
    pub fn confirm_orders_complete_since(&mut self) -> Result<i64> {
        let reply: String = ORDER_COVERAGE_SCRIPT.run(&mut self.conn, |cmd| {
            cmd.arg(0)
                .arg("confirm")
                .arg(self.keys.receipt_stream_key())
                .arg(self.keys.receipt_meta_key());
        })?;
        reply.parse().map_err(|_| Error::UnexpectedScriptReply {
            script: "order_coverage",
            reply: "invalid confirmation timestamp".into(),
        })
    }

    /// Clears the all-outcome coverage assertion, for example before a sender rollback.
    #[implements("FR-071-AC-12")]
    pub fn clear_orders_complete_since(&mut self) -> Result<bool> {
        let reply: String = ORDER_COVERAGE_SCRIPT.run(&mut self.conn, |cmd| {
            cmd.arg(0)
                .arg("clear")
                .arg(self.keys.receipt_stream_key())
                .arg(self.keys.receipt_meta_key());
        })?;
        match reply.as_str() {
            "0" => Ok(false),
            "1" => Ok(true),
            _ => Err(Error::UnexpectedScriptReply {
                script: "order_coverage",
                reply: "invalid clear result".into(),
            }),
        }
    }

    /// Reads the confirmed complete-coverage boundary, if the operator has set one.
    #[implements("FR-071-AC-12")]
    pub fn orders_complete_since(&mut self) -> Result<Option<i64>> {
        let reply: String = ORDER_COVERAGE_SCRIPT.run(&mut self.conn, |cmd| {
            cmd.arg(0)
                .arg("show")
                .arg(self.keys.receipt_stream_key())
                .arg(self.keys.receipt_meta_key());
        })?;
        if reply.is_empty() {
            return Ok(None);
        }
        reply
            .parse()
            .map(Some)
            .map_err(|_| Error::UnexpectedScriptReply {
                script: "order_coverage",
                reply: "invalid confirmation timestamp".into(),
            })
    }

    /// Reads one bounded page of every retained order status, newest enqueue first.
    /// This is a read-only single Redis script; message content is never read.
    ///
    /// # Errors
    /// Fails when Redis is unavailable or a required history key has the wrong type.
    /// Unreadable indexed rows are counted in `skipped` and make coverage incomplete.
    #[implements("FR-071-AC-12")]
    pub fn orders(&mut self, request: &OrderListRequest) -> Result<OrderPage> {
        let cursor = request.cursor.as_ref();
        let reply: String = LIST_ORDERS_SCRIPT.run(&mut self.conn, |cmd| {
            cmd.arg(0)
                .arg(request.since_ms)
                .arg(request.until_ms)
                .arg(request.limit)
                .arg(cursor.map_or_else(String::new, |c| c.enqueued_at_ms.to_string()))
                .arg(cursor.map_or("", |c| c.order_id.as_str()))
                .arg(ORDER_RETENTION_MS)
                .arg(self.keys.receipt_stream_key())
                .arg(self.keys.receipt_meta_key());
        })?;
        let raw: RawOrderPage =
            serde_json::from_str(&reply).map_err(|_| Error::UnexpectedScriptReply {
                script: "orders",
                reply: "invalid order history response".into(),
            })?;
        let truncated = raw.orders.len() > request.limit;
        let orders: Vec<OrderState> = raw
            .orders
            .into_iter()
            .take(request.limit)
            .map(|order| {
                let deadline_at = order
                    .delivered_at
                    .unwrap_or(order.enqueued_at)
                    .saturating_add(order.ack_ms);
                OrderState {
                    order_id: order.order_id,
                    sender: order.sender,
                    recipient: order.recipient,
                    receipt_target: order.receipt_target,
                    expect_ticket: order.expect_ticket,
                    ack_ms: order.ack_ms,
                    deadline_at,
                    enqueued_at_ms: order.enqueued_at,
                    delivered_at_ms: order.delivered_at,
                    acted_at_ms: order.acted_at,
                    state: order.state,
                    expired: order.expired,
                }
            })
            .collect();
        let next_cursor = if truncated {
            orders.last().map(|order| OrderCursor {
                enqueued_at_ms: order.enqueued_at_ms,
                order_id: order.order_id.clone(),
            })
        } else {
            None
        };
        if truncated && next_cursor.is_none() {
            return Err(Error::UnexpectedScriptReply {
                script: "orders",
                reply: "truncated order history page has no continuation position".into(),
            });
        }
        Ok(OrderPage {
            orders,
            truncated,
            next_cursor,
            window: OrderWindow {
                since: request.since_ms,
                until: request.until_ms,
                filter: "enqueued_at",
                order: "newest_first",
            },
            limit: request.limit,
            oldest_available_at: raw.oldest_available_at,
            complete: raw.complete,
            incomplete_reason: raw.incomplete_reason,
            skipped: raw.skipped,
        })
    }

    /// Files the escalation of an expired order, once however often it is called.
    /// Returns whether this call filed it. The escalation has kind `order` and no
    /// ticket; it names the order identity, the recipient lane address, the expected
    /// ticket when the order carried one and the last receipt state, and no message
    /// text. It is checked like any other filing.
    ///
    /// # Errors
    /// Fails when the filing is invalid or Redis cannot be reached.
    pub fn escalate_expired_order(
        &mut self,
        order: &ExpiredOrder,
        created_at_ms: i64,
    ) -> Result<bool> {
        let (machine, session) = decode_queue(&order.queue);
        let mut context = format!(
            "Order {} from {} to {} reached its ack deadline without an Acted receipt \
             (last receipt: {}).",
            order.original_id, order.sender, order.recipient, order.last_state
        );
        if let Some(ticket) = &order.expect_ticket {
            context.push_str(&format!(" It expects ticket {ticket}."));
        }
        let filing = crate::EscalationFiling {
            ticket: None,
            lead: order.recipient.parse().or_else(|_| order.queue.parse())?,
            machine: machine.parse()?,
            session_id: session.parse()?,
            cwd: None,
            kind: "order".into(),
            context,
            options: vec!["re-send the order".into(), "check the lane".into()],
            recommendation: "check the lane".into(),
            created_at_ms,
        };
        filing.validate()?;
        let meta = serde_json::to_string(&filing).map_err(|_| Error::InvalidName {
            name: "order escalation".into(),
        })?;
        let (seq, index, stem) = (
            self.keys.esc_seq_key(),
            self.keys.esc_index_key(),
            self.keys.esc_stem(),
        );
        let (stream, receipts_meta) =
            (self.keys.receipt_stream_key(), self.keys.receipt_meta_key());
        let filed: i64 = ESCALATE_SCRIPT.run(&mut self.conn, |cmd| {
            cmd.arg(2)
                .arg(&seq)
                .arg(&index)
                .arg(&order.original_id)
                .arg(&stem)
                .arg(&meta)
                .arg(crate::EscalationStatus::Waiting.as_str())
                .arg(created_at_ms)
                .arg(&stream)
                .arg(&receipts_meta);
        })?;
        Ok(filed == 1)
    }

    /// Counts one failed attempt to file `order`'s escalation. After `attempts_allowed`
    /// failures the order moves to the failed set and no longer comes back from
    /// [`Relay::expire_orders`]. Returns the failures so far.
    ///
    /// # Errors
    /// Fails when Redis cannot be reached.
    pub fn order_escalation_failed(
        &mut self,
        order: &ExpiredOrder,
        attempts_allowed: u32,
    ) -> Result<u32> {
        let (stream, meta) = (self.keys.receipt_stream_key(), self.keys.receipt_meta_key());
        let failures: i64 = ESCALATE_FAILED_SCRIPT.run(&mut self.conn, |cmd| {
            cmd.arg(0)
                .arg(&order.original_id)
                .arg(attempts_allowed)
                .arg(&stream)
                .arg(&meta);
        })?;
        Ok(u32::try_from(failures).unwrap_or(u32::MAX))
    }

    /// The newest `max` hook health entries (newest first, as
    /// [`Relay::hook_health_newest`]) and the order metrics of `machine`, in one
    /// pipelined request: the unacted orders whose recipient lane is on `machine`, the
    /// delivered-to-acted latency of orders delivered there, and how many orders
    /// have a failed escalation.
    ///
    /// # Errors
    /// Fails when Redis cannot be reached.
    pub fn hook_health_and_order_stats(
        &mut self,
        max: usize,
        machine: &crate::MachineName,
    ) -> Result<(super::HookHealth, OrderStats)> {
        let meta = self.keys.receipt_meta_key();
        let machine_hex: String = machine
            .as_str()
            .bytes()
            .map(|b| format!("{b:02x}"))
            .collect();
        let mut pipeline = redis::pipe();
        pipeline
            .cmd("XREVRANGE")
            .arg(self.keys.health_hooks_key())
            .arg("+")
            .arg("-")
            .arg("COUNT")
            .arg(max)
            .cmd("HGETALL")
            .arg(format!("{meta}:orders-unacted"))
            .cmd("HGETALL")
            .arg(format!("{meta}:order-latency:{machine_hex}"))
            .cmd("ZCARD")
            .arg(format!(
                "{meta}:orders-escalate-failed:{}",
                machine.as_str()
            ));
        let (health, unacted, latency, failed): (
            StreamRangeReply,
            BTreeMap<String, u64>,
            BTreeMap<String, u64>,
            u64,
        ) = pipeline.query(&mut self.conn)?;
        let mut stats = OrderStats {
            unacted: unacted
                .into_iter()
                .filter(|(lane, _)| {
                    lane.split(':')
                        .next()
                        .is_some_and(|server| server == machine.as_str())
                })
                .collect(),
            latency: OrderLatency::default(),
            escalation_failed: failed,
        };
        for (slot, bucket) in stats.latency.buckets.iter_mut().enumerate() {
            *bucket = latency.get(&format!("b{slot}")).copied().unwrap_or(0);
        }
        stats.latency.count = latency.get("count").copied().unwrap_or(0);
        stats.latency.sum_ms = latency.get("sum_ms").copied().unwrap_or(0);
        Ok((super::hook_health_of(&health), stats))
    }
}

fn expired_order_of(row: &[String; 9]) -> Option<ExpiredOrderState> {
    let [
        id,
        expired_at,
        to,
        expect,
        enqueued,
        delivered,
        ack_ms,
        last_state,
        escalation,
    ] = row;
    let enqueued_at_ms: i64 = enqueued.parse().ok()?;
    let from = if delivered.is_empty() {
        enqueued_at_ms
    } else {
        delivered.parse().ok()?
    };
    Some(ExpiredOrderState {
        original_id: id.clone(),
        recipient: to.clone(),
        expect_ticket: (!expect.is_empty()).then(|| expect.clone()),
        enqueued_at_ms,
        deadline_ms: from.saturating_add(ack_ms.parse().ok()?),
        expired_at_ms: expired_at.parse().ok()?,
        delivered_before_expiry: !delivered.is_empty(),
        last_state: last_state.clone(),
        escalation: match escalation.as_str() {
            "" => OrderEscalationState::Pending,
            "failed" => OrderEscalationState::Failed,
            filed
                if filed
                    .strip_prefix('e')
                    .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) =>
            {
                OrderEscalationState::Filed
            }
            _ => OrderEscalationState::Unknown,
        },
    })
}

/// The machine and session id a `session/<hex>/<hex>` queue name encodes; the name
/// itself, twice, for any other queue.
fn decode_queue(queue: &str) -> (String, String) {
    let unhex = |hex: &str| {
        let bytes: Option<Vec<u8>> = hex
            .as_bytes()
            .chunks(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
            .collect();
        String::from_utf8(bytes?).ok()
    };
    queue
        .strip_prefix("session/")
        .and_then(|rest| rest.split_once('/'))
        .and_then(|(machine, session)| Some((unhex(machine)?, unhex(session)?)))
        .unwrap_or_else(|| (queue.to_owned(), queue.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The activity write is sent on every prompt and tool call of every lane, and the
    /// scan every tick of every watcher: both stay a fraction of the full receipt script.
    #[test]
    fn the_hot_scripts_carry_only_the_receipt_core() {
        let full = script(
            include_str!("lua/common.lua"),
            include_str!("lua/receipt_send.lua"),
        )
        .len();
        let activity = super::super::activity::WRITE.body().len();
        let scan = EXPIRE_SCRIPT.body().len();
        eprintln!(
            "script bytes: activity {activity}, scan {scan}, escalate {}, failed {}, full receipt {full}",
            ESCALATE_SCRIPT.body().len(),
            ESCALATE_FAILED_SCRIPT.body().len()
        );
        assert!(activity * 2 < full && scan * 2 < full);
        assert!(activity < 6_000, "{activity}");
    }
}
