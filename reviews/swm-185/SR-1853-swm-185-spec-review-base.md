---
id: SR-1853
title: "Spec review (base, bounded to the added paragraph) - FR-048 typed quota hold paragraph (SWM-185)"
type: SpecReview
analysis: base
scope: "agent-ix/ix-relay; spec/functional/FR-048-session-work-monitor.md (the paragraph added at line 43 only)"
review_set: base
relationships:
  - { target: "ix://agent-ix/ix-relay/FR-048", type: references }
---

## Summary

Ticket: SWM-185. PR pending. Frozen source HEAD c97b4dd against base d5fa1c3. Base review bounded to the one paragraph the change adds to FR-048 at line 43, the paragraph that extends FR-048-AC-5. The whole of FR-048, the other requirements, the wave analyses (failure-domain, integrity, dependency, evidence, risk-complexity, scope-boundary), entity and relationship analyses and the Jevs sweeps were not run, by instruction. The paragraph adds no id, no relationship, no hash, digest, pin, manifest or tracking record, and no `Verification` cell, so the ID, link and antipattern checks of the checklist hold for it. It has three base defects, all medium: three typed terms that the paragraph uses for different things and that disagree, the undefined `hold to wait out` on which the coalescing rule turns, and a reporting obligation that applies only when a snapshot is kept.

## Verdict

**CONDITIONAL** - no `high` finding (3 medium). A reading of the added paragraph against the checklist and the code it describes; I ran no Quire command, no matrix and no Vale. The parent reports FR-048 Quire validate exit 0 and diff check exit 0 at this source; that is a structural result and not a reading of the paragraph.

## Findings

| ID | Severity | Summary | Refs |
|---|---|---|---|
| FND-001 | medium | The paragraph labels the service's typed rate-limit error as rate-limited and says a refresh whose typed outcome is absent stays unknown, but the SDK reports that very error with an absent (`Unknown`) outcome, and `typed state`, `typed outcome`, `typed rate-limit error` and `untyped refresh` are never defined | FR-048 |
| FND-002 | medium | `only for a hold to wait out` is undefined and only local allocation and server reserve are said to follow the rate-limit timer, so the paragraph does not say whether evidence unavailable, quarantined, request not admissible, a busy refresh lock or the legacy local budget count down or coalesce | FR-048 |
| FND-003 | medium | The sentence that reports every blocker and the earliest reevaluation is conditioned on keeping the last snapshot, so a hold with no stored snapshot has a label obligation and no stated reporting obligation | FR-048 |

## Review Context

Reviewer model claude-sonnet-5-5; the harness run id is not visible and none is claimed. Repository agent-ix/ix-relay (public). What was read: the installed quoin spec-review skill and its checklist (quoin 0.29.4 is the version visible to me; the brief names 0.31.1), FR-048 in the worktree, the code and SDK facts of SR-1851 for the behavior the paragraph describes, and the SWM-163 base and EARS comment bodies as format precedent. Examined: the five sentences of the added paragraph against ID format (none added), user-visible terminology consistency, verifiable behavior, error and edge conditions, and the hash, digest and pin antipattern (no instruction creates or checks one). Not run: any command. Assurance profile: none exists under spec/, so none applies.

## Examined Scope

```yaml
scope:
  - {id: "FR-048", path: "spec/functional/FR-048-session-work-monitor.md", role: examined, excerpt: 'A Linear read that the shared ix-board quota protection holds back SHALL be labeled by the typed state the service reports for that attempt: local allocation, server reserve, server backoff, quota evidence unavailable (including quarantined legacy reservations) or request not admissible. Only a server backoff, or the service''s typed rate-limit error, is labeled rate-limited. Local allocation and server reserve holds follow the same timer and coalescing as a rate limit.'}
  - {id: "FR-048", path: "spec/functional/FR-048-session-work-monitor.md", role: examined, excerpt: 'A held, failed or untyped refresh keeps the last snapshot with its original source time, reports every blocker and the earliest reevaluation time the service gave, and counts down only to the monitor''s own next scheduled attempt, and only for a hold to wait out; a reevaluation time is not eligibility. A refresh whose typed outcome is absent stays unknown and is never classified from error text. This behavior extends FR-048-AC-5.'}
  - {id: "FR-048-AC-5", path: "spec/functional/FR-048-session-work-monitor.md", role: context_only, excerpt: 'Sources refresh independently. Failures preserve labeled last snapshots; source ages are available through the status toggle. Errors remain visible when status is hidden. Manual Linear refresh forces a fetch rather than renewing a cache timestamp.'}
  - {id: "FR-048-timer-paragraph", path: "spec/functional/FR-048-session-work-monitor.md", role: context_only, excerpt: 'Rate-limited reads SHALL retry on that timer, with a live countdown to the next attempt. Manual refreshes during rate limiting SHALL coalesce until that attempt; quit SHALL remain responsive.'}
  - {id: "accepted-sdk-lib.rs", path: "/Users/peter/dev/.dev-dept-recovery/swm-185-consumer/accepted-sdk-lib.rs", role: context_only, excerpt: 'Error::LinearRateLimited { .. } | Error::LinearLocalBudgetExhausted { .. } | Error::LinearQueryComplexityExceeded { .. } | Error::InvalidQuotaConfig { .. } => Self::Unknown,'}
```

