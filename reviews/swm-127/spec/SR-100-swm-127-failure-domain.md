---
id: SR-100
title: "Failure-domain analysis - named focus ownership, adoption pins and sources (SWM-127)"
type: SpecReview
analysis: failure-domain
scope: "agent-ix/ix-relay; spec/functional/FR-061-named-focus-definitions.md; spec/functional/FR-062-session-active-focuses-and-adoption.md; spec/functional/FR-063-shared-focus-membership-and-summaries.md; spec/functional/FR-064-named-focus-dashboard-reader.md"
review_set: subset
---

## Summary

Ticket: SWM-127. PR ix-relay#32 at frozen 218a0ff3157d30022689b3c275fc72fd9e503961. Failure-domain analysis of ownership, claim, adoption pins, retention and source failure. Definitions survive session exit; automatic claim is not allowed; replacement sessions inherit nothing; empty never means whole team; last-good with labels is kept. One high finding: nothing releases the pins of dead adopters on a non-archived definition.

## Verdict

**FAIL** - one high finding (pin and adopter-slot leak with no release path), and four medium/low findings on owner-write atomicity, claim evidence, the snapshot source for CLI writes and viewed-focus removal.

## Findings

| ID | Severity | Summary | Refs |
|---|---|---|---|
| FND-001 | high | Pins of expired or replaced adopting sessions are released only by a prune that touches archived definitions, so pins and the 32-adopter slots on a live definition leak permanently and block new adoption | spec/functional/FR-062-session-active-focuses-and-adoption.md:97 |
| FND-002 | medium | The atomic update, archive and transfer compare only the revision or owner epoch the caller sent, not the owner identity verified earlier, so a claim landing between verification and write lets a displaced owner write | spec/functional/FR-061-named-focus-definitions.md:88 |
| FND-003 | medium | Claim without `--force` depends on "positively verified absent or replaced", but the evidence for an owner on another machine, and whether a resumed same-native-session process counts as "replaced", are unspecified | spec/functional/FR-061-named-focus-definitions.md:133 |
| FND-004 | medium | `create` and `update` need a current workspace snapshot but the spec names no source, age bound or timeout for the CLI, so a slow or rate-limited full-team read blocks all definition writes | spec/functional/FR-061-named-focus-definitions.md:107 |
| FND-005 | low | The dashboard `f` selection is not defined when the viewed focus is deactivated, archived, removed or the active set shrinks to zero | spec/functional/FR-064-named-focus-dashboard-reader.md:84 |

## Review Context

Reviewer model claude-sonnet-5-5; harness session ID unavailable; run id unavailable. Linear marker id=SR-100 method=spec-review/failure-domain lang=spec. No AssuranceProfile applies. Read-only apart from evidence artifacts and Linear comments.

## Examined Scope

```yaml
scope:
  - {id: "FR-062", path: "spec/functional/FR-062-session-active-focuses-and-adoption.md", role: examined, excerpt: "Each active adoption registers the session as a pin on the definition's revision\n  ([FR-061](./FR-061-named-focus-definitions.md) retention). A pin is removed by\n  `deactivate`, `set`, `reset`, or by a prune that verifies the session's active\n  record is gone. A failed or ambiguous read keeps the pin."}
  - {id: "FR-061", path: "spec/functional/FR-061-named-focus-definitions.md", role: examined, excerpt: "per `create` when a limit is reached) deletes only an archived definition that\n  has been archived for at least 90 days and has no live adoption."}
  - {id: "FR-061", path: "spec/functional/FR-061-named-focus-definitions.md", role: examined, excerpt: "The relay compares it with the current revision and applies the change in one\n  atomic server-side operation, or refuses with a typed conflict that carries the\n  current revision."}
  - {id: "FR-061", path: "spec/functional/FR-061-named-focus-definitions.md", role: examined, excerpt: "It succeeds without `--force` only when the recorded owner's\n  session is positively verified absent or replaced (the pane hosts a different\n  session, or the owner's registration is gone or replaced in the relay\n  directory)."}
  - {id: "FR-061", path: "spec/functional/FR-061-named-focus-definitions.md", role: examined, excerpt: "A snapshot that is missing, stale, partial or failed SHALL NOT validate a new\n  selection. The command SHALL name the unavailable source and write nothing."}
  - {id: "FR-064", path: "spec/functional/FR-064-named-focus-dashboard-reader.md", role: examined, excerpt: "The key `f` selects the viewed scope: combined (all active focuses), then each\n  focus in order, then combined again. The choice is held only in the dashboard\n  process and is never stored."}
  - {id: "FR-062-AC-10", path: "spec/functional/FR-062-session-active-focuses-and-adoption.md", role: examined, excerpt: "after 30 days without a write the old record is gone and its pin is released by prune only after the absence is verified."}
  - {id: "FR-061-AC-7", path: "spec/functional/FR-061-named-focus-definitions.md", role: examined, excerpt: "Given a live owner, or evidence that cannot establish liveness, the claim is refused unless `--force --reason` is given and the reason is recorded in the history."}
```
