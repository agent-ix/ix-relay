---
id: SR-066
title: "Code review — ix-relay stranded-mail forwarding, claude-only socket, windowed drift, hook-failure severity, spool at-most-once"
type: SpecReview
analysis: code-review
scope: "ix-relay fix/doctor-signal-stranded-mail 9875a0f..02691bd; crates/relay-core (store.rs, store/stranded.rs, store/lua/forward.lua, store/lua/bury.lua, store/lua/pop.lua, store/inflight.rs, store/keys.rs, message.rs, socket_status.rs, tests/stranded.rs, tests/forwarded_render.rs, tests/store.rs, store/stranded_fault_tests.rs, store/pop_budget_tests.rs); crates/agent-msg (spool.rs, hook_run.rs, hook_run/tests.rs, doctor/checks.rs, doctor/facts.rs, tests/hook_failures.rs, tests/stranded_notice.rs); crates/relay-watcher (stranded.rs, main.rs, watch.rs, deliver.rs, socket.rs, socket/deliverer.rs, socket/session.rs, socket/transcript.rs, socket/tests.rs, otlp/relay.rs); crates/claude-wire (receipt.rs, transcript.rs, lib.rs)"
review_set: subset
---

## Summary

Rust review (dev-tools code-review dispatched to rust-review) of the code and tests for
FR-029, FR-030, FR-046, FR-053, FR-054 and FR-055. Specs bcc889e/be97a16/48f0516 were
reviewed earlier (SR-065). The Lua scripts are atomic and write the destination before
they delete the source, so no message is lost or duplicated in normal operation. Four
problems need fixing before merge: the rule that counts each hook-failure id once is
implemented twice and the two copies already disagree; the new process-liveness check runs
`ps` on every tick while holding the deliverer lock; a watcher restart loses the address a
stranded registration was written to; and two FR-053 tests can pass without testing anything.

## Verdict

**FAIL**: FND-001 is a duplication finding, which the code-review rubric records as `high`.
The fix is small. Everything else is `medium` or `low`.

## Findings

