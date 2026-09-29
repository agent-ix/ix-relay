---
id: FR-045
title: "The watcher pushes relay and pane-name metrics over OTLP"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-006"
    type: "implements"
---
# FR-045: The watcher pushes relay and pane-name metrics over OTLP

## Description

Where an OTLP endpoint is configured, `relay-watcher` shall push OpenTelemetry metrics
to it over OTLP/HTTP: the herdr pane-to-name mapping of this machine, so that a
dashboard can turn the stable pane, tab and workspace ids that Claude Code's own metrics
carry into `machine:workspace:tab` names (and history keeps the name a pane had at the
time), and a small set of relay health series. Where no endpoint is configured, the
watcher shall send nothing.

This replaces the Prometheus pull endpoint of AGE-2042 (which was never deployed). The
watcher pushes because a collector outside the host cannot be assumed to reach a port on
it; it binds no listener, ever.

The exporter is built to be reused: a typed registry of instruments (name, unit, kind,
attributes), which the watcher fills from data it already reads, and one exporter that
encodes the registry as OTLP and sends it. A later series is an instrument, a row in the
catalogue below and the few lines that fill it.

## Wire format the relay writes

The watcher writes an OTLP `ExportMetricsServiceRequest` in the **JSON encoding of
OTLP/HTTP**: `POST <endpoint>/v1/metrics` with `Content-Type: application/json`, so no
protobuf dependency is needed. It follows the OTLP JSON mapping: lowerCamelCase member
names, 64-bit integers and nanosecond timestamps as decimal strings, enums as integers,
cumulative temporality (2), monotonic sums. Owner of the type:
`relay_watcher::otlp::encode`. It is emitted only, never read; the only thing read back
is the response, and it is read tolerantly (only `partialSuccess.rejectedDataPoints`, as a
string or a number, and nothing else in the body is required).

**Transport is plain `http://` only.** No TLS library is in the tree, so the watcher
shall refuse an `https://` endpoint of its own settings at start-up with a message that
names the key or variable, and shall ignore an inherited one (below). The collector
survey and the TLS question are recorded in the README and the pull request, not here.

## Behavior

### Configuration

- The configuration loader shall read `[watcher] otlp_endpoint` (a base URL, to which
  `/v1/metrics` is appended), `[watcher] otlp_headers` (`name=value,name=value`, values
  optionally percent-encoded) and `[watcher] otlp_interval_secs`, and the environment
  variables `IX_RELAY_WATCHER_OTLP_ENDPOINT` (a base URL), `IX_RELAY_WATCHER_OTLP_HEADERS`
  and `IX_RELAY_WATCHER_OTLP_INTERVAL_SECS`. These are the relay's own settings and are
  primary.
- The configuration loader shall consult the generic OpenTelemetry variables
  `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT` (used as written), `OTEL_EXPORTER_OTLP_ENDPOINT`
  (a base URL), `OTEL_EXPORTER_OTLP_HEADERS`, `OTEL_EXPORTER_OTLP_METRICS_PROTOCOL` and
  `OTEL_EXPORTER_OTLP_PROTOCOL` only when neither `IX_RELAY_WATCHER_OTLP_ENDPOINT` nor
  `[watcher] otlp_endpoint` is set: other emitters on the host (Claude Code among them)
  are configured through them. Precedence of the endpoint is therefore
  `IX_RELAY_WATCHER_OTLP_ENDPOINT`, then `[watcher] otlp_endpoint`, then the generic
  variables, then none, following [FR-026](./FR-026-configuration-precedence.md) with
  the generic variables as the last fallback. The configuration loader shall treat a
  set-but-empty variable as unset; no OTLP setting has a command-line flag.
- The configuration loader shall take the headers from the same tier as the endpoint
  and from no other: `IX_RELAY_WATCHER_OTLP_HEADERS` for the relay's environment
  endpoint, `[watcher] otlp_headers` for the file's, `OTEL_EXPORTER_OTLP_HEADERS` for a
  generic one. Headers of another tier shall not be sent, so credentials meant for one
  collector never go to another; the loader shall record each such setting as ignored.
- If an inherited generic setting is not usable here (an `https://` endpoint, a
  protocol of `grpc` or `http/protobuf`, an endpoint that is not an `http://` URL, or
  malformed headers), then the configuration loader shall ignore all of the generic
  settings, record which variable and why (never a value), and the watcher shall
  neither exit nor send. The watcher shall log one line for each ignored setting, and
  the heartbeat shall carry the first as `otlp` state `ignoring`.
