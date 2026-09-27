---
id: SR-004
title: "Evidence review of the ix-relay specification"
type: SpecReview
analysis: evidence
scope: "spec/functional, spec/non-functional"
review_set: subset
---
# SR-004: Evidence review of the ix-relay specification

## Summary

`quoin advise` (quoin 0.24.1) over 256 obligations, re-run at AGE-2032 after FR-020 grew to fifteen criteria
and FR-040 was added (the earlier run predated FR-038 and FR-039 as well): 0 inconclusive, 0 uncatalogued, 16 mismatches, all on NFR rows.
The evidence kinds across all recommendations are Property 173, E2E 71, Unit 71, Compile 46, Integration 44, Benchmark 32, Static 20, Eval 13, Fuzz 7, Snapshot 4, Manual 1. The AGE-2029 run counted 214 obligations. Earlier stages added FR-028 to FR-037 (AGE-2033), FR-002-AC-8, FR-005-AC-6,
FR-009-AC-5, FR-015-AC-5, FR-022-AC-6/7 and NFR-003-M-5/6; FR-002-AC-8 was inconclusive as first worded
and was reworded to a concrete outcome. The evidence stage's own results are in the section after the
Findings table. The raw advisor output is in `spec/analysis/quoin-advise*.txt`
and `spec/analysis/quoin-advise.json`, and the full per-obligation table is below.
Everything in the Findings table that departs from the advisor is **judgement**,
labelled as such, not a catalog verdict.