| ID      | Severity | Summary | Refs |
| ------- | -------- | ------- | ---- |
| FND-001 | high | The rule "count each hook-failure record id once" exists twice and the copies differ. `HookHealth::distinct` (used by the OTLP gauge) reads only the stream's `id` field. Doctor's `bus_failures` uses its own `HashSet` filter and falls back to the `id` inside `rec`. A stream entry that has no `id` field but whose record has one (for example, a new id-bearing record shipped by a not-yet-upgraded binary's flush) is counted once by doctor and once per copy by the OTLP gauge. Fix: move the record-id fallback into `hook_health_of` and have doctor call `distinct()`, so only one implementation remains. | crates/relay-core/src/store.rs:1180; crates/agent-msg/src/doctor/checks.rs:1369; crates/agent-msg/src/doctor/facts.rs:506; crates/relay-watcher/src/otlp/relay.rs:208 |
| FND-002 | medium | `SocketDeliverer::registry_read` runs on every tick, including each message wake. It calls `procs.start_time(pid)` for every resolved lead while holding the state mutex. On macOS each call forks `/bin/ps` with a 1 s timeout. The doc comment says "nothing waits: this is a few table lookups", and FR-055 says entries are dropped "without delaying ... any delivery". Neither holds: N leads add N × (fork+ps) before each tick's delivery, about 10–50 ms each normally and up to 1 s under load. The heartbeat is not at real risk (TTL is max(3×sweep, 30 s) = 45 s by default, which would take about 30 leads all timing out). Fix: copy (lead, pid, started) out of the lock, run the checks unlocked, and re-take the lock only to `retain`. Run the process half at most once per place-refresh period (or per-lead every N s); the registration half can stay per-tick. | crates/relay-watcher/src/socket/deliverer.rs:784-798; crates/relay-watcher/src/socket/session.rs:43-70; crates/relay-watcher/src/main.rs:835 |
| FND-003 | medium | The only record of a stranded registration's address is `Stranded::addresses`, kept in memory. After a watcher restart (or a crash, or this deploy) a registration whose place was already invalidated has address `None`. `pass` then never looks for a holder, and at 24 h the mail is dead-lettered and the registration pruned, even when exactly one verified registration holds the address. FR-053 says that case is forwarded. On rollout this hits every registration that is already stranded. Fix: for `Left::Replaced`, take the address from `layout.places[reg.herdr_pane]` (the same pane, now verified for the new session), which needs no memory. For `Left::Gone`, keep the last verified address on the bus (do not null `place` on invalidation; mark it unverified instead) or persist it in the watcher state file. | crates/relay-watcher/src/stranded.rs:40, 156-163; crates/relay-watcher/src/main.rs:483-497 |
| FND-004 | medium | TC-053-04 and the two-holder "ancient" case of TC-053-05 move `last_seen_ms` back on a copy (`ancient`) but never store that copy. The stored record is the 1-hour-old `old`. If `left()` or `holder()` were wrong and the pass reached `bury`/`forward`, the compare-and-set would see a changed record and return `Changed`/`None`, and the test's "nothing moved, nothing pruned, no log" assertions would still pass. Fix: `put(&mut bus, &[&ancient])` before each pass, and add unit assertions on `left()` / `holder()` for each layout. | crates/relay-watcher/src/stranded.rs:461-480, 503-512 |
| FND-005 | low | `Message` is `#[serde(deny_unknown_fields)]`. An un-upgraded `agent-msg` that pops a forwarded entry rejects the whole entry, so the batch fails to decode and goes into redelivery and then dead-letter. FR-053's Dependencies says such a binary "may not draw the notice line", which understates the effect. NFR-004 accepts the skew. Fix: change the spec sentence (no code change). | crates/relay-core/src/message.rs:50; crates/relay-core/src/store/lua/forward.lua:22-27 |
| FND-006 | low | Reply-socket drift lines name no lead, so they are counted against the version of the lead most recently posted to (`Drift::last_posted`). FR-046 says "a lead's version shall be the version its session record reported" and does not mention this approximation. With two Claude Code versions on one machine, a late line can be counted against the wrong version. Fix: state the approximation in FR-046 and in doctor's detail line, or attribute by the peer pid (`LOCAL_PEERPID` / `SO_PEERCRED` mapped to a resolved lead) if that is ever needed. | crates/relay-watcher/src/socket/deliverer.rs:529-583, 2053-2062 |

## Rust checks

- **Atomicity (forward.lua, bury.lua, hook-health script).** Each script checks every
  key's type before its first write and runs as one EVAL. Both stranded scripts
  compare-and-set against the raw registration bytes the caller read. forward.lua
  LPUSHes the destination (in reverse order, so relative order is kept and the moved mail
  comes before the holder's own) before it LTRIMs the source. bury.lua RPUSHes to the
  dead-letter list before trimming the inbox and HDELs the registration last. The mark
  splice cannot be forged: a sender's text is JSON-escaped, so `,"forwarded":true` cannot
  appear in it as written. The pop weight adds `FORWARDED_NOTICE_MAX` only for marked
  entries, so the NFR-002 output bound holds. The hook-health script reads `TIME` before
  it writes (store.rs and common.lua already do this). It dedupes within a chunk and across
  calls and caps the shipped-id ZSET at 2,000 / 24 h. No loss or duplication path found.
- **Spool removal by id.** `Flush::finish` re-reads from `window_start`, drops only whole
  lines whose id was shipped (plus junk lines up to the cut), and rewrites under the file
  lock in append mode. A record appended during the flush, or a line that was partial when
  read and is now complete, is kept. This is better than the old positional cut.
- **Panics on untrusted input:** none added on request paths. Lock poisoning is handled
  with `unwrap_or_else(PoisonError::into_inner)`, and the drift text is cut and neutralised.
- **Bounds (NFR-002):** drift versions ≤ 16, logged pairs ≤ 128, buckets limited to 24 h,
  the shipped-id set ≤ 2,000, and the hook-failure tally is bounded by the stream cap of
  1,000. The stranded pass costs one `inbox_lens` round trip plus one script per candidate.
- **FR-016:** every doctor detail line goes through `report.rs`'s `sanitise_line`, and the
  notice is on the signed first line with the sender text neutralised (property-tested in
  forwarded_render.rs).
- **Hook budget (NFR-001):** the flush still ships in `FLUSH_CHUNK` = 200 pieces, each
  gated on the remaining budget, with one EVALSHA per chunk. No new hook round trip.

## Gates (run at 02691bd)

- `cargo fmt --check`: pass (exit 0; nightly-only rustfmt options warned).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: pass.
- `cargo deny check licenses`: pass. `make audit-unsafe`, `make check-matrix`: pass.
- `cargo test --workspace --no-fail-fast` against a throwaway redis-server 7: see
  GATE-RESULT below.

Result: the second run (`--no-fail-fast`) exited 0 and all 50 test binaries passed. The
first run had 4 failures in `agent-msg --test hook` (`herdr call timed out after 2s`). That
run executed right after clippy, on a loaded machine. The failures did not happen again on
the rerun, and the base commit 9875a0f passed the same test binary 17/17. They are a
timing flake under load, not a regression on this branch.

## Judgement calls

1. Reply-socket drift attributed to the version of the last lead posted to: **keep**, but document it (FND-006).
2. Id-less spool records from older binaries dropped on upgrade: **keep**. They are counted in `dropped`, they are diagnostic only, and NFR-004 rules out a compatibility layer.
3. bury.lua refusing while a batch is in flight: **keep**. The lease returns the batch and the next pass buries it, so the pass converges. Burying in-flight rows would race the hook's ack.
4. Forwarding when the old pane is gone and exactly one verified holder exists: **keep the code** and amend FR-053's "A new session holds the address" bullet, which says "replaced" only (see SR-067).
5. `recipient-pruned` reason not stored: **keep as partial** with a tracking ticket against FR-031's reason field. TC-053-10/13 assert only the log line (see SR-067).