- If a relay setting is present and unusable (an endpoint that is not an `http://` URL,
  holds credentials, a query string or a fragment, or has a port outside 1 to 65535; a
  header that is not `name=value`, has an invalid name, a control character or a bad
  percent escape, or a name the exporter sets itself (`Host`, `Content-Length`,
  `Content-Type`, `Connection`, `Transfer-Encoding`); more than 16 headers; an interval
  outside 5 to 3,600 seconds or not a number), then the configuration loader shall refuse
  it whichever tier supplied it, and `relay-watcher` shall exit 1 at start-up. The error
  shall name the environment variable for an environment value, and the file with the key
  and the real line and column for a file value. The configuration loader shall refuse an
  `https://` endpoint of a relay setting with a message that says this build sends plain
  http only. An error shall never contain a header value or the credentials of an
  endpoint.
- The configuration loader shall skip `OTEL_RESOURCE_ATTRIBUTES` pairs that are not
  `key=value`, as the OpenTelemetry specification says, and keep at most 32. The watcher
  shall log one line counting the skipped pairs only when a push is starting.
- The shared configuration type shall keep the three file keys as raw values, and only
  `relay-watcher` shall validate them: these watcher-only keys are validated by the
  watcher only. `agent-msg` and the hook shall load the same file successfully and ignore
  a bad value, so a typo in a watcher-only setting never stops hook delivery or `send`.
- If the file's OTLP keys hold a value the watcher would refuse, then
  `agent-msg doctor` shall report `watcher_otlp_config_invalid` as a warning, naming the
  key and position.
- The resource shall carry `service.name` (`OTEL_SERVICE_NAME`, else a `service.name` in
  `OTEL_RESOURCE_ATTRIBUTES`, else `ix-relay-watcher`), `host.name` (this machine's bus
  name), `service.version`, then the other `OTEL_RESOURCE_ATTRIBUTES` pairs for keys not
  already set.

### Start-up and isolation

- Where no OTLP setting is present in any tier, the watcher shall start no exporter,
  send nothing and log nothing about OTLP. Where a push starts, the watcher shall log one
  line naming the collector as `host:port` (never the path, a query or a header) and the
  interval. A watcher started with `--once` shall send nothing, start no exporter and log
  nothing about OTLP.
- The watcher shall bind no listening socket. It only connects out.
- The watcher shall push at most once per interval, the first push on the first tick and
  each later one on the first tick at or after the previous push plus the interval. The
  watcher shall hand the snapshot to a dedicated exporter thread through a queue of at
  most two snapshots that drops the oldest when full, and shall never wait on the
  collector: a slow, dead or hostile collector does not delay a tick, does not grow memory
  and does not end the watcher. The exporter thread shall be the only thing that talks to
  the collector.
- The watcher shall do its store reads for a push last in the tick, after the
  deliveries, the registry cache update and the prune, and no failure of those reads
  shall skip or fail any of them: the watcher shall log such a failure once per episode
  and leave the affected series out. A tick that pushes shall add at most two pipelined
  requests to the store, one for the local leads' queues and one for the newest hook
  failures, and no key scan; a tick inside the interval shall add none. The time
  arithmetic on the interval shall not overflow.

### Export

- The exporter shall connect with a 2 second timeout and finish the whole request within
  5 seconds, read at most 64 KiB of response, and not follow redirects. It shall treat a
  2xx as success (counting `partialSuccess.rejectedDataPoints`), a 429 or 5xx or a 3xx or
  a transport failure as retryable, and any other 4xx as a refusal of that request: not
  retried, no backoff, the next snapshot tried as usual. When every point of a push is
  reported rejected, the exporter shall treat the push as a failure with reason
  `rejected_points`, without a backoff.
- The exporter shall reuse the addresses it resolved for the collector's host name, at
  most 4, in the order resolved, for up to 5 minutes, so that a slow name server is paid
  once per lifetime and not on every push.
- The exporter shall start the 5 second request deadline after the lookup, so that a
  lookup slower than the deadline does not fail the push that pays for it.
- If a lookup fails or returns no address, then the exporter shall not cache it.
- If a lookup at the end of the 5 minutes fails or returns no address, then the exporter
  shall use the addresses it held and look the name up again on the next push.