## Findings

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-001 | medium | Mismatch, kept by judgement: NFR-002-M-1..7 and NFR-003-M-1..6 (except M-5, see FND-009) are authored `unit-testing`, `property-based-testing`, `integration-testing` or `fault-injection`; the advisor recommends only `performance-benchmarking`, matched solely on `quantified-threshold`, which every measurement row carries by construction. The rows measure sizes, retention and failure tolerance, not speed, so the authored methods stand. | NFR-002, NFR-003 |
| FND-002 | low | Mismatch, kept by judgement: NFR-004-AC-1, NFR-004-AC-2 and NFR-004-M-1 are authored Inspection; the advisor matched `universal`, `parser` and `quantified-threshold` from the wording. The obligation is about the source's structure (no version fields, no migrations); a static check could later replace inspection. | NFR-004-AC-1, NFR-004-AC-2, NFR-004-M-1 |
| FND-003 | low | Judgement: `agent-behaviour-eval` is recommended for FR-013-AC-2..4, FR-014-AC-1, FR-025-AC-3, NFR-001-M-4, NFR-002-M-3 and NFR-002-M-8, matched on the words "lead"/"agent". The subject in each is deterministic code (watcher, renderer, hook), not an agent's behaviour; reject. | FR-013, FR-014, FR-025, NFR-001, NFR-002 |
| FND-004 | medium | Fuzz is recommended for FR-001-AC-1, FR-001-AC-2 (address parser), FR-021-AC-1 (slugify) and FR-026-AC-6 (config file). Judgement: add fuzz targets for FR-017-AC-1 (hook stdin JSON), FR-008-AC-4 (registry record decoding) and FR-016-AC-1/2 (rendering untrusted text), which take untrusted bytes but were not classified `untrusted-input`. The NFR-004-AC-2 fuzz recommendation is a misfire. | FR-001, FR-021, FR-026, FR-017-AC-1, FR-008-AC-4, FR-016 |
| FND-005 | medium | Model checking is recommended for FR-024-AC-1/2 (escalation status machine) — a genuine fit: a three-state machine with an atomic check-and-set. It is also recommended for FR-003-AC-4, FR-009-AC-2, FR-005-AC-5 and NFR-001-M-5 on a `temporal` match from words like "never" and "while"; judgement: those are property or integration tests. Judgement: the concurrent-pop obligations (FR-010-AC-6, FR-012-AC-4, FR-013-AC-5) also merit a small model of the pop/requeue protocol. | FR-024, FR-012-AC-4, FR-013-AC-5, FR-010-AC-6 |
| FND-006 | low | `deterministic-simulation` and `dynamic-analysis-sanitizer` are recommended for the concurrency criteria (FR-010-AC-6, FR-012-AC-4, FR-013-AC-5, FR-024-AC-3). The atomicity is Redis's (a Lua script or MULTI); judgement: exercise with concurrent clients against a real Redis, and treat the sanitizer as not applicable (no unsafe code). | FR-010-AC-6, FR-012-AC-4, FR-013-AC-5, FR-024-AC-3 |
| FND-007 | low | `compile-time-check` / `design-by-contract` are recommended for FR-004-AC-1, FR-016-AC-1/2, FR-017-AC-1, FR-018-AC-4, FR-022-AC-1, FR-024-AC-1/2 and others on an `invariant` match. Judgement: worth taking for text safety — a text type constructible only through the shared control-character rule would make FR-015, FR-016 and FR-025 hold by construction — and for the `(machine, pane)` type FR-009 needs; elsewhere property tests suffice. | FR-015, FR-016, FR-025, FR-009 |
| FND-008 | low | Authored Verification cells stay at the ISO class (`Test`) for FR criteria; every FR recommendation is class Test, so no FR criterion is a mismatch. The specific method per criterion is the table below; the next stage should write the chosen method into each cell as its evidence lands. | spec/functional |
| FND-009 | low | Mismatch, kept by judgement: NFR-003-M-5 (exit 1 on a malformed configuration) is authored `integration-testing`. The advisor matched `configuration-matrix`, `untrusted-input` and `input-validation` and recommends combinatorial, fuzz and abuse testing. The row checks one start-up path; FR-026 and FR-027 already cover configuration parsing. | NFR-003-M-5 |
| FND-010 | medium | Judgement: the AGE-2033 criteria for FR-028 to FR-030 and FR-032 to FR-036 are mostly single scenarios against Redis and a herdr stub. The advisor labels many of them `universal`, from phrasing such as "every", or `quantified-threshold`, from their timings. The matrix types them Integration. The true properties among them are FR-031-AC-4 (every message removed from an inbox is delivered, requeued or dead-lettered), FR-033-AC-2 (one registration per session id) and FR-037-AC-4 (index and hashes agree). FR-031-AC-4 is also the best model-checking candidate in the new set: a small state model of send, pop, prompt outcome, requeue and prune. | FR-028..FR-037 |
| FND-011 | low | `runtime-monitoring` is recommended for NFR-003-M-2/3, FR-030-AC-3, FR-033-AC-4 and FR-036-AC-3, matched on `temporal`. Judgement: once FR-028's heartbeat exists it is a real monitor for NFR-003, and `agent-msg doctor` (FR-029) is its check. Until then, the fault-injection tests stand. | NFR-003, FR-028, FR-029 |
| FND-012 | low | `agent-behaviour-eval` misfires again, on FR-031-AC-2..4 and FR-036-AC-2, as in FND-003. | FR-031, FR-036 |
| FND-013 | medium | Bug found and fixed by the resolution model (FR-004-AC-6): `classify` compared servers with exact string equality where the address match had folded case, so a machine with a non-ASCII capital (`École`) was reported "unknown server" while also listed as known. Counterexample: registration on `École`, query `école:nope:code`. Fixed in `relay-core/src/resolve.rs`; regression `tc_004_06_*`. | FR-004 |
| FND-014 | medium | Bug found and fixed by the exhaustive state machine (FR-024-AC-2): `resolve` with an empty decision left the escalation `resolved` with no decision (the store treats an empty stored decision as none), so `post-linear` then said "no decision yet". Counterexample: `[Resolve("")]` on a fresh escalation. Fixed with a new `EmptyDecision` error checked before the status, for an empty or whitespace-only decision (review finding 10); FR-024 gained the behaviour bullet. | FR-024 |
| FND-015 | medium | Bug found and fixed by the registry fuzz (FR-008-AC-4): a registry record that is not UTF-8 failed `Relay::agents` as a whole (the hash was read as `String`), hiding every lead. Counterexample: one record `[0x80]`. After review, every registry and alias read (`agents`, `agent`, `refresh_place`, the broadcast field list, `resolve_alias`, `aliases_of`, `all_aliases`) reads bytes and reports or skips what it cannot decode; `tc_008_04_every_registry_and_alias_read_tolerates_bytes_that_are_not_utf8` covers each. | FR-008 |
| FND-016 | low | Bug found and fixed by the precedence property (FR-026-AC-2): `IX_RELAY_MACHINE=` (set but empty) named the machine `""` and was refused, because the default tier read the variable without the empty-is-unset rule. `machine_name` now falls through to the hostname. FR-026-AC-2 is generalised to every setting so the criterion states it. | FR-026 |
| FND-017 | high | Bug found and fixed in this PR after review: a queued inbox entry that is not UTF-8 made `pop_inbox` and `pop_inbox_budget` fail after the server had already removed the batch, losing every message popped with it. Both pops now read the reply as bytes and report an undecodable entry as rejected beside its neighbours; `tc_012_05_*` is un-ignored and the store model plants such entries. | FR-012, FR-020 |
| FND-018 | medium | **Closed by the address-resolution design update.** The recorded counterexample showed that omitting the tab based on cached topology counts makes a printed address fail to round-trip. The design now always includes the tab in the shown address and adds the Herdr pane ID when registered recipients share that tab address; it does not use tab counts to determine identity or uniqueness. Verified by `tc_005_04_every_printed_address_resolves_back_to_its_lead` and `tc_005_04_tab_addresses_resolve_back_to_their_verified_lead`; the implementation is in `AddressBook` and its resolver model. | FR-005 |
| FND-019 | low | Judgement, AGE-2032: `agent-behaviour-eval` is recommended for FR-020-AC-6 (a watcher prompt that fails ambiguously), the same misfire as FND-003: the subject is the watcher's code. Model checking is recommended for FR-020-AC-3 and AC-11 (temporal); the sampled model `inflight_model.rs` stands in for it, with an independent lease monitor and a conservation check read from Redis, and it is not exhaustive. | FR-020 |

