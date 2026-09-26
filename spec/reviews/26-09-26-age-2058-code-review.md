---
id: SR-014
title: "Code review (with Rust lane) of ix-relay PR #31, AGE-2058: OTLP push from relay-watcher"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay@2fcdbd10a3c970968b674317ce17a543ee0b810a; crates/relay-watcher/src/otlp/{mod,encode,export,http,panes,registry,relay,series,test_collector,test_herdr}.rs, crates/relay-watcher/src/main.rs, crates/relay-watcher/src/liveness.rs, crates/relay-watcher/tests/startup.rs, crates/relay-core/src/config.rs, crates/relay-core/src/config/otlp.rs, crates/relay-core/src/health.rs, crates/relay-core/src/test_env.rs, crates/relay-core/tests/health.rs, crates/relay-core/tests/wire_golden.rs, crates/agent-msg/src/doctor/{checks,facts,report}.rs, crates/agent-msg/tests/doctor.rs, crates/agent-msg/tests/watcher_only_config.rs, README.md; removed crates/relay-watcher/src/metrics.rs, crates/relay-watcher/src/metrics_server.rs"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay/FR-045"
    type: "reviews"
---
# SR-014: Code review (with Rust lane) of ix-relay PR #31, AGE-2058

## Summary

Ticket: AGE-2058. PR: ix-relay#31, head 2fcdbd1. The PR replaces the watcher's Prometheus
pull endpoint (AGE-2042) with an OTLP/HTTP JSON push on a dedicated exporter thread.
The Rust lane (rust-review) is folded into this file.

Method: a read-only review of the worktree. The worktree's own target dir was reused. A
throwaway `redis:7` ran on 127.0.0.1:33077. Mutants and probe tests were built from a
scratch `git archive` copy of the source. Fake collectors (Python, on 127.0.0.1 only) ran
against the built binary with `PATH=/nonexistent`, so herdr was never run. Nothing was sent
to any non-loopback address. The full `make ci` gate was not re-run (the lead runs it).
`cargo test -p relay-watcher -p relay-core -p agent-msg -p relay-ops` passed with
`IX_RELAY_TEST_URL` set. `make check-matrix` passed.

The exporter design holds up under attack. A dead or hostile collector never delayed a
tick, and neither the thread count nor memory grew. The wire format passes a strict parse
by the official OTLP proto. Three medium findings remain: a config value that panics the
delivery thread, the cost of the per-push Redis read on the tick, and ambient OTel variables
leaking into the watcher.

