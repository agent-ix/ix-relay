//! The one bounded key scan: every "which leads have a list under this kind of
//! key" question (`inbox --in-flight`, `agent-msg doctor`) goes through it.
//!
//! `SCAN` cannot be asked for a prefix, only for a glob, so a configured key
//! prefix holding `*`, `?`, `[`, `]` or `\` would match other keys unless it is
//! escaped: [`Relay::scan_prefixed`] escapes it. And a `SCAN` over a huge
//! keyspace is many calls, so both the keys returned and the calls made are
//! bounded, and a cut is reported (`truncated`) rather than silent.

use super::{Relay, Result};

/// The most `SCAN` calls one scan makes.
pub(super) const SCAN_MAX_CALLS: usize = 64;

/// How many keys each `SCAN` call is asked to look at.
pub(super) const SCAN_COUNT: usize = 1000;

/// How far one scan may go.
#[derive(Debug, Clone, Copy)]
pub(super) struct ScanLimits {
    /// The most keys returned.
    pub(super) max_keys: usize,
    /// The most `SCAN` calls made.
    pub(super) max_calls: usize,
    /// The `COUNT` given to each call.
    pub(super) count: usize,
}

/// Which bound stopped a scan short: its keys, its calls, or its deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanCut {
    /// It found more keys than its key bound.
    Keys,
    /// It made all the `SCAN` calls it may, and the cursor was not back at zero.
    Calls,
    /// Its caller's deadline passed ([`Relay::set_scan_deadline`]).
    Deadline,
}

/// What a scan found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Scanned {
    /// The key names, sorted and free of duplicates (`SCAN` may repeat one), at
    /// most [`ScanLimits::max_keys`] of them. Names that are not UTF-8 are left out.
    pub(super) keys: Vec<String>,
    /// The bound that stopped the scan short, if one did: it may have missed keys.
    pub(super) cut: Option<ScanCut>,
}

/// `text` with the characters a `SCAN MATCH` pattern treats specially escaped, so
/// a configured key prefix is matched literally.
pub(super) fn glob_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '*' | '?' | '[' | ']' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

impl Relay {
    /// Makes every key scan from now on stop at `deadline` (`None`: no deadline):
    /// each `SCAN` call is armed with what is left, and a scan that runs out
    /// returns what it found, cut by [`ScanCut::Deadline`], instead of waiting out
    /// its remaining calls. For a caller with one fixed budget for a whole report
    /// (`agent-msg doctor`), where sixty-four calls that each take a second are
    /// not an acceptable answer.
    pub fn set_scan_deadline(&self, deadline: Option<std::time::Instant>) {
        self.scan_deadline.set(deadline);
    }

    /// Every key that starts with the literal `head` (a key prefix plus a kind,
    /// e.g. `ixr:inflight:`), within the default bounds: `max_keys` keys and
    /// [`SCAN_MAX_CALLS`] calls.
    pub(super) fn scan_prefixed(&mut self, head: &str, max_keys: usize) -> Result<Scanned> {
        self.scan_limited(
            head,
            ScanLimits {
                max_keys,
                max_calls: SCAN_MAX_CALLS,
                count: SCAN_COUNT,
            },
        )
    }