## Evidence stage results (AGE-2029)

What was written against the advisor's Property, Fuzz and model-checking recommendations, and how the method
differs from the recommendation. `spec/tests.md` names each test per row.

- **Property** rows on pure or store-seam code are `proptest` tests, several against an independent reference
  model written from the requirement text (address resolution, the store's queue and registry protocol,
  configuration precedence). Store-backed ones run against a real Redis and skip without `IX_RELAY_TEST_URL`.
- **Fuzz** rows (FR-001-AC-1/2, FR-021-AC-1, FR-026-AC-6) and **Property** rows that also got a fuzz lane
  (FR-017-AC-1, FR-008-AC-4, FR-016-AC-1/2) are covered by property-based fuzz lanes, not coverage-guided:
  `cargo-fuzz` needs nightly, which is not pinned here. The matrix types the first group Fuzz and the second
  Property. Each has an always-run seed corpus and an ignored `fuzz_*` lane (`make fuzz`, `fuzz/README.md`).
  A coverage-guided run has not been recorded, so these obligations are written, not discharged as Fuzz.
- **Model checking** rows (FR-024-AC-1/2, the FR-014 typing decision, the NFR-003-M-3 backoff arithmetic, the shared
  unsafe-character predicate) are **exhaustive over a bound**, run as ordinary tests: every scalar value for the
  predicate; every event sequence to depth five for the escalation machine; 14,580 combinations for the typing
  decision (both `get` calls included); every failure count to 100,000 for the backoff. The FR-012/FR-013 pop and
  requeue protocol is **sampled**, not exhaustive: a `proptest` model of the real store over generated operation
  sequences (its two consumers run one after the other, so it is not concurrency evidence; FR-012-AC-4 cites the
  existing concurrent test).
  These are proofs over the stated bound. Kani is not installed. It would prove unboundedly: the escalation
  machine for every sequence, the backoff closed form for every `u32`, `with_suffix`'s length invariant for every
  base and suffix, `Place::address`'s tab rule, and the address grammar's accept/refuse split. The unsafe
  predicate and the typing decision need no Kani (a finite domain, and only sequence-number equality matters).
