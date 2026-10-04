# Relay directory and harness lifecycle design

Status: 0.3.2 implementation in progress. The Rust directory model and Redis
refinement tests run; real cross-server acceptance and rollout are pending.
Bounded exploration is not an unbounded formal proof.

## Audit evidence

- `relay-ops/src/enrollment.rs::recipient` always gathers Herdr topology,
  requiring a saved SSH profile for remote machines before enqueue.
- `relay-watcher/src/main.rs` enrolls missing local genuine sessions and
  refreshes/invalidate places, but a registration timestamp is not a
  dedicated lease proving the last successful topology observation.
- Hook/watcher delivery uses claims and acknowledgement with redelivery.
  `plugins/opencode/ix-relay.ts` uses destructive `inbox --json`, has a
  process-wide registration latch, and checks whoami before session-created
  evidence is available. Its header also retains obsolete identity claims.

## Authorities and records

Redis is the sender's directory authority. Herdr supplies local evidence to
startup adapters and the owning machine's watcher; direct Herdr querying is
an optional fallback, never a required cross-server transport.

Separate durable session registration/queue identity from leased address
ownership. Registration alone, last activity, and watcher heartbeat do not
prove current address ownership.

An authoritative machine snapshot contains machine, publication epoch,
revision, Redis observation/expiry times, and all verified lead bindings.
A binding contains genuine session ID, canonical address, pane, harness kind,
and delivery capability. Snapshot publication is atomic; a complete empty
snapshot removes all address ownership, without deleting session mail.
An incomplete/failed Herdr read must not publish absence or renew the lease.
Ambiguities are represented explicitly and cannot be hidden by filtering.
Ordered Redis observation tickets are allocated before a topology read.
Publication accepts only the latest allocated ticket, once; a persistent
committed ticket prevents replay even after snapshot expiry. This fences
competing readers and delayed publications without relying on host clocks.
Startup registration writes cannot replace the authoritative directory.

## Directory state machine

Per machine: Unknown -> Fresh -> Expired. A successful fenced complete local
observation transitions any state to Fresh. Time expiry transitions Fresh to
Expired. A failed read preserves the previous expiry, not its freshness.

Per address within a fresh snapshot: Absent, Unique(session), or Ambiguous.
Closed/replaced/renamed sessions change these states only through a newer
atomic snapshot. Durable queues remain attached to their genuine sessions.

Resolution:

1. Expand shortcuts using this caller's verified binding.
2. Read the machine snapshot from Redis.
3. Fresh Unique: atomically validate epoch/revision/lease and enqueue to its
   session queue. Fresh Ambiguous: reject. Fresh Absent: request refresh once.
4. Unknown/Expired: request refresh through Redis for the owning watcher.
5. Wait within one total deadline for a newer successful snapshot and retry
   resolution. Coalesce requests; missing/offline watcher yields a precise
   error, no guessed identity, no enqueue.
6. Optional direct Herdr fallback, when available, must submit verified
   evidence through the same publication/fencing rules; it cannot override
   fresher directory ownership or become required for remote sends.

## Delivery state machine

Queued -> Claimed(lease) -> AcceptedByHarness -> Acknowledged.
Claim expiry -> Queued(redelivery), or DeadLetter after the retry bound.
Harness acknowledgement proves acceptance at the adapter's defined boundary,
not model comprehension. A lead's explicit reply is a separate acknowledgement.
An enqueue timeout has an uncertain outcome; deduplication by send request ID
is required for safe automated retry. Never infer receipt from an empty inbox.

## Harness adapters

| Adapter | Session proof | Startup/resume | Delivery boundary and risks |
|---|---|---|---|
| Claude | Genuine hook session ID matched to local Herdr | SessionStart, reject fork/subagent | Hook context or socket acceptance; socket/version failure returns to existing claim-aware delivery |
| Codex | Genuine hook ID matched to local Herdr | Installed launcher with --no-daemon; resume may produce new ID | Hook context; inherited daemon pane and old registry records never prove identity |
| OpenCode | Confirmed root session ID from API, matched to local Herdr | Session-created/updated and resumed-session lookup | Registration per session, not process; replace destructive pop with claim/ack; ephemeral run exit must leave unaccepted mail recoverable |