## Findings

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-001 | medium | `otlp_interval_secs` and `IX_RELAY_WATCHER_OTLP_INTERVAL_SECS` have a lower bound (5) and no upper bound. `publish_if_due` computes `now + a.interval`, and `Instant + Duration` panics on overflow. Measured on the built binary with the variable at 18446744073709551615, and again with the file key at 9223372036854775807: the watcher logs "pushing metrics ... every 9223372036854775807s", then the main (delivery) thread panics with "overflow when adding duration to instant" on the first push. Exit is 101. `agent-msg doctor` gives no warning for that file. The config is accepted, then the delivery-critical process dies, and under a supervisor it crash-loops. This contradicts FR-045 ("refused at start-up with exit 1") and NFR-003. Fix: bound the interval (for example at most a day) in `config/otlp.rs`, and use `checked_add` in `publish_if_due`. | crates/relay-watcher/src/otlp/mod.rs:252, crates/relay-core/src/config/otlp.rs:431-446, crates/relay-core/src/config/otlp.rs:526-543 |
| FND-002 | medium | The per-push store read runs on the delivery tick and is not cheap. `relay::read` calls `inbox_overviews` (one pipeline), `overdue_leases` (TIME plus one pipeline of LLEN and LRANGE of 200 rows per lead), `backlog()` and `hook_health(1000)`. `backlog()` SCANs the whole keyspace for both the in-flight and dead-letter prefixes, up to 64 calls each at COUNT 1000, across every machine's leads, plus LLEN pipelines, only to keep this machine's local leads. A probe on a scratch copy used the `an_idle_tick_is_two_requests_and_no_herdr_call` setup with a recording publisher. The idle push tick made 16 requests (HGETALL, EVAL+3 LLEN, 3×LLEN/LINDEX, TIME, 3×LLEN/LRANGE, 10×SCAN, XRANGE) where the FR-038-AC-5 pin allows 2. The pin test runs with OTLP disabled, so nothing pins or states the cost of a push tick. The local dead-letter lengths could be one pipelined LLEN over known keys. Also, `publish_if_due(...)?` sits before `state.registry = ...` and `prune_refused`, so a connection error in a metrics-only read aborts the rest of the tick. | crates/relay-watcher/src/otlp/relay.rs:136-219, crates/relay-watcher/src/main.rs:893-900, crates/relay-core/src/store/inflight.rs:557-600, crates/relay-watcher/src/main.rs:3469-3506 |
| FND-003 | medium | The watcher now reads the generic OpenTelemetry variables, which other emitters on the same host set. Claude Code, whose metrics this feature joins against, is configured through `OTEL_EXPORTER_OTLP_ENDPOINT`/`_HEADERS`/`_PROTOCOL`. Three effects follow. (a) An inherited `https://` endpoint makes `relay-watcher` exit 1 at start-up (`an_https_endpoint_is_refused_at_startup_naming_its_source`): a telemetry variable stops delivery. (b) An inherited gRPC endpoint (`:4317`, protocol grpc) is accepted and fails forever, because `OTEL_EXPORTER_OTLP_PROTOCOL` is not read. (c) Endpoint and headers resolve per key, each from its own tier. With `otlp_endpoint` in the file and `OTEL_EXPORTER_OTLP_HEADERS` in the environment (credentials meant for another collector), the watcher sends those credentials over plain http to the file's collector. The headers should come from the same tier as the endpoint, and a relay-specific opt-in (or a non-fatal refusal of an https env endpoint) would keep a foreign variable from stopping the watcher. | crates/relay-core/src/config/otlp.rs:485-524, README.md:286-293 |
| FND-004 | low | `hook_health(HOOK_ENTRIES)` is `XRANGE - + COUNT 1000`, which returns the oldest 1,000 entries. The stream is written with `MAXLEN ~ 1000`. Measured on redis:7: after 1,099 XADDs, XLEN is 1099. So up to about 99 of the newest failures are left out of `ix.relay.hook.failures.recent`, the gauge whose name says "recent". Use `XREVRANGE + - COUNT 1000`, or a reverse-read store call. | crates/relay-watcher/src/otlp/relay.rs:195-217, crates/relay-core/src/store.rs:499-503, crates/relay-core/src/store.rs:40-41 |
| FND-005 | low | The exporter's `Stats` counters (`sent`, `failed`, `dropped_snapshots`, `rejected_points`) are written and never read outside tests. They are not in the heartbeat, `doctor`, logs or any series. A collector that answers 200 with `partialSuccess.rejectedDataPoints` for every point, which is how a silently dropped encoding would show, reads as `otlp: sending`. So do snapshots dropped while backing off. FR-045 says these are "counted", but the counts are unobservable. | crates/relay-watcher/src/otlp/export.rs:130-142, crates/relay-watcher/src/otlp/export.rs:344-366, crates/relay-core/src/health.rs:66-77 |
| FND-006 | low | DNS resolution (`(host, port).to_socket_addrs()`) runs before the request deadline starts to bite, and has no bound of its own (std has no resolver timeout). A hanging resolver holds the exporter thread; the tick is not affected, because `submit` only locks the mailbox. The heartbeat keeps the previous state (`sending`) and carries no last-success time, so `doctor` cannot show a wedged exporter. The glibc resolver timeouts bound this in practice. This was not measured: the host resolver was not tampered with. | crates/relay-watcher/src/otlp/http.rs:118-139, crates/relay-core/src/health.rs:66-80 |
| FND-007 | low | `percent_decode` uses `u8::from_str_radix(hex, 16)`, which accepts a leading `+`, so `%+a` decodes to LF. Measured: `otlp_headers = "a=s3cret%+a"` is refused only by the later control-character check ("header 1 has a control character"). Resource attributes are sanitised later. There is no injection, but the decoder is laxer than RFC 3986. Check that both bytes are ASCII hex digits. | crates/relay-core/src/config/otlp.rs:290-305 |
| FND-008 | low | A `Retry-After` that does not fit in u64 (measured: `99999999999999999999`) fails `parse()` and is treated as absent, so the sender backs off 1, 2, 4, 8 s instead of the 5-minute cap. A huge request to wait is read as no request to wait. | crates/relay-watcher/src/otlp/http.rs:291, crates/relay-watcher/src/otlp/export.rs:193-201 |
| FND-009 | low | The start-up line "N malformed pair(s) of OTEL_RESOURCE_ATTRIBUTES ignored" is printed with no endpoint configured and with `--once` (measured: no endpoint, `OTEL_RESOURCE_ATTRIBUTES=bad`). FR-045-AC-1 says nothing about OTLP is logged without an endpoint. The AC-1 test filters on the substrings `metrics`/`otlp`, which this line does not contain. | crates/relay-watcher/src/main.rs:557-563, crates/relay-watcher/tests/startup.rs:318-330 |

