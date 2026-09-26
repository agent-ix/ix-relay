//! The Redis lane's shared harness for `relay-core`'s integration tests: a
//! [`Relay`] under a key prefix no other case shares, a second raw connection
//! for planting records the API would never write, and removal of the
//! prefix's keys on drop. A dev-only library, so every helper is `pub` API
//! and no test binary sees an unused one.
#![expect(
    clippy::expect_used,
    reason = "test support: a failed connection is the failure signal"
)]

use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use relay_core::Relay;

static CASE: AtomicU64 = AtomicU64::new(0);

/// A connected relay, a raw connection to the same Redis, and the prefix
/// every key of this case lives under.
pub struct Bus {
    /// The store under test.
    pub relay: Relay,
    /// A second connection, for raw writes and key inspection.
    pub raw: redis::Connection,
    /// The Redis URL, for opening more connections.
    pub url: String,
    /// The key prefix of this case.
    pub prefix: String,
}

impl Bus {
    /// A bus for `test`, or `None` (after saying so on stderr) when no test
    /// Redis is configured, so the test can return early.
    pub fn connect(test: &str) -> Option<Self> {
        let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
            eprintln!("skipping {test}: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return None;
        };
        let prefix = format!(
            "test:{}:{test}:{}:",
            std::process::id(),
            CASE.fetch_add(1, Ordering::Relaxed)
        );
        let relay = Relay::connect(&url, Duration::from_secs(2))
            .expect("connect to test redis")
            .with_prefix(prefix.clone());
        let raw = redis::Client::open(url.as_str())
            .and_then(|c| c.get_connection())
            .expect("second connection to test redis");
        Some(Self {
            relay,
            raw,
            url,
            prefix,
        })
    }

    /// `tail` under this case's prefix.
    pub fn key(&self, tail: &str) -> String {
        format!("{}{tail}", self.prefix)
    }

    /// Every key this case wrote.
    pub fn keys(&mut self) -> Vec<String> {
        redis::cmd("KEYS")
            .arg(format!("{}*", self.prefix))
            .query(&mut self.raw)
            .expect("KEYS")
    }

    /// Another store connection under the same prefix, for a concurrent client.
    pub fn another_connection(&self) -> Relay {
        Relay::connect(&self.url, Duration::from_secs(2))
            .expect("connect to test redis")
            .with_prefix(self.prefix.clone())
    }
}

impl Bus {
    /// Stores `record` (raw bytes) as the registry entry for `field`.
    pub fn plant(&mut self, field: &str, record: &[u8]) {
        redis::cmd("HSET")
            .arg(self.key("agents"))
            .arg(field)
            .arg(record)
            .exec(&mut self.raw)
            .expect("HSET raw record");
    }

    /// Sets a raw field (any bytes) of the hash `table` under the prefix.
    pub fn plant_in(&mut self, table: &str, field: &[u8], value: &[u8]) {
        redis::cmd("HSET")
            .arg(self.key(table))
            .arg(field)
            .arg(value)
            .exec(&mut self.raw)
            .expect("HSET raw");
    }

    /// Pushes a raw entry (any bytes) onto `name`'s inbox.
    pub fn rpush_raw(&mut self, name: &str, entry: &[u8]) {
        let key = self.key(&format!("inbox:{name}"));
        redis::cmd("RPUSH")
            .arg(key)
            .arg(entry)
            .exec(&mut self.raw)
            .expect("RPUSH raw entry");
    }

    /// Sets a raw field of a hash under the prefix.
    pub fn hset_raw(&mut self, table: &str, field: &str, value: &str) {
        let key = self.key(table);
        redis::cmd("HSET")
            .arg(key)
            .arg(field)
            .arg(value)
            .exec(&mut self.raw)
            .expect("HSET raw");
    }
}

impl Drop for Bus {
    fn drop(&mut self) {
        let keys = self.keys();
        if !keys.is_empty() {
            let _ = redis::cmd("DEL").arg(keys).exec(&mut self.raw);
        }
    }
}

/// `herdr agent list` as herdr 0.9.1 reports it with two claude panes: `w1:p1` is
/// a process detected as `claude` whose harness has not reported a session (no
/// `agent_session` member at all, which herdr's schema allows), `w1:p2` has one
/// (`sess-lead-a`). Shared by the check's and the watcher's regression tests.
pub const HERDR_LIST_ONE_CLAUDE_PANE_WITHOUT_A_SESSION: &str = r#"{"id":"cli:agent:list","result":{"agents":[
    {"agent":"claude","agent_status":"idle","cwd":"/w","focused":false,"foreground_cwd":"/w",
     "pane_id":"w1:p1","revision":2,"state_change_seq":10,"tab_id":"w1:t1",
     "terminal_id":"term_1","terminal_title":"claude","terminal_title_stripped":"claude",
     "workspace_id":"w1"},
    {"agent":"claude","agent_session":{"agent":"claude","kind":"id","source":"herdr:claude",
     "value":"sess-lead-a"},"agent_status":"idle","cwd":"/w","focused":false,
     "foreground_cwd":"/w","pane_id":"w1:p2","revision":3,"state_change_seq":11,
     "tab_id":"w1:t2","terminal_id":"term_2","terminal_title":"lead-a",
     "terminal_title_stripped":"lead-a","workspace_id":"w1"}],"type":"agent_list"}}"#;