## Finding details

### FND-001 (medium) The typed terms disagree with each other

Sentence two says `Only a server backoff, or the service's typed rate-limit error, is labeled rate-limited`. The last sentence says `A refresh whose typed outcome is absent stays unknown`. In the accepted SDK the typed rate-limit error `LinearRateLimited` is reported with `RefreshOutcome::Unknown` (`from_error`, lib.rs:460-463: `LinearRateLimited`, `LinearLocalBudgetExhausted`, `LinearQueryComplexityExceeded` and `InvalidQuotaConfig` all map to `Unknown`), so the refresh the second sentence labels rate-limited is, by the last sentence, a refresh whose typed outcome is absent and so stays unknown. The code resolves it by a third rule the paragraph does not state: an `Unknown` outcome with a reported cooldown is classed `ServerBackoff`, and one with only a known read age is classed as a failed refresh of unknown cause (`TrackerRead::provenance`). The paragraph also uses `typed state`, `typed outcome`, `typed rate-limit error` and `untyped refresh` for the decision state, the refresh outcome, an SDK error variant and a refresh that is none of those, without defining any of them; `untyped` appears once and cannot be matched to an SDK value. Two implementers can build opposite behavior for a legacy rate limit and both claim the paragraph. Confidence high. Fix: name the three typed sources once (the decision's state, the refresh outcome, the SDK's legacy rate-limit error), say which of them may produce the rate-limited label, and say what `unknown` means when the outcome is absent but a typed legacy error or cooldown is present.

### FND-002 (medium) The set of holds that wait is not stated

The fourth sentence limits the countdown to `a hold to wait out` and the third says local allocation and server reserve holds follow the rate-limit timer and coalescing. No sentence says which labels are not a hold to wait out. The code gives a countdown and coalescing to a server backoff, a local allocation hold, a server reserve hold and the legacy local-budget error, and gives none to quota evidence unavailable, quarantined legacy reservations, request not admissible, an unexplained refusal, a busy refresh lock or a failed source (`Cause::waits`). The legacy local budget is in neither sentence. The README names only local allocation and server reserve. A reader can conclude that evidence unavailable also follows the timer, since it is a hold, or that it does not, and a test cannot decide which. Confidence high. Fix: state the waiting causes and the non-waiting causes in the paragraph, including the legacy local budget, and state that the non-waiting causes show no countdown and do not coalesce manual refreshes.

### FND-003 (medium) The reporting obligation applies only to a kept snapshot

The fourth sentence starts `A held, failed or untyped refresh keeps the last snapshot with its original source time, reports every blocker and the earliest reevaluation time the service gave`. The report of every blocker and of the earliest reevaluation is a clause of keeping a snapshot. When the cache holds no snapshot, the first sentence still requires the label, but nothing says the monitor reports the blockers, the attempt or the earliest reevaluation. A failed or untyped refresh has no blockers to report, which the sentence also does not separate from a hold. The cold-start behavior is therefore unspecified, and the SDK flattens an actual backoff with no snapshot to a bare duration (SR-1851 FND-001), so the missing sentence is where a reader would look to decide whether that is allowed. Confidence medium: the first sentence arguably covers the label, and whether the cold path owes the same detail is the product decision the paragraph should record. Fix: state the reporting obligation for a hold with and without a kept snapshot, and limit `reports every blocker` to a typed hold that has blockers.

## Verdict after disposition pass 1

Not run at the review pass. Dispositions follow for this same reviewer session after the fix round.