- If every address was tried and failed to connect, then the exporter shall drop the
  cached addresses, so that the next push looks the name up again.
- If the request deadline ends a push before every address was tried, then the exporter
  shall keep the cached addresses, because no address was shown bad. The first address is
  tried first on every push; the order is not adapted.
- While the backoff after a retryable failure has not passed, the exporter shall send
  nothing: the backoff is 1 second, doubling per consecutive failure to 5 minutes, or the
  collector's `Retry-After` (in seconds, a value too large for 64 bits reading as the
  largest) when longer, bounded at 5 minutes. When a snapshot reaches the exporter while
  it backs off, the exporter shall drop it and count it. The exporter shall treat a panic
  in the send path as a failed push.
- The exporter shall log one line when an outage begins (naming the collector's `host:port`
  and a reason code) and one when it ends, and nothing in between.
- The exporter shall leave out, and count, the points of a request past 4 MiB.
- The exporter shall make every attribute value safe before it is sent: control,
  bidirectional and invisible characters (and newlines and tabs) replaced as in
  [FR-016](./FR-016-render-neutralises-control-characters.md), the value cut to 128
  characters, then JSON-escaped by the encoder. The registry shall hold at most 6,000
  points per push; for a second point for the same instrument and attributes it shall add
  the values of a sum and otherwise keep the first, and it shall drop a value the
  instrument's kind cannot hold, all counted.

### Status

- The heartbeat shall carry `otlp` as `disabled`, `ignoring` (with the note),
  `sending` (with the collector's `host:port`, the number of series left out or folded
  when not zero, and the exporter's counters: pushes sent and failed, snapshots dropped
  and points rejected, when not all zero) or `failing` (with `host:port`, a reason code
  such as `connect_failed`, `timeout`, `bad_response`, `io_error`, `http_503`,
  `rejected_points` or `stuck`, and the counters). The heartbeat shall report `failing`
  with reason `stuck` while one send has run longer than 30 seconds, which makes a hung
  name lookup visible. `agent-msg doctor` shall say which: `otlp: sending to <host:port>`,
  `otlp: disabled` (with the note when `ignoring`) or `otlp: unknown` in the detail
  lines, a detail line with the counters, and a warning `watcher_otlp_failing` for
  `failing`, never a failure. When a heartbeat has no `otlp` member (an older watcher) or
  one the reader cannot decode, the reader shall read it as `unknown`, with the rest of
  the heartbeat intact.

### Series

Phase 1 emits these instruments; the table is the catalogue in
`relay_watcher::otlp::series`, and a test compares the two, and the two with what a push
exports.

| Name | Kind | Unit | Attributes | Meaning |
|------|------|------|------------|---------|
| `herdr.pane.info` | gauge | `1` | machine, pane, tab, workspace, workspace_label, tab_label, session, bus_name | One series per live herdr pane of this machine, value 1. `bus_name` is the registered lead's name, empty for a pane no lead registered. |
| `herdr.pane.status` | gauge | `1` | machine, pane, status | The status herdr reports for each pane, value 1; join to `herdr.pane.info` on (machine, pane). |
| `ix.relay.messages` | sum | `{message}` | outcome | Messages by outcome as this watcher saw them, cumulative since it started: `delivered` (messages of batches this watcher delivered and saw confirmed: typed into a pane, or posted to a Claude Code inbox socket and confirmed within its wait; hook deliveries and a held message approved later are not counted; socket denials, postponements and deferrals are not counted at all), `requeued` (put back by a rejected prompt), `redelivered` (returned to an inbox by an expired lease), `dead_lettered` (moved to the dead letters by an expired lease). |
| `ix.relay.inbox.depth` | gauge | `{message}` | lead | Messages waiting in a local lead's inbox. |
| `ix.relay.inbox.oldest_age` | gauge | `s` | lead | Seconds the oldest waiting message has waited, 0 when empty. |
| `ix.relay.inflight.entries` | gauge | `{message}` | lead | Messages a local lead has had popped and not yet acknowledged (entries, not batches: a batch count needs the rows, which the push does not read). |
| `ix.relay.deadletter.entries` | gauge | `{message}` | lead | Dead-lettered messages held for a local lead. |
| `ix.relay.hook.failures.recent` | gauge | `{failure}` | stage, error_kind | Hook failures of this machine still in the capped `health:hooks` stream that happened since the watcher started. A gauge, not a counter: it falls when the stream drops old entries. |
| `ix.relay.watcher.tick.duration` | histogram | `s` | none | How long each watcher tick took, buckets 5 ms to 10 s, since the watcher started. |
| `ix.relay.registrations` | gauge | `{registration}` | machine | Leads registered on this machine. |
| `ix.relay.watcher.health_snapshot.read_failures` | sum | `{failure}` | snapshot | Failed reads used to build the relay-health snapshot, cumulative since this watcher started; `snapshot` is `lead_overviews` or `hook_health_newest`. |

