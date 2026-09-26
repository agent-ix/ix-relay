---
id: SR-003
title: "Integrity review of the ix-relay specification"
type: SpecReview
analysis: integrity
scope: "spec/spec.md, spec/stakeholder, spec/usecase, spec/functional, spec/non-functional"
review_set: subset
---
# SR-003: Integrity review of the ix-relay specification

## Summary

Traceability is complete: StR-001 → US-001..003 → FR-001..022; StR-002 →
US-004..005 → FR-023..027; StR-003 → US-006..007 → FR-028..037; every NFR constrains named FRs and those FRs now name
the NFR back (added during this review). Every acceptance criterion has a
verification method. The review found one real inconsistency between a size
bound and its mechanism, one undeclared external-tool dependency, and one
tie-break that was implicit (now stated).

## Findings

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-001 | medium | FR-018-AC-4 and NFR-002 bound the *rendered* hook output, but the mechanism FR-018 prescribes (the budgeted pop) budgets *stored JSON* bytes. A line feed is 2 bytes in JSON and 5 when rendered, and an address label can be longer than a bus name, so the two disagree on newline-heavy batches. The rendered bound is the real obligation (the harness truncates rendered output); recorded as a gap on `main`, now AGE-2031; the watcher's pop, which AGE-2026 moved onto the same stored-byte budget, has the same gap (NFR-002 watcher-prompt row). | FR-018-AC-4, NFR-002, FR-012 |
| FND-002 | medium | herdr is an external CLI every delivery path depends on, and no NFR declares the herdr version relied on, how its absence is detected, or the user-facing error. Behaviour on absence is implied only (FR-013 treats a spawn failure as "no input sent"). Addressed by FR-036 (herdr version and shape check, AGE-2033). | FR-013, FR-014, FR-007, FR-021 |
| FND-003 | low | The `linear` CLI used by `resolve` is likewise undeclared; out of scope per spec.md 2.2 beyond FR-024-AC-5. | FR-024 |
| FND-004 | low | FR-009 did not say which registration wins when two on the same machine carry the local pane id; the code takes the most recently registered. Stated during this review. | FR-009 |
| FND-005 | low | NFRs were referenced from FRs only in one direction; back-references added to FR-008, FR-010, FR-013, FR-014 and FR-018 during this review. | NFR-001, NFR-002, NFR-003, NFR-004 |
| FND-006 | low | Hidden assumption: every participant that can reach Redis is trusted to be a lead (spec.md 3.3). No requirement authenticates a sender, so a message's signature proves only which bus name the sending process claimed. Deliberate, and out of scope, but it bounds what FR-016's anti-forgery guarantee means. | FR-016, spec.md |
| FND-007 | low | FR-006 carries two obligations (what a registration records; names are never both registered and an alias). Both are testable separately (FR-006-AC-1..3 vs FR-006-AC-4). | FR-006 |
| FND-008 | medium | FR-035 (an old address is refused, no grace window) and FR-002 (a bus-name alias keeps resolving after `/rename`) state different policies for addresses and names. That is consistent only if an address names a place and a bus name names a lead; FR-035 says so, but the policy should be confirmed with the operator. | FR-035, FR-002 |
| FND-009 | low | Fixed during review: FR-032 first called a recipient with no watcher on its machine "undeliverable", but a working lead there still receives through its own hooks. FR-032 now distinguishes "undeliverable" (pane replaced) from "not wakeable" (no watcher heartbeat, or no pane). | FR-032, FR-028 |
