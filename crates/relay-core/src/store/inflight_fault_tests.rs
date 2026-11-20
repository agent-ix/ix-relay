//! Fault injection for the in-flight scripts: what a script that fails after its
//! first write leaves behind (FR-020-AC-10). Unit tests because they run a
//! script body by hand, with an argument no caller passes. Redis-backed
//! (`IX_RELAY_TEST_URL`); skipped with a message without it.
use std::{num::NonZeroUsize, time::Duration};

use super::inflight::REQUEUE_SCRIPT;
use crate::{AgentName, Claim, Message, Popper, Relay, Target};

fn name(s: &str) -> AgentName {
    s.parse().unwrap()
}

fn message(text: &str) -> Message {
    Message {
        from: name("a"),
        to: Target::Agent(name("r")),
        text: text.into(),
        sent_at_ms: 1,
        forwarded: false,
    }
}

/// Tracing: TC-020-10
/// ACs: FR-020-AC-10
///
/// `requeue_batch` writes the inbox and the log before it deletes the in-flight
/// rows. Run with a log cap `XADD` refuses, it fails after the `LPUSH`: the batch
/// must then be in flight still (a duplicate is possible, a loss is not). A script
/// that deleted the rows first, or wrote the log before the inbox, loses both
/// messages here.
#[test]
fn a_requeue_that_fails_after_its_first_write_never_leaves_a_message_nowhere() {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
        return;
    };
    let mut r = Relay::connect(&url, Duration::from_secs(5))
        .unwrap()
        .with_prefix(format!("test:{}:inflight-fault:", std::process::id()));
    let lead = name("r");
    r.requeue_front(&lead, &[message("one"), message("two")])
        .unwrap();
    let batch = r
        .pop_inbox_budget(
            &lead,
            NonZeroUsize::new(5).unwrap(),
            usize::MAX,
            Claim::new(Popper::Hook),
        )
        .unwrap()
        .batch
        .unwrap();
    assert_eq!(r.inbox_len(&lead).unwrap(), 0);
    assert_eq!(r.inflight_len(&lead).unwrap(), 2);

    let failed: redis::RedisResult<i64> = redis::cmd("EVAL")
        .arg(REQUEUE_SCRIPT.as_str())
        .arg(3)
        .arg(r.keys.inflight_key(&lead))
        .arg(r.keys.inbox_key(&lead))
        .arg(r.keys.log_key())
        .arg(batch.get())
        .arg("r")
        .arg("not-a-number")
        .arg("h")
        .query(&mut r.conn);
    assert!(failed.is_err(), "XADD refuses a cap that is not a number");

    let somewhere = r.inbox_len(&lead).unwrap() + r.inflight_len(&lead).unwrap();
    assert!(
        somewhere >= 2,
        "both messages must still exist, found {somewhere}"
    );
    // Clean up: the keys are ours alone.
    for key in [
        r.keys.inflight_key(&lead),
        r.keys.inbox_key(&lead),
        r.keys.log_key(),
    ] {
        redis::cmd("DEL").arg(key).exec(&mut r.conn).unwrap();
    }
    redis::cmd("DEL")
        .arg(r.keys.inflight_seq_key())
        .exec(&mut r.conn)
        .unwrap();
}
