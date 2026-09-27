---
id: SR-032
title: "Architecture evaluation of Herdr-backed address resolution"
type: SpecReview
analysis: architecture-evaluation
scope: "Working tree at 2026-09-27; FR-001, FR-002, FR-003, FR-004, FR-005, Herdr place identity and watcher refresh"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay/FR-003"
    type: reviews
---
# SR-032: Architecture evaluation of Herdr-backed address resolution

## Summary

The core direction is sound: Herdr owns workspace, tab and pane identity; the
CLI resolves a human-readable `server:workspace[:tab[:pane]]` against a local
Redis snapshot; and sending refuses multiple matching leads instead of
guessing. This keeps lookups fast and makes the address model harness-neutral.
The implementation is not an arbitrary pile of fallback paths, and the recent
address matching code is small enough to understand.

The model is not yet coherent at the boundary where a relay registration is
attached to a Herdr pane. A pane value inherited from a shared harness process
is still a claim. The watcher can promote that claim into a location when
Herdr lists the pane but provides no session evidence. Separately, the written
cardinality contract and current resolver describe registered leads, while the
requested rule is framed in terms of Herdr tabs and panes. Until those two
contracts agree, the implementation can look correct in covered cases and
still route a broad address more specifically than intended.

**Verdict: do not treat this design as complete yet.** Keep the prefix address
API and Herdr as the location authority. Tighten the claim-to-location proof
and explicitly define whether ambiguity counts registered leads or all panes
in Herdr. Then encode the registration's location evidence as an explicit
state instead of relying on optional fields plus comments.

## Findings

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-001 | high | The watcher treats a live Herdr pane as sufficient proof for an inherited `herdr_pane` claim when the Herdr session id is absent. `pane_has_current_location` returns true if the pane exists and there is no contradictory session pair. The refresh loop then copies that pane's workspace/tab/Herdr IDs onto the registration. A shared Codex daemon can therefore attach the wrong valid pane and make the bad location routable. Keep claimed and verified location distinct; promote only on a positive harness/session-to-pane join or another explicit Herdr proof. When Herdr omits session identity, report the location as unverified unless the harness-specific adapter can prove the association. | `crates/relay-watcher/src/relink.rs:98-113`, `crates/relay-watcher/src/main.rs:438-450`, `crates/agent-msg/src/main.rs:840-857` |
| FND-002 | high | The specificity rule is underspecified and currently counts only registered relay leads. `resolve_target` selects registrations whose cached Herdr location matches, then succeeds whenever that set has one member. It does not know the complete Herdr pane/tab topology. Thus `server:workspace` can resolve when Herdr has multiple tabs but only one registered lead, and `server:workspace:tab` can resolve when the tab has multiple panes but only one registered lead. FR-003-AC-8 explicitly says “registered leads”; the desired rule says one tab/pane may be assumed, while two or more require specificity. Decide which cardinality is normative. If Herdr topology is normative, cache a versioned topology snapshot (or an equivalent complete count/identity view) and fail closed when it is unknown or stale; if registered recipients are normative, state that clearly in the FR and user-facing help. | `crates/relay-core/src/resolve.rs:80-93`, `spec/functional/FR-003-address-matching-ignores-case.md:13-33,45-46` |
| FND-003 | medium | The stored location mixes independent facts and validity in optional fields: human workspace/tab labels, `tabs_in_workspace`, and optional `HerdrPlaceIds` with an optional pane and wall-clock verification time. Legacy and hook-only records are represented by `None`, but that does not say whether the pane is claimed, missing, stale, ambiguous, or verified. This forces consumers to infer state differently; address matching checks `herdr_ids.is_some()`, the watcher uses `herdr_pane` plus session comparison, and display code applies a time-based stale warning. Replace this with a small explicit evidence state (for example `Unplaced`, `Claimed { pane }`, `Verified { place, observed_at }`, `Unverifiable { reason }`) and make resolution accept only `Verified`. Keep compatibility decoding at the Redis boundary, then normalize old records into a named legacy/unverified state. Remove `tabs_in_workspace` if it no longer participates in resolution or display decisions. | `crates/relay-core/src/address.rs:29-60`, `crates/relay-core/src/resolve.rs:81-86`, `crates/relay-watcher/src/relink.rs:102-113` |

## Design assessment

### What is well-shaped

- The address is a prefix query over server, workspace, tab and optional pane;
  matching is pure over a registry snapshot and case-insensitive.
- Herdr IDs back the human labels, and the bus name remains a separate direct
  recipient form. Harness aliases do not need to become part of location
  identity.
- A multi-match result is explicit and includes candidates. That is safer than
  selecting the first registration and keeps ambiguity visible to the sender.
- The cache has a clear purpose: ordinary sends do not perform a Herdr or SSH
  round trip.

### Where complexity is accidental

The current compatibility fields are not yet a measured compatibility layer;
they are the domain model. In particular, `Option<HerdrPlaceIds>` serves as a
proxy for verification, while `herdr_pane` separately serves as a claim. The
comments explain intended provenance, but the type permits the invalid states
that caused this incident. Keep legacy JSON tolerance at deserialization, but
convert it immediately to explicit domain states so routing code cannot mistake
absence of contrary evidence for proof.

`tabs_in_workspace` also looks like leftover cached topology: address selection
does not read it. If no current UI contract depends on it, remove it from the
domain object and wire format in a deliberate schema migration rather than
letting it look authoritative.

### Recommended design boundary

1. Herdr adapter returns a typed location snapshot: stable workspace/tab/pane
   IDs, labels, and the available pane/session evidence, with an observation
   generation or monotonic age.
2. Harness adapters translate their own session quirks into a typed identity
   claim; they do not assert Herdr location by exporting an unchecked pane
   string.
3. A single join/verification function returns an explicit placement state.
   No consumer independently interprets optional fields.
4. The watcher publishes only verified placement and topology data. `find`,
   `who`, `doctor`, metrics and `send` consume the same state and surface the
   same unverified/ambiguous reason.
5. The resolver applies the agreed cardinality rule to a fresh snapshot. It
   resolves exactly one eligible destination and otherwise returns candidate
   Herdr addresses or a clear unknown/stale-topology error.

This keeps the useful fast cache while making the proof boundary testable. The
next tests should cover a claimed pane that exists but belongs to another
session, a sessionless Herdr pane with no independent proof, one registered
lead in a multi-pane Herdr tab, and one registered tab in a multi-tab Herdr
workspace. Property tests should assert that every routed address resolves to
one verified Herdr identity and never to a claim-only record.