The health-read failure sum starts at zero for both `snapshot` values and is
monotonic for the lifetime of this watcher process; it resets when the watcher
restarts and is not persisted. Each failed `lead_overviews` result for a local
lead counts once (including a per-lead Redis error such as a wrong-type queue
key); a top-level failure of the `lead_overviews` read counts once. A failed
`hook_health_newest` read counts once. These are counts of read results, not
outage episodes, failed fields, skipped entries or ticks: a successful read
with malformed hook records does not count as a read failure because that
reader intentionally skips malformed entries. The `snapshot` attribute has
exactly the two values shown above and is the only attribute on this
instrument, so at most two series are emitted. The metric has no machine,
host, lead, pane, Redis key, error text or other secret or high-cardinality
attribute; the collector's `host.name` resource remains as already specified.
Each push includes both cumulative points, including zero values, and a later
successful push reports failures accumulated while earlier reads failed.

**SWM-5 scope:** Linear SWM-5 has been respecified to match this requirement:
export Redis health-snapshot read failures through FR-045's existing OTLP push
path. This metric adds no Prometheus pull `/metrics` endpoint.

Not in phase 1: `ix.relay.inflight.batches`, `ix.relay.messages` outcomes `sent` and `denied` (a count from the log
stream would need a reader of its own), `ix.relay.delivery.latency` (send to
acknowledgement needs a send time per message that the watcher does not keep), a
`deliverer` attribute, escalation counts, and a last-tick age (the heartbeat has it).

- The watcher shall build the panes by the join `agent-msg find` uses
  ([FR-005](./FR-005-shown-address-rule.md)), not a copy of it, over the layout read it
  already makes for the place refresh (about every 30 seconds): no herdr call of its own.
  A registration on another machine is not a pane's name here, and a pane of another
  machine is not published: each machine's watcher publishes its own. When herdr lists one
  pane twice, the watcher shall publish one series for it, the first listed. A push holds
  at most 2,000 panes.
- When a layout read fails, the watcher shall keep publishing the last panes while they
  are at most three refresh periods (90 seconds) old, and leave them out after that, so
  that names that may be long gone are not published under a fresh timestamp. The relay
  series go on.
- The relay series come from what the watcher counts as it works (the counters and the
  histogram) and from two pipelined store requests per push (`lead_overviews`, one
  request over the local leads' own keys, and `hook_health_newest`, the newest 1,000 hook
  failures first), after the tick's deliveries. `lead` is an attribute only where it is bounded: at most 50 leads have
  series of their own and the rest are folded into `lead=other` (depth summed, oldest age
  the largest), and the number folded is reported in the heartbeat (`dropped`) and by
  `doctor`. No series carries message text. An error of any kind reading the store leaves those
  series out and does not reach the tick.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-045-AC-1 | Given no OTLP setting in any tier, the watcher starts no exporter, sends nothing and logs nothing about OTLP or metrics (not even a skipped `OTEL_RESOURCE_ATTRIBUTES` line); given `--once` and an endpoint, nothing is sent and nothing about pushing is logged; given an endpoint, one start-up line names `host:port` and never the path or a header value. | Test |
