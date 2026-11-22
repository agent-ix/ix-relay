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

## Dispositions (round 1, all 3)

Disposition pass 1 at frozen source f4cfaad (atop c97b4dd, base d5fa1c3), PR 47, by the same reviewer, judged from the current FR-048 text (lines 43 to 49, which replace the single paragraph I reviewed) and the four FR-065 edits (lines 141, 150-152, 159-161 and 200-203), read as the bounded base check alongside the existing FR-048 scope and not as a review of the whole requirements. The code was read for the behavior the text describes. I ran no Quire command and no Vale. Parent-supplied facts relied on, none re-run by me: Quire validate of FR-048 and FR-065 exit 0 with only the two legacy line 16 EARS warnings outside the changed paragraphs and no new engine warning; diff check exit 0. No id, relationship, hash, digest, pin or `Verification` cell was added by either file, and FR-065 gets no new criterion.

| FND | Outcome | Reason |
|---|---|---|
| FND-001 | fixed | Line 43 now names the three typed sources and defines each: the decision state (local allocation held, server reserve held, server backoff, evidence unavailable, request not admissible, or available when no hold is reported), the refresh outcome (cache reused, refreshed, quota refused, source failed, refresh lock refused or unknown) and the legacy errors (rate-limited and local-budget-exhausted, reported with an unknown refresh outcome). Line 45 resolves the contradiction: the typed outcome decides first, a refresh is labeled rate-limited only for the server backoff decision state or the legacy rate-limited error, an unknown outcome with a legacy cooldown is treated as a server backoff, a failed refresh with no cooldown is labeled as an unknown cause, and nothing is classified from error text. `typed state`, `typed outcome` and `untyped` no longer appear. The code applies the same order (`TrackerFailure::cause`, `TrackerRead::provenance`). |
| FND-002 | fixed | Line 43 states the waiting causes (server backoff, local allocation hold, server reserve hold, the legacy local-budget error) and the non-waiting causes, and line 47 gives the countdown and the coalescing rule for each class as four singular SHALL sentences, including SHALL NOT for the non-waiting class. The legacy local budget is now listed. The two lists overlap on the unknown outcome, which is recorded as new finding FND-004 below and is not a reason to hold this one: the original defect, a set of waiting holds that was not stated, is closed. |
| FND-003 | fixed | Line 49 says that if the service provides a quota decision, with or without a stored snapshot, the monitor reports each blocker with its reported dimension and values, and reports the attempt and the earliest reevaluation time; an omitted reevaluation time or value is shown as unknown. The obligation is limited to a decision, so a failed or unknown refresh has no blocker to report and the text no longer implies one. A kept snapshot is a separate sentence (`If a refresh fails and a snapshot is stored ...`). The code agrees: a cold decision carries the same facts as a warm one (monitor_ops.rs:1042). |

## New findings (disposition pass 1)

Next unused id FND-004. It is low and is to be fixed in this PR.

| ID | Severity | Summary | Refs |
|---|---|---|---|
| FND-004 | low | The waiting and non-waiting lists in FR-048 line 43 overlap: the legacy local-budget error is a waiting cause and is reported with an unknown outcome, while `an unknown outcome` is a non-waiting cause, and line 45 makes an unknown outcome with a cooldown a waiting server backoff | FR-048, FR-048-AC-5 |

### FND-004 (low) An unknown outcome is both waiting and non-waiting

Line 43 says the legacy errors are reported `with an unknown refresh outcome`, lists `the legacy local-budget error` among the waiting causes, and ends the non-waiting list with `a source failure or an unknown outcome`. The legacy local-budget error therefore falls in both lists. Line 45 adds a third reading: an unknown outcome with a legacy cooldown is treated as a server backoff, which is a waiting cause, and an unknown outcome with no cooldown is a failure of unknown cause. So `an unknown outcome` is non-waiting only when it carries no cooldown and no legacy local-budget error, and the sentence does not say that. The code resolves it one way: `Cause::waits` is true for the local-budget cause and a failed refresh of unknown cause does not wait. One more case the text should record: with a kept snapshot the service reports no legacy error, so the same local-budget refusal arrives as an unknown outcome with no cooldown and is non-waiting, as the earlier SWM-184 decision recorded for the dashboard. A test of line 47 for an unknown outcome cannot decide between the two lists. Confidence medium: a careful reader takes the more specific waiting rule first, and the finding is the missing precedence, not a missing behavior. Fix: end the non-waiting list with `or an unknown outcome that reports neither a legacy cooldown nor the legacy local-budget error`, and say that the legacy local-budget error is a waiting cause only when the service reports it.