## Rust lane (rust-review)

- Panic surface: FND-001 (`Instant + Duration`) is the one reachable panic on a request
  path. `unwrap`/`expect` appear only inside `#[cfg(test)]` modules. `catch_unwind`
  wraps the send path, and the mailbox and status locks recover from poisoning.
- Blocking and locks: the tick holds the mailbox mutex only for a `VecDeque` push. Network
  I/O happens only on `otlp-exporter`. Redis reads for the health series run on the tick
  (FND-002). `Drop` closes the mailbox and does not join, which is deliberate and documented.
- Integer conversions: `n as u64` / `len() as u64` from `usize` (relay.rs:61-76, 142) are
  lossless on the supported 64-bit targets. `i64::try_from(...).unwrap_or(i64::MAX)` is
  used for gauges. Nanosecond timestamps are `u64` via `try_from`. Fine.
- Resource bounds: queue 2, 6,000 points, 4 MiB request (checked per point before
  pushing), 64 KiB response (buffer at most 64 KiB + 4 KiB), 16 headers × 1 KiB, 32
  resource attributes, 2,000 panes, 50 leads, 64 hook groups. All are enforced. The
  interval has no upper bound (FND-001).
- Style: `encode` serialises every point twice (once to size it, once in the request),
  which is acceptable at 6,000 points per 30 s. The `Instrument` statics plus
  `CATALOGUE` form a clean, reusable registry seam, as the owner asked.

## Verification performed

- **Pull removal is complete.** Nothing matches `metrics_port`, `metrics_listen`,
  `IX_RELAY_WATCHER_METRICS_*`, `metrics_server`, `MetricsStatus`, `watcher_metrics_*` or
  `9464` outside the review archives. `TcpListener` in relay-watcher appears only in tests.
  The built binary held no listening socket in 11 runs, read from `/proc/<pid>/net/tcp*`
  joined to `/proc/<pid>/fd`: with an endpoint in every collector mode, and without one
  (1 thread, 1 socket, Redis only). No link names the old FR-045 file.
- **Mixed version.** An old reader sees no `metrics` member and prints "unknown".
  `WatcherHeartbeat` has no `deny_unknown_fields`, so a new reader ignores an old
  `metrics` member and reads `otlp` as None ("unknown"). The `WatcherFile` golden is
  unchanged. The heartbeat goldens changed on purpose, because the member was renamed.