| FR-045-AC-2 | Given the relay's environment endpoint, the file's endpoint and the generic variables in every combination, the endpoint is the relay environment's, else the file's, else the generic one, else none, and the interval is the variable's, else the file's, else 30 s, each reported with its tier; the headers are those of the endpoint's tier and no other's (another tier's headers are recorded as ignored); a set-but-empty variable is unset. | Test |
| FR-045-AC-3 | Given a fake registry, the request is byte for byte the expected JSON; a label holding a quote, a backslash, a newline, ESC, a bidirectional or zero-width character, an attempt to close the string and add a member, or 10 KB of text reaches the output neutralised, cut and escaped, and the body parses as JSON with no injected member; the resource takes `service.name` from `OTEL_SERVICE_NAME`, else `OTEL_RESOURCE_ATTRIBUTES`, else the default; and a request over 4 MiB leaves out points and counts them, staying valid JSON. | Test |
| FR-045-AC-4 | Given a herdr view of another machine, or a registration naming another machine with the same pane id, no series carries that machine or that lead's name; a pane no lead registered has an empty `bus_name`; a pane listed twice is one series; the pane set is bounded at 2,000. | Test |
| FR-045-AC-5 | Given herdr's recorded output before and after a tab rename, the push built at the next refresh carries the new `tab_label` and none of the old one (injected clock); panes are published up to 90 seconds after their layout read and not after, while the relay series continue; and a watcher tick that reads a layout pushes it with its registered names, and one that cannot read it still succeeds and pushes. | Test |
| FR-045-AC-6 | Against a real Redis and a collector the test owns, the running watcher binary pushes OTLP JSON with the configured `service.name` and the machine as `host.name`, and the heartbeat reads `sending` to that collector's `host:port`. | Test |
| FR-045-AC-7 | The watcher holds no listening socket, with no endpoint and with one (read from the OS, `/proc/<pid>/net/tcp*`, on Linux). | Test |
| FR-045-AC-8 | The heartbeat carries `otlp` as `disabled`, `ignoring`, `sending` or `failing`, an older record without it, and one whose member cannot be decoded, read as `unknown` without failing the record, and `agent-msg doctor` warns naming the collector on `failing` (with the reason neutralised), and says `otlp: sending to <host:port>` (with the number of series left out when not zero), `disabled` (with the note when ignoring) or `unknown` otherwise. | Test |
| FR-045-AC-9 | The registry refuses a value its kind cannot hold and a negative sum, adds the values of a duplicate sum series and keeps the first of any other, bounds a push at 6,000 points, and sanitises and cuts every attribute value at 128 characters. | Test |
| FR-045-AC-10 | Against a real listener: the request is `POST /v1/metrics` with `Content-Type: application/json`, the configured headers and the right `Content-Length`; a response is read by length, by chunks or by close; a redirect is returned as a status and not followed; garbage is a bad response and an unresolvable host a connect failure, neither a panic; a collector that accepts and never answers, and one that sends a byte at a time, are cut at the request deadline; and a response is read only up to 64 KiB. | Test |
| FR-045-AC-11 | The names, units, kinds and attribute keys a push exports are exactly the catalogue's, and the catalogue is exactly the series table above. | Test |
| FR-045-AC-12 | Leads past 50 are folded into `lead=other` (depth summed, oldest age the largest) and counted as dropped; a lead's depth, oldest age, in-flight entries and dead letters come from its own keys; a hook failure record is read leniently (unknown members ignored, a malformed one skipped), its stage and error kind are neutralised, and at most 64 distinct pairs are kept; every message outcome is exported, at zero when it has not happened. | Test |
| FR-045-AC-13 | Backoff doubles from 1 second to 5 minutes without overflow, including for a wait too large to add to an instant; on an injected clock a snapshot that arrives while backing off is dropped unsent and one at the not-before instant is sent, and a success resets the wait. | Test |
| FR-045-AC-14 | An unusable relay setting is refused with exit 1 from the built binary and a message naming the variable, or the file with the real line, and never a header value or a password: `https://`, a bare host, credentials, a query, a bad port, a header without `=`, a control character, a bad or `%+a` percent escape, a reserved header name, and an interval of 4, 3,601, `abc`, 9223372036854775807 or 18446744073709551615 (from the environment and from the file); 5 and 3,600 are accepted. | Test |
| FR-045-AC-15 | Given a bad OTLP value in the file (`otlp_endpoint = 5`, an https URL, an interval above the ceiling), the file still loads for `agent-msg`: the hook exits 0 and delivers, `send` and `who` work, and `doctor` warns `watcher_otlp_config_invalid` naming the key and line. | Test |
| FR-045-AC-16 | Given inherited generic variables that are unusable (`https://`, protocol `grpc` or `http/protobuf`, a bare host, credentials, malformed headers), the watcher does not exit, sends nothing, logs one line naming the variable and the reason, and the heartbeat reads `ignoring`; given a relay endpoint, the generic variables are not consulted at all and their credentials are never sent to it. | Test |
| FR-045-AC-17 | A `Retry-After` is honoured when longer than the backoff and bounded at 5 minutes, a value too large for 64 bits reads as the largest and so as the bound, and a shorter one does not shorten the backoff; a 4xx other than 429 is not retried and does not back off; a 3xx is not followed and backs off. | Test |
| FR-045-AC-18 | One log line starts an outage and one ends it; a partial success counts the rejected points; a 2xx that rejects every point is a failure with reason `rejected_points` shown in the status, logged once, with no backoff and no `sent` count. | Test |
| FR-045-AC-19 | A collector that never answers holds the exporter thread only: `submit` never waits, the queue holds two snapshots, the oldest are dropped and counted, and the newest are sent when the collector lets go; a panic in the send path is a counted failed push and leaves a working exporter. | Test |
| FR-045-AC-20 | Against a real listener a 500, a 429 with `Retry-After`, garbage and a closed connection each back off, in that order and by those waits, on an injected clock, and a configured header value never appears in a log line or in the status. | Test |
| FR-045-AC-21 | The heartbeat carries the exporter's counters (pushes sent and failed, snapshots dropped, points rejected) when not all zero and they round-trip; a send that has run longer than 30 seconds reads as `failing` with reason `stuck` until it ends, and a send within the limit does not; `doctor` shows the counters. | Test |
| FR-045-AC-22 | At most one push per interval and the first on the first tick; a tick that pushes makes exactly the two ordinary requests of an idle tick plus two pipelined ones (the local leads' queues, the newest hook failures) and no key scan, the next tick inside the interval makes two; a huge interval never overflows time arithmetic; and a failing push read is logged once and leaves the registry cache updated and the tick Ok. | Test |
| FR-045-AC-23 | Given more than 1,000 entries in the hook health stream, the hook gauge counts the newest failures, not the oldest 1,000. | Test |
| FR-045-AC-24 | On an injected resolver and clock: two pushes within 5 minutes make one lookup, one second short of 5 minutes is still cached, and a push at 5 minutes makes another. | Test |
| FR-045-AC-25 | On an injected resolver: a push after every address failed to connect makes another lookup with no time passed; a push whose deadline ended before an address was tried, twice, makes one lookup in all and leaves the cache in place. | Test |
| FR-045-AC-26 | On an injected resolver: a lookup that failed or returned no address is not cached and is repeated on the next push; at the end of the 5 minutes a failed or empty lookup leaves the expired addresses in use, the push succeeds, and the next push looks up again. | Test |
| FR-045-AC-27 | A lookup that takes longer than the whole request budget still lets the push succeed; a resolver that returns 6 addresses leaves 4 in use, in the order returned. | Test |
| FR-045-AC-28 | Given a local lead whose queue overview read returns a Redis error, including one lead with a wrong-type key among otherwise readable leads, the next OTLP push increments only `snapshot=lead_overviews` once per failed lead and preserves the other leads' gauges; a failed `hook_health_newest` read increments only `snapshot=hook_health_newest` once; one whole-read connection error counts once for that read source; malformed entries in a successful hook read do not increment either counter; repeated failed pushes accumulate, a recovered read adds nothing, and both zero-valued points are present before any failure and on every later push. The sum is monotonic from zero for one watcher process and resets on process restart, uses `{failure}`, and carries only the bounded `snapshot` attribute with values `lead_overviews` and `hook_health_newest`; no host, machine, lead, key, error, credential or other high-cardinality value is an attribute. | Test |

## Dependencies

- **Upstream**: [US-006](../usecase/US-006-see-when-the-relay-is-failing.md)
- **Related**: [FR-026](./FR-026-configuration-precedence.md) (the keys),
  [FR-028](./FR-028-watcher-heartbeat.md) and [FR-029](./FR-029-doctor-reports-relay-health.md)
  (the status), [FR-043](./FR-043-moves-and-renames-are-followed-and-recorded.md) (the
  layout read the panes share), [FR-038](./FR-038-watcher-wakes-when-a-message-is-sent.md)
  (the idle tick's two requests), [NFR-002](../non-functional/NFR-002-bounded-resources.md)
  (the bounds), [NFR-003](../non-functional/NFR-003-watcher-keeps-running.md) (the
  watcher survives a bad collector).
- Delivered by AGE-2058; it replaces AGE-2042's pull endpoint.
- Not in this requirement: TLS support, the collector's own configuration and the
  dashboards, which belong to the observability stack, not to this repository.
