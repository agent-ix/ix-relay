---
id: SR-030
title: "Code review (with Rust lane) of ix-relay PR #35, AGE-2064: OTLP exporter address cache"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay@d2c1dc842ed1f01a4c7bfc773df3050e64256a5a; crates/relay-watcher/src/otlp/http.rs (Resolve, SystemResolver, CachedAddrs, HttpTransport::new/with_resolver/addrs/connect/post, tc_045_24_* tests); read for context: crates/relay-watcher/src/otlp/export.rs (Exporter thread, Mailbox, Sender::send), crates/relay-watcher/src/otlp/mod.rs (Publisher::start)"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay/FR-045"
    type: "reviews"
---
# SR-030: Code review (with Rust lane) of ix-relay PR #35, AGE-2064

## Summary

Ticket: AGE-2064. The PR caches the collector's resolved addresses in `HttpTransport` for
`ADDR_CACHE_TTL` (300 s). A failed or empty lookup is not cached. The cache is dropped
when every address fails to connect. The resolver and the clock are injectable through
`HttpTransport::with_resolver`.

**Delivery-path isolation holds.** The lookup runs inside `HttpTransport::post`. Only the
`otlp-exporter` thread calls `post` (`export.rs` `Exporter::start`). The watcher hands
snapshots over with `Exporter::submit`, which pushes into a two-slot mailbox that drops
the oldest when full and returns at once. The tick loop never waits on the exporter, so a
hung `getaddrinfo` can stall only the exporter thread. The `stuck` status (a send over
30 s) makes such a stall visible. This is the same as `main`. The PR does not move the
lookup, and it cannot slow or block delivery.

**Does it meet the ticket's aim?** Mostly. On `main` the name was resolved inside
`connect`, after `post` had already fixed the 5 s deadline. A 15 s lookup therefore made
*every* push fail with `timeout`. Now the slow lookup is paid once per TTL. Measured on a
scratch copy (100 ms/300 ms timeouts, a 400 ms fake lookup): the first push returns
`Err(Timeout)` with the addresses cached, and the second returns 200 with no new lookup.
The first lookup, and the one after each expiry, are still unbounded: std has no
interruptible `getaddrinfo`, and the lookup is not moved to a thread of its own. They are
paid on the exporter thread only (see FND-001).

**Cache correctness.**

- The clock is `Instant` (monotonic), injected as `Box<dyn Fn() -> Instant + Send>`.
- `saturating_duration_since` makes a backwards step read as zero age, so the entry stays
  cached. No panic.
- At the TTL boundary, `< ADDR_CACHE_TTL` expires the entry at exactly 300 s. A test pins
  both 299 s and 300 s.
- The cache has no key. It belongs to one `HttpTransport`, which owns one immutable
  endpoint, and `Publisher::start` builds one transport per endpoint. No other endpoint
  can reuse an entry.
- Address order is what `getaddrinfo` returned, truncated to 4 in `SystemResolver`, as on
  `main`.

**Mutations** were run on a scratch copy with its own target dir.

| Mutation | Result |
| --- | --- |
| Never expire | killed (`tc_045_24_an_expired_cache_is_resolved_again`) |
| TTL `<` to `<=` | killed (same test) |
| Never drop the cache when every address fails | killed (`..._when_every_cached_address_fails_...`) |
| Cache a failed lookup | killed (`..._a_failed_or_empty_resolve_is_not_cached`) |
| Cache an empty lookup | survives, but equivalent: an empty list tries no address, so the drop after the loop clears it |
| Keep the expired entry when the lookup fails | survives, but equivalent: the entry is still expired, so the next call looks up again |
| Also drop the cache on the deadline early-return | survives: this path has no test (see FND-002 and SR-031) |

**Gates, on a scratch copy.** `cargo test -p relay-watcher otlp::http` passed 10 of 10
with `IX_RELAY_TEST_URL` unset, so the TC-045-24 tests do not need Redis. `cargo clippy
-p relay-watcher --all-targets -D warnings` is clean. `cargo fmt --check` is clean.
`make check-matrix` passes. The PR changes nothing under `.github`.