- **OTLP correctness, independent oracle.** The official `opentelemetry-proto` 1.40
  `ExportMetricsServiceRequest` with protobuf 6.33 `json_format.Parse(...,
  ignore_unknown_fields=False)`, found offline in a local venv with nothing downloaded,
  strictly parsed three bodies: the byte-exact golden, a full 10-instrument body built
  through `Publisher::build` with panes, leads, a hostile lead name and hooks, and a live
  body captured from the binary. Checks passed on all three: sums are cumulative (2) and
  monotonic with a start time; histograms have `len(bucketCounts) == len(explicitBounds)+1`
  and `sum(bucketCounts) == count`; int64 values and timestamps are strings; attributes are
  `stringValue`; the resource carries `service.name`, `host.name` and `service.version`;
  the scope has a name and version. The metric descriptions state what is counted (the
  delivered scope, and the hook gauge's capped-stream semantics).
- **Exporter safety, measured on the binary** (sweep 1 s, push every 5 s, 25 s per mode).
  Modes: collector accepts and never answers; 500; 429 with Retry-After 10^20; 429 with
  999999; 5 KB of garbage; an endless body; a 307 redirect; a slow-loris answer at one byte
  a second; and a listener with a full backlog, which blackholes the connect. In every mode
  `last_tick_ms` advanced about once a second, threads stayed at 2 and RSS was flat
  (8.6-8.7 MB). Status went `failing` with `timeout`, `http_500`, `http_429`,
  `bad_response`, `http_307` or `timeout`, and the endless body counts as a truncated 200.
  Each outage logged one line. Retry-After 999999 was capped (one request in 25 s). The
  blackholed connect timed out within 2 s. The watcher never exited. The captured request
  head held `Authorization` only on the wire, and no header value appeared in stderr or
  the heartbeat.
- **Config.** On the file keys, `agent-msg who` exited 0 and `doctor` warned
  `watcher_otlp_config_invalid`, naming the key and line, for: `otlp_endpoint = 5`,
  `"ftp://x"`, `"http://"`, a 5,000-byte URL, `otlp_headers` with a literal CR/LF, `%0d%0a`
  and `%+a`. The watcher refused the same values with exit 1, with no secret in the
  message. The exception is the huge interval (FND-001). Env precedence: the metrics
  endpoint is used as written (root `/` when no path), and the general one has
  `/v1/metrics` appended to its path. This matches the OTel exporter spec. The file key is
  a base URL.
- **Mutants** (scratch copy, reverted). Each killed the named test. Sanitiser removed: 5
  tests fail. Point cap removed: `tc_045_09_the_registry_is_bounded_in_points`. Request
  deadline made 1 h: `tc_045_10_a_silent_or_dripping_collector_is_cut_at_the_request_deadline`
  hangs and the harness times out, as tests.md states. Retry-After bound removed:
  `tc_045_13_retry_after_is_honoured_and_bounded`.
- **Tree hygiene.** `git diff HEAD` is empty. There are no untracked or ignored files
  besides `target/`, no proptest-regressions, no `.github` change and no new lint allow.
  Every test tests.md names for FR-045 exists.

## Verdict

**CHANGES REQUESTED.** The design, the wire format and the isolation of the exporter are
sound, and the claims about them were verified. Three medium findings should be fixed
before merge. FND-001 needs a bound and `checked_add`. FND-003 needs the headers taken
from the endpoint's tier, and an inherited https endpoint must not stop the watcher.
FND-002 needs the scan replaced by a pipelined LLEN of local keys, the push tick's cost
stated and pinned, and the `?` not aborting the rest of the tick. The low findings can be
fixed in the same round or deferred.

## Dispositions

Disposition pass 1, reviewed at agent-ix/ix-relay@3ff496ca5aa075ef5ad69787e0be38f0b119e0f8 (fix
commits 5552a29 and 3ff496c; 3ff496c only edits two rustdoc links). Each outcome was
checked against the code and measured on a release build of that head (own target dir,
throwaway redis:7, fake collectors on 127.0.0.1 only, `PATH=/nonexistent`).

| FND | outcome | sha/reason |
|-----|---------|------------|
| FND-001 | fixed | 5552a29: interval bounded 5..3600 in env and file; `next_due_after` and the request deadline use `checked_add`. Built binary: env 18446744073709551615, 9223372036854775807, 3601, 4, 0, -1, abc, 1e3 and file 9223372036854775807, 3601, 4, 0, -1, "abc", 1.5 all exit 1 naming the variable or file line; 5 and 3600 run; no panic; doctor warns `watcher_otlp_config_invalid` for each bad file value |
| FND-002 | fixed | 5552a29: push read is last in the tick and never returns an error; `lead_overviews` (one pipeline) plus `hook_health_newest`, no SCAN. MONITOR on redis:7 with 0/5/50 local leads: no SCAN or TIME; each push adds one XREVRANGE and 4 commands per lead in one pipeline; idle ticks unchanged. Fault injection (wrong-type keys): logged once over 4 pushes, ticks and pushes continue. `tc_045_22_*` pins 2+2 requests with OTLP on |
| FND-003 | fixed | 5552a29: relay settings first; generic `OTEL_*` only as a fallback, only http:// with protocol unset or http/json, otherwise ignored with one log line and heartbeat `ignoring`, never an exit; headers from the endpoint's tier only. 31 combinations run on the built binary against a loopback collector (see below) |
| FND-004 | fixed | 5552a29: `hook_health_newest` reads `XREVRANGE + - COUNT 1000`; `tc_045_23_*` (1,100 entries, asserts the stream held more than 1,000) fails when reverted to `hook_health` (mutant run) |
| FND-005 | fixed | 5552a29: `OtlpCounters` ride in the heartbeat and doctor; a 2xx rejecting every point is `failing` `rejected_points`. Built binary against a collector rejecting every point: heartbeat `failing`, reason `rejected_points`, counters sent 0 failed 4 rejected_points 24, one log line |
| FND-006 | fixed | 5552a29: not bounded (std has no resolver timeout) but made visible: a send running over 30 s reads `failing` with reason `stuck` (`STUCK_AFTER`); unit-tested on an injected clock; the host resolver was not tampered with |
| FND-007 | fixed | 5552a29: both escape bytes must be ASCII hex digits; `%+a` is refused as a bad escape; a short or multi-byte escape returns None without panicking |
| FND-008 | fixed | 5552a29: an all-digit `Retry-After` too large for u64 reads as u64::MAX and is capped at 5 min. Built binary against `Retry-After: 99999999999999999999`: one request in 16 s, 3 snapshots dropped |
| FND-009 | fixed | 5552a29: the skipped-pairs line is printed only when a push starts; nothing with no endpoint or with `--once` (measured on the binary); the AC-1 test now filters on otel/malformed/pair and runs with a malformed `OTEL_RESOURCE_ATTRIBUTES` |

Regression checks at this head: no listening socket (read from `/proc/<pid>/net/tcp*`
joined to the fd table) in 10 collector modes and with no endpoint; 1 thread with no or an
ignored endpoint, 2 with one; RSS flat (4.2-4.4 MB) over 16 s in every mode (reject_all,
silent, 500, 429 with a huge Retry-After, garbage, endless body, 307 redirect, slow-loris,
blackholed connect, nothing listening); tick cadence unchanged in every mode. A live body
from the binary parses strictly (`ignore_unknown_fields=False`) with the official
opentelemetry-proto `ExportMetricsServiceRequest` (protobuf 6.33.6, offline): cumulative
monotonic sum, histogram buckets consistent, resource and scope complete. Mutants
(reverted, tree identical to the commit afterwards): generic headers sent to a file
endpoint fails 3 config tests; a `backlog()` scan in the push read fails
`tc_045_22_a_push_tick_*`; the rejected-all branch removed fails `tc_045_18_*`. No
`.github` change, no new lint allow, no unwrap/expect/panic outside tests in the fix diff,
no new proptest-regressions, clean worktree. `cargo test -p relay-watcher -p relay-core -p
agent-msg` with `IX_RELAY_TEST_URL` passed with no Redis test skipped.

## New findings (disposition pass 1)

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-010 | low | A failed push read sends false zeros instead of leaving the series out. `relay::read` inserts a zero `LeadRow` for every local lead before the pipeline, and on an `Err` from `lead_overviews` (or a per-lead error) keeps those rows, so the push carries `ix.relay.inbox.depth`, `inbox.oldest_age`, `inflight.entries` and `deadletter.entries` at 0 for every lead. FR-045 says the watcher "shall ... leave the affected series out", and the log line says "those series are left out". Before this fix a connection error aborted the push; now it pushes the zeros, so a backlog reads as drained for that push. Measured: `tc_045_22_a_failing_push_read_*` with the body captured sends `ix.relay.inbox.depth{lead=a-lead}` `asInt "0"` after the cut; on the binary, a wrong-type in-flight key still exported the lead's four gauges. Fix: insert a lead's row only from a successful overview, and assert in the failing-read test that no per-lead point is sent. | crates/relay-watcher/src/otlp/relay.rs:151-172, crates/relay-watcher/src/otlp/mod.rs:311-313, spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md:118-121 |
