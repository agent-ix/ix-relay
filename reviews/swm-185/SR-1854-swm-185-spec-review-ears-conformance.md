---
id: SR-1854
title: "Spec review (EARS conformance, bounded to the added paragraph) - FR-048 typed quota hold paragraph (SWM-185)"
type: SpecReview
analysis: ears-conformance
scope: "agent-ix/ix-relay; spec/functional/FR-048-session-work-monitor.md (the paragraph added at line 43 only)"
review_set: subset
relationships:
  - { target: "ix://agent-ix/ix-relay/FR-048", type: references }
---

## Summary

Ticket: SWM-185. PR pending. Frozen source HEAD c97b4dd against base d5fa1c3. EARS conformance bounded to the five sentences the change adds to FR-048 at line 43. The engine check was not run by me (`quire validate` is parent-run; the parent reports FR-048 validate exit 0 and supplied no `[ears:*]` warning output), so no engine finding is counted and the findings below are the semantic judgment the skill asks for. The paragraph has 5 statement sentences with 1 `SHALL`. Findings: 2 medium and 1 low. The dominant defect is that four of five obligations carry no normative keyword and the fourth sentence packs five obligations into one.

## Verdict

**CONDITIONAL** - no `high` finding (2 medium, 1 low). A reading of the paragraph against EARS; I ran no command.

## Findings

| ID | Severity | Summary | Refs |
|---|---|---|---|
| FND-001 | medium | Only the first sentence of the added paragraph contains `SHALL`; the rate-limit rule, the timer and coalescing rule, the snapshot and reporting rule and the unknown rule are stated as facts, so the engine counts one requirement and four obligations are not normative | FR-048 |
| FND-002 | medium | The fourth sentence is non-singular: it states five obligations (keep the snapshot and its time, report every blocker, report the earliest reevaluation, count down only to the monitor's own attempt, only for a hold to wait out) and a negation in one statement joined by `and` and a semicolon | FR-048 |
| FND-003 | low | The first sentence states an unwanted condition as a noun-phrase subject with no named system (`A Linear read that ... holds back SHALL be labeled`) instead of `If ..., the monitor SHALL ...` | FR-048 |

## Review Context

Reviewer model claude-sonnet-5-5; the harness run id is not visible and none is claimed. What was read: the installed quoin spec-ears-analysis skill (quoin 0.29.4 is the version visible to me; the brief names 0.31.1), FR-048 in the worktree and the adjacent timer paragraph at line 41 as the local dialect, and the SWM-163 EARS comment body as format precedent. Engine lexicon (`non-singular`, `vague-response`, `missing-subject`, `non-canonical-trigger`, `unclassifiable`) was applied by reading only. Not run: any command, including `quire validate`. No assurance profile applies.

## Examined Scope

```yaml
scope:
  - {id: "FR-048", path: "spec/functional/FR-048-session-work-monitor.md", role: examined, excerpt: 'A Linear read that the shared ix-board quota protection holds back SHALL be labeled by the typed state the service reports for that attempt: local allocation, server reserve, server backoff, quota evidence unavailable (including quarantined legacy reservations) or request not admissible.'}
  - {id: "FR-048", path: "spec/functional/FR-048-session-work-monitor.md", role: examined, excerpt: 'A held, failed or untyped refresh keeps the last snapshot with its original source time, reports every blocker and the earliest reevaluation time the service gave, and counts down only to the monitor''s own next scheduled attempt, and only for a hold to wait out; a reevaluation time is not eligibility.'}
  - {id: "FR-048", path: "spec/functional/FR-048-session-work-monitor.md", role: examined, excerpt: 'Only a server backoff, or the service''s typed rate-limit error, is labeled rate-limited. Local allocation and server reserve holds follow the same timer and coalescing as a rate limit. A refresh whose typed outcome is absent stays unknown and is never classified from error text.'}
  - {id: "FR-048-timer-paragraph", path: "spec/functional/FR-048-session-work-monitor.md", role: context_only, excerpt: 'Automatic Linear reads SHALL run on a 30-minute timer. Rate-limited reads SHALL retry on that timer, with a live countdown to the next attempt. Manual refreshes during rate limiting SHALL coalesce until that attempt; quit SHALL remain responsive.'}
```

## Finding details

### FND-001 (medium) Four of five obligations have no normative keyword

The adjacent timer paragraph puts `SHALL` in every sentence. The added paragraph has one: `A Linear read ... SHALL be labeled`. The next four sentences state `Only a server backoff ... is labeled rate-limited`, `Local allocation and server reserve holds follow the same timer and coalescing as a rate limit`, `A held, failed or untyped refresh keeps ... reports ... counts down` and `A refresh whose typed outcome is absent stays unknown and is never classified from error text` as plain present-tense facts, and the last closes with `This behavior extends FR-048-AC-5`. The engine counts one `shall` and so classifies the block as one statement; a requirement-bearing sentence with no `SHALL` is not an obligation a test can be traced to. The keep, report, count-down and never-classify rules are the behavior the change implements, so they are the ones that need the keyword. Confidence high. Fix: state each obligation as a `SHALL` sentence with the monitor as the subject.

### FND-002 (medium) The fourth sentence is non-singular

`A held, failed or untyped refresh keeps the last snapshot with its original source time, reports every blocker and the earliest reevaluation time the service gave, and counts down only to the monitor's own next scheduled attempt, and only for a hold to wait out; a reevaluation time is not eligibility.` It joins five verbs (keeps, reports every blocker, reports the earliest reevaluation, counts down, restricts the countdown) under three conditions (held, failed, untyped), and ends with a negation (`not eligibility`). One test or one acceptance criterion cannot carry it, and the three conditions differ in what they can report (a failed or untyped refresh has no blocker). Confidence high. Fix: one obligation per sentence: keep the snapshot and its time; report each blocker of a typed hold; report the earliest reevaluation with the statement that it grants no eligibility; count down to the monitor's own next attempt for the holds that wait.

### FND-003 (low) The trigger is stated as a noun phrase with no system

The first sentence's subject is `A Linear read that the shared ix-board quota protection holds back`, an unwanted condition, and the system that labels it is not named. The EARS pattern for an unwanted condition is `If <condition>, then the <system> SHALL <response>`, here `If the shared ix-board quota protection holds back a Linear read, the monitor SHALL label it ...`. The adjacent paragraph's passive subjects (`Automatic Linear reads SHALL run`) are the local dialect, so this is cosmetic and a reasonable reviewer could reject it. Confidence low. Fix: name the monitor and use the unwanted-condition form.

## Verdict after disposition pass 1

Not run at the review pass. Dispositions follow for this same reviewer session after the fix round.
