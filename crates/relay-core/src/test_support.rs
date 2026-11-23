// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! Opt-in request instrumentation for tests against the unchanged Redis transport.
#![expect(
    clippy::expect_used,
    reason = "a poisoned test observation log is a harness invariant failure"
)]

use redis::{ConnectionLike, RedisResult, Value};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

/// Packed client requests, including failed requests. Each pipeline is exactly one entry.
/// Clones share the log. Payloads are retained only in this explicitly enabled test facility.
#[derive(Clone, Default)]
pub struct RequestLog(Arc<Mutex<Vec<Vec<u8>>>>);

impl RequestLog {
    /// Copies the recorded packed requests in call order.
    ///
    /// # Panics
    /// Panics if another test thread poisoned the observation log.
    pub fn snapshot(&self) -> Vec<Vec<u8>> {
        self.0
            .lock()
            .expect("test request log must not be poisoned")
            .clone()
    }

    /// Removes earlier observations without changing the connection or Redis state.
    ///
    /// # Panics
    /// Panics if another test thread poisoned the observation log.
    pub fn clear(&self) {
        self.0
            .lock()
            .expect("test request log must not be poisoned")
            .clear();
    }

    fn record(&self, packed: &[u8]) {
        self.0
            .lock()
            .expect("test request log must not be poisoned")
            .push(packed.to_vec());
    }
}

pub(crate) struct ObservedConnection {
    inner: redis::Connection,
    requests: Option<RequestLog>,
}

impl ObservedConnection {
    pub(crate) fn new(inner: redis::Connection) -> Self {
        Self {
            inner,
            requests: None,
        }
    }

    pub(crate) fn observe_requests(&mut self) -> RequestLog {
        let log = RequestLog::default();
        self.requests = Some(log.clone());
        log
    }

    pub(crate) fn set_read_timeout(&self, timeout: Option<Duration>) -> RedisResult<()> {
        self.inner.set_read_timeout(timeout)
    }

    pub(crate) fn set_write_timeout(&self, timeout: Option<Duration>) -> RedisResult<()> {
        self.inner.set_write_timeout(timeout)
    }

    fn record(&self, packed: &[u8]) {
        if let Some(log) = &self.requests {
            log.record(packed);
        }
    }
}

impl ConnectionLike for ObservedConnection {
    fn req_command(&mut self, command: &redis::Cmd) -> RedisResult<Value> {
        self.record(&command.get_packed_command());
        self.inner.req_command(command)
    }

    fn req_packed_command(&mut self, command: &[u8]) -> RedisResult<Value> {
        self.record(command);
        self.inner.req_packed_command(command)
    }

    fn req_packed_commands(
        &mut self,
        commands: &[u8],
        offset: usize,
        count: usize,
    ) -> RedisResult<Vec<Value>> {
        self.record(commands);
        self.inner.req_packed_commands(commands, offset, count)
    }

    fn get_db(&self) -> i64 {
        self.inner.get_db()
    }
    fn supports_pipelining(&self) -> bool {
        self.inner.supports_pipelining()
    }
    fn check_connection(&mut self) -> bool {
        self.inner.check_connection()
    }
    fn is_open(&self) -> bool {
        self.inner.is_open()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ix_trace_rs::trace;
    use redis::IntoConnectionInfo;

    fn connection() -> Option<ObservedConnection> {
        let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
            eprintln!("skipping request observer: IX_RELAY_TEST_URL unset");
            return None;
        };
        let info = url.into_connection_info().unwrap();
        let settings = info.redis_settings().clone().set_skip_set_lib_name();
        let connection = redis::Client::open(info.set_redis_settings(settings))
            .unwrap()
            .get_connection_with_timeout(Duration::from_secs(2))
            .unwrap();
        connection
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        connection
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        Some(ObservedConnection::new(connection))
    }

    #[test]
    #[trace("FR-038-AC-5", "FR-045-AC-22")]
    fn observer_counts_boundary_aligned_and_large_script_pipelines_once() {
        let Some(mut connection) = connection() else {
            return;
        };
        let log = connection.observe_requests();
        // ECHO's complete first frame ends exactly at the old reader's 8192-byte boundary.
        let echo = redis::cmd("ECHO")
            .arg("x".repeat(8169))
            .get_packed_command();
        assert_eq!(echo.len(), 8192);
        let mut pipeline = redis::pipe();
        pipeline.cmd("ECHO").arg("x".repeat(8169)).cmd("PING");
        let replies: Vec<String> = pipeline.query(&mut connection).unwrap();
        assert_eq!(replies, ["x".repeat(8169), "PONG".into()]);
        assert_eq!(log.snapshot(), [pipeline.get_packed_pipeline()]);
        log.clear();
        let script = format!(
            "local x = [=[{}]=]; return 7",
            "\r\n*1\r\n$4\r\nSCAN\r\n".repeat(4096)
        );
        let mut pipeline = redis::pipe();
        pipeline.cmd("EVAL").arg(script).arg(0).ignore().cmd("PING");
        let replies: Vec<String> = pipeline.query(&mut connection).unwrap();
        assert_eq!(replies, ["PONG"]);
        assert_eq!(log.snapshot(), [pipeline.get_packed_pipeline()]);
        log.clear();
        let mut failed = redis::cmd("EVAL");
        failed
            .arg("return redis.error_reply('fixture failure')")
            .arg(0);
        assert!(failed.query::<()>(&mut connection).is_err());
        assert_eq!(log.snapshot(), [failed.get_packed_command()]);
    }

    #[test]
    #[trace("FR-038-AC-5")]
    fn observer_preserves_atomic_offsets_errors_and_no_response_flags() {
        let Some(mut connection) = connection() else {
            return;
        };
        let log = connection.observe_requests();
        let mut transaction = redis::pipe();
        transaction
            .atomic()
            .cmd("ECHO")
            .arg("discard")
            .ignore()
            .cmd("PING");
        let replies: Vec<String> = transaction.query(&mut connection).unwrap();
        assert_eq!(replies, ["PONG"]);
        assert_eq!(log.snapshot(), [transaction.get_packed_pipeline()]);
        log.clear();
        let mut off = redis::cmd("CLIENT");
        off.arg("REPLY").arg("OFF").set_no_response(true);
        off.query::<()>(&mut connection).unwrap();
        let mut on = redis::cmd("CLIENT");
        on.arg("REPLY").arg("ON");
        assert_eq!(on.query::<String>(&mut connection).unwrap(), "OK");
        assert_eq!(
            log.snapshot(),
            [off.get_packed_command(), on.get_packed_command()]
        );
        log.clear();
        let ping = redis::cmd("PING").get_packed_command();
        assert_eq!(
            redis::from_redis_value::<String>(connection.req_packed_command(&ping).unwrap())
                .unwrap(),
            "PONG"
        );
        assert_eq!(log.snapshot(), [ping]);
    }
}