## FR-065 bounded check

The four edits are the `e` key row (`source errors and tracker quota detail`), the sentence that `e` keeps its Status, Health and Retro meaning and opens the tracker inspector in Tickets, Agents and Scope, the key-ownership list that adds the tracker inspector with Up, Down, PageUp, PageDown, Escape and `e`, and the evidence-row bullet that names the inspector as the full-text path. Each is a single statement that points to FR-048 for the behavior and does not restate it. They are consistent with FR-048 line 49 (the `e` key and the three lead views) and with the code (`View::is_department`, the popup key set in monitor_nav.rs). No ambiguity, undefined term, missing edge case or orphan reference was found in them, and no id, hash or pin was added. Nothing is raised against FR-065.

## Review context (disposition pass 1, appended)

Same disposition session as SR-1851. Correction to the Review Context above, which used the older cached quoin 0.29.4: this pass read the current files, all readable, under /Users/peter/.codex/plugins/cache/agent-ix/quoin/0.31.1/skills/: spec-review/SKILL.md and spec-review/references/checklist.md. Also read: dev-team/0.4.1/skills/reviewer/SKILL.md. Not opened: spec-review/references/workflow.md (no waves or SpecReview rewrite were run in this bounded pass). Examined: the added FR-048 text (lines 43 to 49) and the four FR-065 edits against ID format, terminology, verifiable behavior, error and edge conditions and the hash, digest and pin antipattern. Not run: any command, wave analysis, entity or relationship analysis, or whole-requirement sweep, by instruction. Assurance profile: none exists under spec/. The original text of this artifact above is unchanged.

## Current verdict (after disposition pass 1)

**CONDITIONAL** - all 3 initial findings are fixed on the text as read, and 1 new low finding (FND-004) is open, so the artifact stays CONDITIONAL until it is fixed. No high or medium finding. This is a reading of the spec and the parent-supplied results above; I ran no Quire command.

## Dispositions (round 2, FND-004 only)

Disposition pass 2 by the same reviewer session, PR 47, same single 185 tree, judged by reading FR-048 line 43 as edited and fix2-ready.md as a claim. Bounded to FND-004; no new method wave. I ran no command. Parent-supplied facts relied on, none re-run by me: Quire validate of FR-048 exit 0 with only the two old line 16 EARS warnings; the four cumulative SRs of round 1 validated and their original prefixes byte-preserved; the four round 1 comments posted to SWM-185. The edit changes no source, test, pin, README or FR-065 text.

| FND | Outcome | Reason |
|---|---|---|
| FND-004 | fixed | The waiting list now ends `or the legacy local-budget error when the service reports that error`, and the non-waiting list ends `a source failure, or an unknown outcome that reports neither a legacy cooldown nor the legacy local-budget error`, which is the wording I requested. The lists no longer overlap: an unknown outcome with a legacy cooldown is a server backoff by line 45 and waits, the reported legacy local-budget error waits, and an unknown outcome with neither is non-waiting; a kept-snapshot local-budget refusal reports no legacy error and so does not wait, as `Cause::waits` already behaves. No SHALL sentence changed and none gained a second obligation, no id, relationship or policy was added, and the text outside line 43 is unchanged. |

## New findings (disposition pass 2)

None. In the changed sentence, `when the service reports that error` is a single condition and `neither ... nor` is unambiguous. I did not re-review the rest of the paragraph.

## Review context (disposition pass 2, appended)

The same current paths as in the round 1 context were used, all readable, under /Users/peter/.codex/plugins/cache/agent-ix/: quoin/0.31.1/skills/spec-review/SKILL.md, quoin/0.31.1/skills/spec-ears-analysis/SKILL.md and dev-team/0.4.1/skills/reviewer/SKILL.md; no older cached copy was used. Scope limit: this round accepts the UI and spec subset at the current source only. The upstream 0.7.2 capacity bug hold is outside this artifact: the final SDK pin and its dependency-delta acceptance and checks are still required before the aggregate and merge, and nothing here says the delivery is complete.

## Current verdict (after disposition pass 2)

**PASS** - all 4 findings of this artifact are fixed on the text as read and no new finding was found. This is a reading of the spec and the parent-supplied results above; I ran no Quire command. It does not clear the upstream 0.7.2 hold.