- **Not written**: FR-028 to FR-037 and the AGE-2032/AGE-2033 remainder (unbuilt, red); rows that need a
  fault-injecting Redis or a herdr stub beyond the existing ones (FR-010-AC-5, FR-013-AC-4/5, the NFR benchmarks);
  FR-006-AC-1..3, FR-007, FR-009-AC-1..3/5, FR-018, FR-019, FR-027 (hook and watcher process behaviour); FR-022-AC-5
  (FR-022-AC-6/7 are written by #16 and by `relay-ops/tests/collision.rs`).

## Recommendations by obligation

Generated from `spec/analysis/quoin-advise.json`: method (evidence kind; matched
characteristic or property shape).

| Obligation | Authored | Recommended | Flag |
|---|---|---|---|
| FR-001-AC-1 | Test | fuzzing (Fuzz; parser)<br>property-based-testing (Property; universal) |  |
| FR-001-AC-2 | Test | fuzzing (Fuzz; parser)<br>property-based-testing (Property; universal) |  |
| FR-001-AC-3 | Test | property-based-testing (Property; universal) |  |
| FR-001-AC-4 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-001-AC-5 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-002-AC-1 | Test | property-based-testing (Property; universal) |  |
| FR-002-AC-2 | Test | property-based-testing (Property; universal) |  |
| FR-002-AC-3 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-002-AC-4 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-002-AC-5 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-002-AC-6 | Test | property-based-testing (Property; universal) |  |
| FR-002-AC-7 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-002-AC-8 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-003-AC-1 | Test | property-based-testing (Property; universal) |  |
| FR-003-AC-2 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-003-AC-3 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-003-AC-4 | Test | compile-time-check (Compile; invariant)<br>design-by-contract (Compile; invariant)<br>model-checking (Static; temporal)<br>property-based-testing (Property; invariant)<br>runtime-monitoring (Integration; temporal) |  |
| FR-003-AC-5 | Test | property-based-testing (Property; universal) |  |
| FR-003-AC-6 | Test | property-based-testing (Property; universal) |  |
| FR-004-AC-1 | Test | compile-time-check (Compile; invariant)<br>design-by-contract (Compile; invariant)<br>property-based-testing (Property; invariant) |  |
| FR-004-AC-2 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-004-AC-3 | Test | metamorphic-testing (Property; ordering)<br>property-based-testing (Property; ordering) |  |
| FR-004-AC-4 | Test | property-based-testing (Property; universal) |  |
| FR-004-AC-5 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-004-AC-6 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-004-AC-7 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-004-AC-8 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-004-AC-9 | Test | property-based-testing (Property; universal) |  |
| FR-005-AC-1 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-005-AC-2 | Test | property-based-testing (Property; universal) |  |
| FR-005-AC-3 | Test | golden-approval-testing (Snapshot; stable-output)<br>property-based-testing (Property; universal) |  |
| FR-005-AC-4 | Test | golden-approval-testing (Snapshot; stable-output)<br>property-based-testing (Property; universal) |  |
| FR-005-AC-5 | Test | model-checking (Static; temporal)<br>property-based-testing (Property; universal)<br>runtime-monitoring (Integration; temporal) |  |
| FR-005-AC-6 | Test | property-based-testing (Property; universal) |  |
| FR-005-AC-7 | Test | property-based-testing (Property; universal) |  |
| FR-005-AC-8 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-005-AC-9 | Test | bdd-spec-by-example (E2E; example)<br>golden-approval-testing (Snapshot; stable-output)<br>performance-benchmarking (Benchmark; quantified-threshold)<br>unit-testing (Unit; example) |  |
| FR-006-AC-1 | Test | property-based-testing (Property; universal) |  |
| FR-006-AC-2 | Test | property-based-testing (Property; universal) |  |
| FR-006-AC-3 | Test | property-based-testing (Property; universal) |  |
| FR-006-AC-4 | Test | property-based-testing (Property; universal) |  |
| FR-006-AC-5 | Test | property-based-testing (Property; universal) |  |
| FR-007-AC-1 | Test | property-based-testing (Property; universal) |  |
| FR-007-AC-2 | Test | property-based-testing (Property; universal) |  |
| FR-007-AC-3 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-007-AC-4 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-007-AC-5 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-008-AC-1 | Test | property-based-testing (Property; universal) |  |
| FR-008-AC-2 | Test | property-based-testing (Property; universal) |  |
| FR-008-AC-3 | Test | property-based-testing (Property; universal) |  |
| FR-008-AC-4 | Test | bdd-spec-by-example (E2E; example)<br>fuzzing (Fuzz; parser)<br>unit-testing (Unit; example) |  |
| FR-008-AC-5 | Test | property-based-testing (Property; universal) |  |
| FR-009-AC-1 | Test | property-based-testing (Property; universal) |  |
| FR-009-AC-2 | Test | compile-time-check (Compile; invariant)<br>design-by-contract (Compile; invariant)<br>model-checking (Static; temporal)<br>property-based-testing (Property; invariant)<br>runtime-monitoring (Integration; temporal) |  |
| FR-009-AC-3 | Test | property-based-testing (Property; universal) |  |
| FR-009-AC-4 | Test | property-based-testing (Property; universal) |  |
| FR-009-AC-5 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-010-AC-1 | Test | property-based-testing (Property; universal) |  |
| FR-010-AC-2 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-010-AC-3 | Test | property-based-testing (Property; universal) |  |
| FR-010-AC-4 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-010-AC-5 | Test | property-based-testing (Property; universal) |  |
| FR-010-AC-6 | Test | deterministic-simulation (Integration; concurrent, concurrency)<br>dynamic-analysis-sanitizer (Integration; concurrent, concurrency) |  |
| FR-010-AC-7 | Test | compile-time-check (Compile; invariant)<br>design-by-contract (Compile; invariant)<br>model-checking (Static; temporal)<br>property-based-testing (Property; invariant)<br>runtime-monitoring (Integration; temporal) |  |
| FR-011-AC-1 | Test | property-based-testing (Property; universal) |  |
| FR-011-AC-2 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-011-AC-3 | Test | property-based-testing (Property; universal) |  |
| FR-012-AC-1 | Test | property-based-testing (Property; universal) |  |
| FR-012-AC-2 | Test | property-based-testing (Property; universal) |  |
| FR-012-AC-3 | Test | property-based-testing (Property; universal) |  |
| FR-012-AC-4 | Test | deterministic-simulation (Integration; concurrent)<br>dynamic-analysis-sanitizer (Integration; concurrent)<br>model-checking (Static; temporal)<br>property-based-testing (Property; universal)<br>runtime-monitoring (Integration; temporal) |  |
| FR-012-AC-5 | Test | property-based-testing (Property; universal) |  |
| FR-013-AC-1 | Test | property-based-testing (Property; universal) |  |
| FR-013-AC-2 | Test | agent-behaviour-eval (Eval; agent-behaviour)<br>bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-013-AC-3 | Test | agent-behaviour-eval (Eval; agent-behaviour)<br>bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-013-AC-4 | Test | agent-behaviour-eval (Eval; agent-behaviour)<br>property-based-testing (Property; universal) |  |
| FR-013-AC-5 | Test | deterministic-simulation (Integration; concurrent)<br>dynamic-analysis-sanitizer (Integration; concurrent)<br>property-based-testing (Property; universal) |  |
| FR-013-AC-6 | Test | property-based-testing (Property; universal) |  |
| FR-014-AC-1 | Test | agent-behaviour-eval (Eval; agent-behaviour)<br>bdd-spec-by-example (E2E; example)<br>combinatorial-tway (Property; configuration-matrix)<br>design-by-contract (Compile; precondition-bearing)<br>unit-testing (Unit; example) |  |
| FR-014-AC-2 | Test | property-based-testing (Property; universal) |  |
| FR-014-AC-3 | Test | property-based-testing (Property; universal) |  |
| FR-014-AC-4 | Test | property-based-testing (Property; universal) |  |
| FR-015-AC-1 | Test | property-based-testing (Property; universal) |  |
| FR-015-AC-2 | Test | property-based-testing (Property; universal) |  |
| FR-015-AC-3 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-015-AC-4 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-015-AC-5 | Test | property-based-testing (Property; universal) |  |
| FR-016-AC-1 | Test | compile-time-check (Compile; invariant)<br>design-by-contract (Compile; invariant)<br>property-based-testing (Property; invariant) |  |
| FR-016-AC-2 | Test | compile-time-check (Compile; invariant)<br>design-by-contract (Compile; invariant)<br>property-based-testing (Property; invariant) |  |
| FR-016-AC-3 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-016-AC-4 | Test | property-based-testing (Property; universal) |  |
| FR-016-AC-5 | Test | property-based-testing (Property; universal) |  |
| FR-017-AC-1 | Test | compile-time-check (Compile; invariant)<br>design-by-contract (Compile; invariant)<br>integration-testing (Integration; io-boundary)<br>property-based-testing (Property; invariant) |  |
| FR-017-AC-2 | Test | property-based-testing (Property; universal) |  |
| FR-017-AC-3 | Test | integration-testing (Integration; io-boundary)<br>property-based-testing (Property; universal) |  |
| FR-017-AC-4 | Test | property-based-testing (Property; universal) |  |
| FR-017-AC-5 | Test | integration-testing (Integration; io-boundary)<br>property-based-testing (Property; universal) |  |
| FR-017-AC-6 | Test | combinatorial-tway (Property; configuration-matrix)<br>property-based-testing (Property; universal) |  |
| FR-017-AC-7 | Test | integration-testing (Integration; io-boundary)<br>property-based-testing (Property; universal) |  |
| FR-018-AC-1 | Test | bdd-spec-by-example (E2E; example)<br>integration-testing (Integration; io-boundary)<br>unit-testing (Unit; example) |  |
| FR-018-AC-2 | Test | bdd-spec-by-example (E2E; example)<br>integration-testing (Integration; io-boundary)<br>unit-testing (Unit; example) |  |
| FR-018-AC-3 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-018-AC-4 | Test | compile-time-check (Compile; invariant)<br>design-by-contract (Compile; invariant)<br>property-based-testing (Property; invariant) |  |
| FR-018-AC-5 | Test | model-checking (Static; temporal)<br>property-based-testing (Property; universal)<br>runtime-monitoring (Integration; temporal) |  |
| FR-018-AC-6 | Test | property-based-testing (Property; universal) |  |
| FR-018-AC-7 | Test | property-based-testing (Property; universal) |  |
| FR-019-AC-1 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-019-AC-2 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-019-AC-3 | Test | property-based-testing (Property; universal) |  |
| FR-019-AC-4 | Test | property-based-testing (Property; universal) |  |
| FR-020-AC-1 | Test | bdd-spec-by-example (E2E; example)<br>integration-testing (Integration; io-boundary)<br>unit-testing (Unit; example) |  |
| FR-020-AC-2 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-020-AC-3 | Test | model-checking (Static; temporal)<br>property-based-testing (Property; universal)<br>runtime-monitoring (Integration; temporal) |  |
| FR-020-AC-4 | Test | property-based-testing (Property; universal) |  |
| FR-020-AC-5 | Test | property-based-testing (Property; universal) |  |
| FR-020-AC-6 | Test | agent-behaviour-eval (Eval; agent-behaviour)<br>property-based-testing (Property; universal) |  |
| FR-020-AC-7 | Test | compile-time-check (Compile; invariant)<br>design-by-contract (Compile; invariant)<br>property-based-testing (Property; invariant) |  |
| FR-020-AC-8 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-020-AC-9 | Test | property-based-testing (Property; universal) |  |
| FR-020-AC-10 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-020-AC-11 | Test | property-based-testing (Property; universal) |  |
| FR-020-AC-12 | Test | property-based-testing (Property; universal) |  |
| FR-020-AC-13 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-020-AC-14 | Test | property-based-testing (Property; universal) |  |
| FR-020-AC-15 | Test | metamorphic-testing (Property; round-trip)<br>performance-benchmarking (Benchmark; quantified-threshold)<br>property-based-testing (Property; round-trip) |  |
| FR-021-AC-1 | Test | fuzzing (Fuzz; parser)<br>property-based-testing (Property; universal) |  |
| FR-021-AC-2 | Test | property-based-testing (Property; universal) |  |
| FR-021-AC-3 | Test | bdd-spec-by-example (E2E; example)<br>inspection (Manual; no-executable-oracle)<br>unit-testing (Unit; example) |  |
| FR-021-AC-4 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-021-AC-5 | Test | property-based-testing (Property; universal) |  |
| FR-022-AC-1 | Test | compile-time-check (Compile; invariant)<br>design-by-contract (Compile; invariant)<br>property-based-testing (Property; invariant) |  |
| FR-022-AC-2 | Test | property-based-testing (Property; universal) |  |
| FR-022-AC-3 | Test | property-based-testing (Property; universal) |  |
| FR-022-AC-4 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-022-AC-5 | Test | property-based-testing (Property; universal) |  |
| FR-022-AC-6 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-022-AC-7 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-022-AC-8 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-022-AC-9 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-023-AC-1 | Test | property-based-testing (Property; universal) |  |
| FR-023-AC-2 | Test | property-based-testing (Property; universal) |  |
| FR-023-AC-3 | Test | property-based-testing (Property; universal) |  |
| FR-023-AC-4 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-024-AC-1 | Test | compile-time-check (Compile; invariant)<br>design-by-contract (Compile; invariant)<br>model-checking (Static; temporal)<br>property-based-testing (Property; invariant)<br>runtime-monitoring (Integration; temporal) |  |
| FR-024-AC-2 | Test | compile-time-check (Compile; invariant)<br>design-by-contract (Compile; invariant)<br>model-checking (Static; temporal)<br>property-based-testing (Property; invariant)<br>runtime-monitoring (Integration; temporal) |  |
| FR-024-AC-3 | Test | bdd-spec-by-example (E2E; example)<br>deterministic-simulation (Integration; concurrent)<br>dynamic-analysis-sanitizer (Integration; concurrent)<br>unit-testing (Unit; example) |  |
| FR-024-AC-4 | Test | property-based-testing (Property; universal) |  |
| FR-024-AC-5 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-024-AC-6 | Test | property-based-testing (Property; universal) |  |
| FR-025-AC-1 | Test | property-based-testing (Property; universal) |  |
| FR-025-AC-2 | Test | property-based-testing (Property; universal) |  |
| FR-025-AC-3 | Test | agent-behaviour-eval (Eval; agent-behaviour)<br>compile-time-check (Compile; invariant)<br>design-by-contract (Compile; invariant)<br>property-based-testing (Property; invariant) |  |
| FR-025-AC-4 | Test | property-based-testing (Property; universal) |  |
| FR-026-AC-1 | Test | bdd-spec-by-example (E2E; example)<br>combinatorial-tway (Property; configuration-matrix)<br>unit-testing (Unit; example) |  |
| FR-026-AC-2 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-026-AC-3 | Test | property-based-testing (Property; universal) |  |
| FR-026-AC-4 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-026-AC-5 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-026-AC-6 | Test | fuzzing (Fuzz; untrusted-input)<br>negative-abuse-testing (Integration; input-validation)<br>property-based-testing (Property; universal) |  |
| FR-027-AC-1 | Test | property-based-testing (Property; universal) |  |
| FR-027-AC-2 | Test | bdd-spec-by-example (E2E; example)<br>integration-testing (Integration; io-boundary)<br>unit-testing (Unit; example) |  |
| FR-027-AC-3 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-027-AC-4 | Test | property-based-testing (Property; universal) |  |
| FR-027-AC-5 | Test | combinatorial-tway (Property; configuration-matrix)<br>property-based-testing (Property; universal) |  |
| FR-028-AC-1 | Test | performance-benchmarking (Benchmark; quantified-threshold)<br>property-based-testing (Property; universal) |  |
| FR-028-AC-2 | Test | bdd-spec-by-example (E2E; example)<br>performance-benchmarking (Benchmark; quantified-threshold)<br>unit-testing (Unit; example) |  |
| FR-028-AC-3 | Test | metamorphic-testing (Property; idempotence)<br>performance-benchmarking (Benchmark; quantified-threshold)<br>property-based-testing (Property; idempotence) |  |
| FR-028-AC-4 | Test | performance-benchmarking (Benchmark; quantified-threshold)<br>property-based-testing (Property; universal) |  |
| FR-029-AC-1 | Test | property-based-testing (Property; universal) |  |
| FR-029-AC-2 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-029-AC-3 | Test | performance-benchmarking (Benchmark; latency, quantified-threshold)<br>bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-029-AC-4 | Test | property-based-testing (Property; universal) |  |
| FR-029-AC-5 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-030-AC-1 | Test | bdd-spec-by-example (E2E; example)<br>integration-testing (Integration; io-boundary)<br>unit-testing (Unit; example) |  |
| FR-030-AC-2 | Test | metamorphic-testing (Property; idempotence)<br>property-based-testing (Property; idempotence) |  |
| FR-030-AC-3 | Test | compile-time-check (Compile; invariant)<br>design-by-contract (Compile; invariant)<br>model-checking (Static; temporal)<br>property-based-testing (Property; invariant)<br>runtime-monitoring (Integration; temporal) |  |
| FR-030-AC-4 | Test | property-based-testing (Property; universal) |  |
| FR-030-AC-5 | Test | compile-time-check (Compile; invariant)<br>design-by-contract (Compile; invariant)<br>property-based-testing (Property; invariant) |  |
| FR-030-AC-6 | Test | property-based-testing (Property; universal) |  |
| FR-030-AC-7 | Test | property-based-testing (Property; universal) |  |
| FR-030-AC-8 | Test | property-based-testing (Property; universal) |  |
| FR-030-AC-9 | Test | property-based-testing (Property; universal) |  |
| FR-031-AC-1 | Test | property-based-testing (Property; universal) |  |
| FR-031-AC-2 | Test | agent-behaviour-eval (Eval; agent-behaviour)<br>bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-031-AC-3 | Test | agent-behaviour-eval (Eval; agent-behaviour)<br>bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-031-AC-4 | Test | agent-behaviour-eval (Eval; agent-behaviour)<br>compile-time-check (Compile; invariant)<br>design-by-contract (Compile; invariant)<br>integration-testing (Integration; io-boundary)<br>property-based-testing (Property; invariant) |  |
| FR-031-AC-5 | Test | property-based-testing (Property; universal) |  |
| FR-032-AC-1 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-032-AC-2 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-032-AC-3 | Test | property-based-testing (Property; universal) |  |
| FR-032-AC-4 | Test | property-based-testing (Property; universal) |  |
| FR-033-AC-1 | Test | performance-benchmarking (Benchmark; latency, quantified-threshold)<br>property-based-testing (Property; universal) |  |
| FR-033-AC-2 | Test | property-based-testing (Property; universal) |  |
| FR-033-AC-3 | Test | design-by-contract (Compile; precondition-bearing)<br>property-based-testing (Property; universal) |  |
| FR-033-AC-4 | Test | compile-time-check (Compile; invariant)<br>design-by-contract (Compile; invariant)<br>model-checking (Static; temporal)<br>property-based-testing (Property; invariant)<br>runtime-monitoring (Integration; temporal) |  |
| FR-034-AC-1 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-034-AC-2 | Test | property-based-testing (Property; universal) |  |
| FR-034-AC-3 | Test | property-based-testing (Property; universal) |  |
| FR-034-AC-4 | Test | property-based-testing (Property; universal) |  |
| FR-035-AC-1 | Test | property-based-testing (Property; universal) |  |
| FR-035-AC-2 | Test | property-based-testing (Property; universal) |  |
| FR-035-AC-3 | Test | property-based-testing (Property; universal) |  |
| FR-035-AC-4 | Test | property-based-testing (Property; universal) |  |
| FR-036-AC-1 | Test | property-based-testing (Property; universal) |  |
| FR-036-AC-2 | Test | agent-behaviour-eval (Eval; agent-behaviour)<br>property-based-testing (Property; universal) |  |
| FR-036-AC-3 | Test | bdd-spec-by-example (E2E; example)<br>model-checking (Static; temporal)<br>runtime-monitoring (Integration; temporal)<br>unit-testing (Unit; example) |  |
| FR-036-AC-4 | Test | property-based-testing (Property; universal) |  |
| FR-037-AC-1 | Test | property-based-testing (Property; universal) |  |
| FR-037-AC-2 | Test | property-based-testing (Property; universal) |  |
| FR-037-AC-3 | Test | property-based-testing (Property; universal) |  |
| FR-037-AC-4 | Test | property-based-testing (Property; universal) |  |
| FR-038-AC-1 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-038-AC-2 | Test | property-based-testing (Property; universal) |  |
| FR-038-AC-3 | Test | fault-injection (Integration; reliability)<br>performance-benchmarking (Benchmark; quantified-threshold)<br>property-based-testing (Property; universal) |  |
| FR-038-AC-4 | Test | fault-injection (Integration; reliability)<br>property-based-testing (Property; universal) |  |
| FR-038-AC-5 | Test | property-based-testing (Property; universal) |  |
| FR-038-AC-6 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-038-AC-7 | Test | property-based-testing (Property; universal) |  |
| FR-038-AC-8 | Test | property-based-testing (Property; universal) |  |
| FR-038-AC-9 | Test | property-based-testing (Property; universal) |  |
| FR-038-AC-10 | Test | fault-injection (Integration; reliability)<br>property-based-testing (Property; universal) |  |
| FR-038-AC-11 | Test | property-based-testing (Property; universal) |  |
| FR-038-AC-12 | Test | property-based-testing (Property; universal) |  |
| FR-039-AC-1 | Test | metamorphic-testing (Property; idempotence)<br>performance-benchmarking (Benchmark; quantified-threshold)<br>property-based-testing (Property; idempotence) |  |
| FR-039-AC-2 | Test | compile-time-check (Compile; invariant)<br>design-by-contract (Compile; invariant)<br>model-checking (Static; temporal)<br>property-based-testing (Property; invariant)<br>runtime-monitoring (Integration; temporal) |  |
| FR-039-AC-3 | Test | property-based-testing (Property; universal) |  |
| FR-039-AC-4 | Test | performance-benchmarking (Benchmark; latency, quantified-threshold)<br>compile-time-check (Compile; invariant)<br>design-by-contract (Compile; invariant)<br>model-checking (Static; temporal)<br>property-based-testing (Property; invariant)<br>runtime-monitoring (Integration; temporal) |  |
| FR-040-AC-1 | Test | compile-time-check (Compile; invariant)<br>design-by-contract (Compile; invariant)<br>model-checking (Static; temporal)<br>property-based-testing (Property; invariant)<br>runtime-monitoring (Integration; temporal) |  |
| FR-040-AC-2 | Test | compile-time-check (Compile; invariant)<br>design-by-contract (Compile; invariant)<br>model-checking (Static; temporal)<br>property-based-testing (Property; invariant)<br>runtime-monitoring (Integration; temporal) |  |
| FR-040-AC-3 | Test | property-based-testing (Property; universal) |  |
| FR-040-AC-4 | Test | compile-time-check (Compile; invariant)<br>design-by-contract (Compile; invariant)<br>model-checking (Static; temporal)<br>property-based-testing (Property; invariant)<br>runtime-monitoring (Integration; temporal) |  |
| FR-040-AC-5 | Test | bdd-spec-by-example (E2E; example)<br>unit-testing (Unit; example) |  |
| FR-040-AC-6 | Test | golden-approval-testing (Snapshot; stable-output)<br>property-based-testing (Property; universal) |  |
| NFR-004-AC-1 | Inspection | property-based-testing (Property; universal) | mismatch |
| NFR-004-AC-2 | Inspection | fuzzing (Fuzz; parser)<br>property-based-testing (Property; universal) | mismatch |
| NFR-001-M-1 | performance-benchmarking | performance-benchmarking (Benchmark; quantified-threshold) |  |
| NFR-001-M-2 | performance-benchmarking | performance-benchmarking (Benchmark; quantified-threshold) |  |
| NFR-001-M-3 | performance-benchmarking | performance-benchmarking (Benchmark; quantified-threshold) |  |
| NFR-001-M-4 | performance-benchmarking | agent-behaviour-eval (Eval; agent-behaviour)<br>performance-benchmarking (Benchmark; quantified-threshold) |  |
| NFR-001-M-5 | performance-benchmarking | integration-testing (Integration; io-boundary)<br>model-checking (Static; temporal)<br>performance-benchmarking (Benchmark; quantified-threshold)<br>runtime-monitoring (Integration; temporal) |  |
| NFR-001-M-6 | performance-benchmarking | performance-benchmarking (Benchmark; quantified-threshold) |  |
| NFR-002-M-1 | unit-testing | performance-benchmarking (Benchmark; quantified-threshold) | mismatch |
| NFR-002-M-2 | property-based-testing | performance-benchmarking (Benchmark; quantified-threshold) | mismatch |
| NFR-002-M-3 | property-based-testing | agent-behaviour-eval (Eval; agent-behaviour)<br>performance-benchmarking (Benchmark; quantified-threshold) | mismatch |
| NFR-002-M-4 | integration-testing | performance-benchmarking (Benchmark; quantified-threshold) | mismatch |
| NFR-002-M-5 | integration-testing | performance-benchmarking (Benchmark; quantified-threshold) | mismatch |
| NFR-002-M-6 | fault-injection | performance-benchmarking (Benchmark; quantified-threshold) | mismatch |
| NFR-002-M-7 | fault-injection | performance-benchmarking (Benchmark; quantified-threshold) | mismatch |
| NFR-002-M-8 | performance-benchmarking | agent-behaviour-eval (Eval; agent-behaviour)<br>performance-benchmarking (Benchmark; quantified-threshold) |  |
| NFR-003-M-1 | fault-injection | performance-benchmarking (Benchmark; quantified-threshold) | mismatch |
| NFR-003-M-2 | fault-injection | model-checking (Static; temporal)<br>performance-benchmarking (Benchmark; quantified-threshold)<br>runtime-monitoring (Integration; temporal) | mismatch |
| NFR-003-M-3 | fault-injection | model-checking (Static; temporal)<br>performance-benchmarking (Benchmark; quantified-threshold)<br>runtime-monitoring (Integration; temporal) | mismatch |
| NFR-003-M-4 | fault-injection | performance-benchmarking (Benchmark; quantified-threshold) | mismatch |
| NFR-003-M-5 | integration-testing | combinatorial-tway (Property; configuration-matrix)<br>fuzzing (Fuzz; untrusted-input)<br>negative-abuse-testing (Integration; input-validation)<br>performance-benchmarking (Benchmark; quantified-threshold) | mismatch |
| NFR-003-M-6 | unit-testing | performance-benchmarking (Benchmark; quantified-threshold) | mismatch |
| NFR-004-M-1 | inspection | performance-benchmarking (Benchmark; quantified-threshold) | mismatch |
