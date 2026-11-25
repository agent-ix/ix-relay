---
id: SR-6120
title: "Integrity review of the FR-068 lock-skip requirement added in SWM-276 round 1"
type: SpecReview
analysis: integrity
scope: "agent-ix/ix-relay; PR #64 round 1 (56266b1); spec/functional/FR-068-hook-activity-events.md (new Behavior bullet and FR-068-AC-14), checked against crates/agent-msg/src/activity_state.rs and crates/agent-msg/src/hook_run.rs"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay/FR-068"
    type: "reviews"
---

# SR-6120: Integrity review of the FR-068 lock-skip requirement (SWM-276 round 1)

## Summary

Ticket: SWM-276. Fix round 1 of PR agent-ix/ix-relay#64 (head `56266b17b52d5ee4c0a6bcdf8a7455b859ad156b`) adds one Behavior bullet and FR-068-AC-14 to FR-068. They retarget the activity-state concurrency test, which was previously tagged FR-068-AC-11 (SR-6122 FND-003). This review checks the new statement and criterion for internal consistency and against the implementation they describe. No other spec file changed.

## Verdict

**CHANGES REQUESTED.** The new bullet and AC-14 promise that an update skipped for the lock is "reported as skipped to its caller" and that "no update SHALL be lost silently". That is false for the system as built. Every production path (`unregistered`, `dropped`, `failed`, `succeeded`, called from hook_run.rs:706-1035) goes through `ActivityState::update`, which discards the result (`let _ = self.try_update(change);`). The module doc says the skip is "an uncounted event". The AC-14 test calls the private `try_update` directly, so it passes while every real skip is silent. Either the spec should state the design honestly (a lock-skipped update is dropped uncounted, bounded by a 50 ms wait, and the file never holds a torn or lost *applied* update), or the code must surface the skip. Rewording the spec is the smaller change and matches the module doc. A second, low finding: "the lock bound" is not quantified anywhere in FR-068.

Clean: the bullet sits in the right section, after the skip/drop counting bullet. AC-14 is atomic in intent (concurrency plus held-lock) and has a single Test method. The test now carries `#[trace("FR-068-AC-14")]`.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | high | FR-068-AC-14 and the new bullet claim that a lock-skipped update is returned to its caller as skipped and none is lost silently. In production `update()` discards `try_update`'s result, so every lock skip is silent and uncounted (module doc: "an uncounted event"). The AC-14 test exercises only the private `try_update` | spec/functional/FR-068-hook-activity-events.md:137 |
| FND-002 | low | "within the lock bound" / "a bounded wait" never states the bound (50 ms, `LOCK_WAIT`); the test asserts `< 500 ms`, so two readers would derive different pass thresholds | spec/functional/FR-068-hook-activity-events.md:103 |