## Findings

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-001 | low | When the lookup takes longer than the 5 s request budget (the ticket's ~15 s missing-AAAA case), the push that pays for it always fails. `post` fixes the deadline before `connect` calls `addrs()`, so the lookup's time counts against the request budget. The first address then sees `left.is_zero()` and returns `Timeout`, although the addresses are good and now cached. At the default 30 s interval, one push in ten is lost and a backoff starts. This is still a large improvement on `main`, where every push was lost. It affects the exporter only, never delivery. Fix: take the deadline after the lookup, or give the lookup its own budget. Reproduced on a scratch copy. | crates/relay-watcher/src/otlp/http.rs:257-260, crates/relay-watcher/src/otlp/http.rs:204-210 |
| FND-002 | low | With 4 cached addresses that all time out (blackholed), the cache is never dropped. The time spent is 2 + 2 + 1 s, then the 4th address sees a zero budget and hits the early `return Err(TransportError::Timeout)`, which skips `self.cache = None`. Only the TTL clears the cache, which contradicts the FR-045 bullet "when every cached address fails to connect the exporter shall drop the cache". It is bounded at 5 minutes. Reproduced on a scratch copy (4 addresses in 192.0.2.0/24, timeouts of 100 ms and 250 ms: `Err(Timeout)` twice, one lookup, still cached). No test covers the early-return path: a mutation that also drops the cache there survives. Choose one behaviour, test it, and state it in the spec. | crates/relay-watcher/src/otlp/http.rs:206-218 |
| FND-003 | low | Expiry clears the good addresses before it looks up again (`self.cache = None`). A DNS failure at expiry therefore fails pushes, although the old addresses may still work. A lookup that is slow and then fails is paid on every push, because failures are not cached, which is by design. Stale-on-error (keep the expired entry when a re-resolve fails) would keep pushing through a DNS outage. This is a design note, and the behaviour is the same as `main` for failures. | crates/relay-watcher/src/otlp/http.rs:187-191 |
| FND-004 | low | Partial failure: when the first cached address is dead and the second is live, the cache is kept, which is correct. But the dead address is tried first on every push. That costs up to the 2 s connect timeout each push if it is blackholed, or nothing if it refuses. Order is never adapted, for example by moving the address that connected to the front. The push stays within its 5 s budget and it matches `main`. Judged acceptable, noted for the record. | crates/relay-watcher/src/otlp/http.rs:206-216 |
| FND-005 | low | The `MAX_ADDRS` cap moved from the transport into `SystemResolver`. `addrs()` no longer enforces it, so an injected resolver can return more than 4 addresses. The doc comment ("at most 4") and the spec describe it as a property of the transport. Apply `.take(MAX_ADDRS)` in `addrs()`. This is test-seam hygiene only: production uses `SystemResolver`. | crates/relay-watcher/src/otlp/http.rs:112-116, crates/relay-watcher/src/otlp/http.rs:188-199 |
| FND-006 | low | The test helper `dead_addr()` binds an ephemeral port and drops it. The port can be rebound by a parallel test or process before the connect, so the "dead" address may accept. The risk is small, and it would give a spurious 200 or a hang-free failure, not a false pass of the drop logic. Consider an address that is dead by construction, or keeping the listener bound but not accepting. | crates/relay-watcher/src/otlp/http.rs:625-628 |

## Rust lane (rust-review)

- **Panics and unwraps.** There is no `unwrap`, `expect` or indexing on the request path.
  All new `unwrap`s are in `#[cfg(test)]` code.
- **Lints.** No new `#[allow]`, no `unsafe`.
- **Trait seam.** `Resolve: Send` with `&self`, and a boxed `Fn() -> Instant + Send`
  clock. The seam is minimal, and `new` keeps its signature, so no caller changes.
  `with_resolver` is `pub` in a binary crate and used only by `new` and the tests. That is
  acceptable.
- **Blocking discipline.** A blocking `getaddrinfo` runs on a dedicated std thread, never
  on a tick. There is no lock: the cache is `&mut self` state.
- **Resource bounds.** The cache holds one entry of at most 4 addresses (via the system
  resolver, see FND-005). `addrs.clone()` per push is trivial.
- **Integers and time.** `Instant` arithmetic uses `saturating_duration_since`. The TTL
  constant is a `Duration`.
- **Tests.** No sleeps: a stepped `AtomicU64` clock and a scripted resolver are used. The
  `Tracing: TC-045-24` and `ACs: FR-045-AC-24` tags are on all four tests.
- **Thread or leak on timeout.** There is none. No thread is spawned per lookup, and a
  hung lookup holds only the exporter thread. That thread is not joined on drop, as
  before.

## Verdict

Approve. All six findings are low. Nothing is above low in the delivery path. The exporter
still runs on its own thread behind a non-blocking, bounded mailbox, and the lookup
happens only there. FND-001 and FND-002 are worth fixing in this PR because they are
small, but neither blocks a merge: **merge if only lows remain**.