Unknown harness capability must not be advertised as tested delivery. OpenCode
must not gate startup registration on a whoami result obtained before genuine
session evidence exists. Multiple root sessions sharing one tab are ambiguous,
not a reason to select the most recently active session silently.

## Safety obligations

S1: Every enqueued recipient belongs to exactly one genuine session in a
fresh accepted snapshot at the atomic enqueue linearization point.
S2: No registry alias, AGENT_MSG_NAME, focus, or inherited pane establishes
address ownership. Preserve AGENT_MSG_NAME unchanged.
S3: Failed/incomplete reads cannot renew ownership or delete queued mail.
S4: Older publications cannot restore a closed, moved, or replaced binding.
S5: A rename/move changes routing but not the session's queue; a new session
in a reused pane never inherits the previous session's queue.
S6: Redis-only sender resolution performs zero Herdr/SSH calls for a fresh
binding, regardless of sender-side machine profiles.
S7: No adapter destroys a message before its documented acceptance boundary.

Proof approach: define pure transitions and bounded exhaustive exploration of
two machines, two sessions, two addresses, two publishers and two messages.
Check S1-S7 after every transition, including expired leases, publication
reordering and enqueue/publication races. This proves the finite model only;
implementation refinement requires Redis atomicity and adapter contract tests.

## Conditional liveness

If Redis, the owning watcher, local Herdr and recipient adapter eventually
remain healthy, fair refresh/sweep/claim processing eventually resolves a
unique existing lead and delivers queued mail. No liveness promise applies
while the watcher cannot observe Herdr or an ephemeral harness has exited.
Bound every foreground operation; do not trade correctness for fabricated
identity on timeout.

## Required evidence before release

- Model exploration with negative mutations demonstrating each invariant can
  fail when its guard is removed; generated transition traces as regressions.
- Pure property tests: move, rename, close, replacement, ambiguity, expiry,
  restart, duplicate watchers, delayed publication, partial topology, skew.
- Real Redis: snapshot atomic replacement and enqueue fencing, wrong key
  types, offline/recovered watcher, request coalescing and deadline bounds.
- One-way network acceptance: Luna has no remote profiles and no outbound
  SSH; fresh and cold Luna -> Terra sends and replies work through Redis.
- Kill/crash between claim, adapter acceptance and ack for each harness;
  record duplicate/loss behavior and distinguish queue, delivery and reply.
- Genuine fresh/resumed startup for Claude, Codex and OpenCode; subagent
  exclusion, reused panes, stale daemon context and two roots in one tab.
- Real cross-server messages with explicit receipt replies; socket and hook
  paths reported separately. Simulated adapters do not prove real receipt.

Implementation order: model and invariants; leased fenced directory store;
local watcher publication/refresh; atomic relay-first resolution/enqueue;
harness adapter repairs; failure/integration/live acceptance; version/tag
and rollout only after evidence is recorded.

## Current evidence and boundaries

The executable directory model is `relay-core/src/directory.rs`. Exhaustive
four-event sequences cover fences, ownership, closure, ambiguity and expiry.
Real Redis tests compare generated lifecycle traces to that model, including
observation starts, failed reads, delayed/replayed publication, rename,
closure, ambiguity, resolution, retained enqueue proofs, and expiry. Tests
also force enqueue/publication races and wrong key types.

The existing `store_model.rs` and `escalation_model.rs` cover store and
operator-decision transitions separately. Harness hooks have real-payload
contract fixtures for Claude and Codex. Directory enrollment now preserves
and tests Claude/Codex/OpenCode kind metadata identically while refusing
missing genuine session associations.

OpenCode's load-time identity gate and at-most-once destructive pop are
pre-existing adapter limitations. They are modeled above but are not repaired
or claimed reliable by the directory/resolver patch. No real OpenCode lead
has been used for this release's acceptance. Socket/hook acceptance and an
explicit lead reply must remain distinct evidence.