    /// [`Relay::scan_prefixed`] with explicit limits.
    pub(super) fn scan_limited(&mut self, head: &str, limits: ScanLimits) -> Result<Scanned> {
        let pattern = format!("{}*", glob_escape(head));
        let mut keys: Vec<String> = Vec::new();
        let mut cursor: u64 = 0;
        let mut cut = None;
        for call in 1..=limits.max_calls {
            if let Some(deadline) = self.scan_deadline.get() {
                let left = deadline.saturating_duration_since(std::time::Instant::now());
                if left.is_zero() {
                    cut = Some(ScanCut::Deadline);
                    break;
                }
                self.set_timeout(left.min(self.timeout.get()))?;
            }
            let reply: redis::RedisResult<(u64, Vec<Vec<u8>>)> = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(&pattern)
                .arg("COUNT")
                .arg(limits.count)
                .query(&mut self.conn);
            let (next, batch) = match reply {
                Ok(found) => found,
                // The deadline is what stopped this call: keep what was found.
                Err(_)
                    if self
                        .scan_deadline
                        .get()
                        .is_some_and(|d| d <= std::time::Instant::now()) =>
                {
                    cut = Some(ScanCut::Deadline);
                    break;
                }
                Err(e) => return Err(e.into()),
            };
            keys.extend(batch.into_iter().filter_map(|k| String::from_utf8(k).ok()));
            cursor = next;
            if cursor == 0 {
                break;
            }
            if call == limits.max_calls {
                cut = Some(ScanCut::Calls);
            }
        }
        keys.sort();
        keys.dedup();
        if keys.len() > limits.max_keys {
            keys.truncate(limits.max_keys);
            cut = Some(ScanCut::Keys);
        }
        Ok(Scanned { keys, cut })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn relay(test: &str, prefix_tail: &str) -> Option<Relay> {
        let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
            eprintln!("skipping {test}: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return None;
        };
        Some(
            Relay::connect(&url, Duration::from_secs(5))
                .unwrap()
                .with_prefix(format!("test:{}:{test}:{prefix_tail}", std::process::id())),
        )
    }

    fn plant(r: &mut Relay, keys: impl IntoIterator<Item = String>) {
        let mut pipe = redis::pipe();
        for key in keys {
            pipe.cmd("SET").arg(key).arg("x").ignore();
        }
        let () = pipe.query(&mut r.conn).unwrap();
    }

    #[test]
    fn a_glob_in_a_prefix_matches_literally() {
        assert_eq!(glob_escape("a*b?c[d]\\"), "a\\*b\\?c\\[d\\]\\\\");
        assert_eq!(glob_escape("ixr:inflight:"), "ixr:inflight:");
    }

    /// Tracing: TC-029-08
    /// ACs: FR-029-AC-8
    ///
    /// A prefix holding glob characters matches only itself: under `rv*[x]:` a
    /// decoy key that the unescaped pattern would match is not returned. A scan
    /// that did not escape returns the decoy.
    #[test]
    fn a_prefix_with_glob_characters_never_matches_a_neighbour() {
        let Some(mut r) = relay("scan-glob", "rv*[x]:") else {
            return;
        };
        let head = r.keys.inflight_stem();
        let decoy = format!("test:{}:scan-glob:rvZZx:inflight:decoy", std::process::id());
        plant(&mut r, [format!("{head}a"), format!("{head}b"), decoy]);
        let found = r.scan_prefixed(&head, 100).unwrap();
        assert_eq!(found.keys, [format!("{head}a"), format!("{head}b")]);
        assert_eq!(found.cut, None);
    }

    /// Tracing: TC-029-08
    /// ACs: FR-029-AC-8
    ///
    /// Both bounds cut and say so: at most `max_keys` keys come back, and after
    /// `max_calls` calls the scan stops with `truncated` set even though the
    /// cursor has not come back to zero.
    #[test]
    fn the_scan_stops_at_its_key_bound_and_at_its_call_bound_and_says_so() {
        let Some(mut r) = relay("scan-bounds", "") else {
            return;
        };
        let head = r.keys.scratch_head("k:");
        plant(&mut r, (0..300).map(|i| format!("{head}{i:04}")));

        let all = r.scan_prefixed(&head, 1000).unwrap();
        assert_eq!((all.keys.len(), all.cut), (300, None));

        let keys_cut = r.scan_prefixed(&head, 50).unwrap();
        assert_eq!(
            (keys_cut.keys.len(), keys_cut.cut),
            (50, Some(ScanCut::Keys))
        );

        let limits = ScanLimits {
            max_keys: 1000,
            max_calls: 2,
            count: 10,
        };
        let calls_cut = r.scan_limited(&head, limits).unwrap();
        assert_eq!(
            calls_cut.cut,
            Some(ScanCut::Calls),
            "two calls of 10 cannot cover the keyspace"
        );
        assert!(calls_cut.keys.len() < 300);
    }

    /// A scan with a deadline that has passed makes no call and says why; one with
    /// time left finds everything.
    #[test]
    fn a_passed_deadline_stops_a_scan_before_its_first_call() {
        let Some(mut r) = relay("scan-deadline", "") else {
            return;
        };
        let head = r.keys.scratch_head("d:");
        plant(&mut r, (0..5).map(|i| format!("{head}{i}")));
        r.set_scan_deadline(Some(std::time::Instant::now()));
        let cut = r.scan_prefixed(&head, 100).unwrap();
        assert_eq!((cut.keys.len(), cut.cut), (0, Some(ScanCut::Deadline)));
        r.set_scan_deadline(Some(std::time::Instant::now() + Duration::from_secs(30)));
        let all = r.scan_prefixed(&head, 100).unwrap();
        assert_eq!((all.keys.len(), all.cut), (5, None));
    }
}
