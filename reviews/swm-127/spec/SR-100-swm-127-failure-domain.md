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

## New findings (disposition pass 1)

| ID | Severity | Summary | Refs |
|---|---|---|---|
| FND-006 | low | `claim` is said to carry "the caller's `owner_id`" and be compared with the stored owner record, but a claimer is not the stored owner; the claim command carries only `--expected-owner-epoch`, so the atomic comparison for a claim is undefined | spec/functional/FR-061-named-focus-definitions.md:172-177 |
| FND-007 | low | The deadline-bounded snapshot read is "a further need that root hands to the ix-board owner" without being named an implementation prerequisite of FR-061-AC-20 and FR-063-AC-15, so an implementer cannot tell whether those criteria are blocked | spec/functional/FR-061-named-focus-definitions.md:421-425 |

Found while rechecking the fixes at source head e4931a965ef90d3ccbd3a37bee169ea8c0ebd611. FND-006 follows from the fix to FND-002; FND-007 follows from the fix to FND-004. Both have no outcome yet and go back to the author. Each is stated with its source text in the machine record of the disposition comment.

## Dispositions

Round 1 against source head e4931a965ef90d3ccbd3a37bee169ea8c0ebd611 (fix commits a39c0bd and e4931a9 on the reviewed frozen head 218a0ff3157d30022689b3c275fc72fd9e503961). Original findings above are unchanged. FND-006 and FND-007 are new this round and have no outcome yet.

| FND | outcome | reason |
|---|---|---|
| FND-001 | fixed a39c0bd | Bounded reclamation on non-archived definitions runs on a refused `pin put`, on `update` and on removal examination (FR-061 lines 249-257), using positive evidence only (lines 196-219); unknown pins are kept and at most 32 pins are examined. FR-061-AC-16, FR-062-AC-15 (33 restarts of one session) and FR-062-AC-16 (33 dead incarnations on a non-archived definition) assert it. |
| FND-002 | fixed a39c0bd | `update`, `archive`, `claim` and `transfer` carry the owner ID and epoch and the atomic operation compares both (FR-061 lines 172-177); FR-061-AC-9 asserts the displaced owner is refused. The claim case is not well defined; see FND-006. |
| FND-003 | fixed a39c0bd | One liveness rule (live, absent, replaced, unknown) from the relay registry read and the FR-047 placement state (FR-061 lines 196-219): replaced means a different native session ID, a same-session resume is live, and unknown needs `--force --reason`. FR-061-AC-7 asserts each case. |
| FND-004 | fixed a39c0bd | The snapshot comes only from the shared ix-board read service, at most five minutes old, with a 120 second default deadline, `--read-timeout` 1 to 600, reason `timeout`, exit status 1 and no store write before resolution (FR-061 lines 120-152); FR-061-AC-19 and AC-20 assert it. The upstream deadline support is not named as a prerequisite; see FND-007. |
| FND-005 | fixed a39c0bd | FR-064 lines 105-113 define the viewed focus when it is deactivated, the set is empty, or it is archived, unavailable or removed; FR-064-AC-8 asserts each. |

### Round 2

Round 2 against source head 8b5a68f91373b76a061445cc066e57585e40bea2 (the diff from e4931a9 changes FR-061, FR-062 and FR-063 only). Only findings whose latest outcome was open are rated; FND-001 to FND-005 are rechecked for regression below.

| FND | outcome | reason |
|---|---|---|
| FND-006 | fixed 8b5a68f | FR-061 Ownership (lines 183-200) separates the restricted operations. `update`, `archive` and `transfer` compare the caller's `owner_id` and epoch (`not_owner` for the owner ID, `owner_epoch_conflict` for the epoch). `claim` does not compare the claimer's ID: it carries the observed stored `owner_id` from the validated read plus `--expected-owner-epoch`, compares both with the stored record, and assigns the caller with epoch plus 1, refusing a mismatch with `owner_epoch_conflict` and the current owner and epoch. No new flag is added. FR-061-AC-8 asserts two simultaneous claims (one wins), an observed-owner mismatch with a matching epoch, and that the claimer's own ID is never compared; FR-061-AC-9 asserts a transfer racing a claim yields one new owner. |
| FND-007 | fixed 8b5a68f | FR-061 (lines 137-145 and the Dependencies bullet) makes the bounded shared read an explicit implementation prerequisite of FR-061-AC-19 and AC-20 and of FR-063-AC-15, carried by SWM-178 and by the exact edge ix-board FR-004 (frontmatter line 24; FR-063 frontmatter line 20). It records the owner-verified current state (five-minute cache, 30-second per-request timeout, no overall budget) and the additive extension, and says the criteria are blocked until it exists. The relay defines no SDK API, adds no raw client and starts no detached thread. FR-061-AC-20 and FR-063-AC-15 are marked blocked. The 120 second default and `--read-timeout` 1 to 600 are unchanged. |

Regression check at 8b5a68f: FND-001 (pin reclamation and AC-15 to AC-17, FR-062 AC-15, AC-16) unchanged; FND-002 (displaced owner refused in the atomic operation) is kept and now stated per operation; FND-003 (liveness rule, same-session resume is live, unknown needs force, FR-061-AC-7) is unchanged and the claim bullet still applies the evidence rule; FND-004 (source, five minutes, 120 s default, `--read-timeout`, exit 1, no write before resolution) is unchanged; FND-005 (FR-064) was not changed by the diff. The late FR-004 edge sits on FR-061 and FR-063 only, and FR-061 still declares no edge to FR-062, FR-063 or FR-064, so no cycle is introduced.

## Verdict after disposition pass 1

**CONDITIONAL** for this method. The high finding FND-001 and FND-002 to FND-005 are fixed. Two new low findings (FND-006, FND-007) are open and go back to the author.

## Verdict after disposition pass 2

**PASS** for this method. All seven findings (five original, two new) are fixed and nothing regressed; no new finding. The two criteria that need the bounded shared read are blocked on the upstream extension by explicit statement, which is an implementation prerequisite and not a spec defect.
