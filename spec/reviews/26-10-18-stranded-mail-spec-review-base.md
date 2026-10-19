---
id: SR-065
title: "Base review of the stranded-mail, claude-only socket, per-version drift, hook-failure severity and spool at-most-once spec change"
type: SpecReview
analysis: base
scope: "agent-ix/ix-relay@bcc889e (branch fix/doctor-signal-stranded-mail); spec/functional/FR-053-stranded-mail-is-forwarded-or-dead-lettered.md, spec/functional/FR-054-socket-delivery-is-for-claude-leads.md, spec/functional/FR-055-socket-status-keeps-only-live-leads.md, spec/functional/FR-029-doctor-reports-relay-health.md, spec/functional/FR-030-hook-failures-are-spooled-and-reported.md, spec/functional/FR-031-dead-letters-instead-of-drops.md, spec/functional/FR-044-watcher-delivers-through-claude-codes-inbox-socket.md, spec/functional/FR-046-claude-code-formats-are-typed-and-drift-is-detected.md, spec/tests.md"
review_set: base
relationships:
  - target: "ix://agent-ix/ix-relay/FR-029"
    type: "reviews"
  - target: "ix://agent-ix/ix-relay/FR-030"
    type: "reviews"
  - target: "ix://agent-ix/ix-relay/FR-046"
    type: "reviews"
  - target: "ix://agent-ix/ix-relay/FR-053"
    type: "reviews"
  - target: "ix://agent-ix/ix-relay/FR-054"
    type: "reviews"
  - target: "ix://agent-ix/ix-relay/FR-055"
    type: "reviews"
---
# SR-065: Base review of the stranded-mail and doctor-signal spec change

## Summary

Base checklist over the artifacts commit bcc889e adds or changes, checked against
NFR-001, NFR-002, NFR-004, FR-012, FR-016, FR-020, FR-031 and FR-042, major findings
only. The owner's decisions (forward to the new holder of the address with a
prior-session notice, dead-letter when the tab is gone, no grace delay, wait while the
new session has not registered, fleet upgrades together, the hook FAIL thresholds, and a
per-record id with dedupe for spool shipping) were taken as settled. Six findings, all
fixed in the spec in this review; FR-054, FR-055, FR-031 and FR-044 had no major
finding. One residual question for the owner is recorded as FND-007. ID formats,
coverage rows and `make check-matrix` are consistent after the fixes; quire validation
passes on every touched file.

## Findings

| ID | Severity | Summary | Refs | Escape Cause |
|----|----------|---------|------|--------------|
| FND-001 | high | FR-029-AC-14 required a burst of 100 failures from one session "within a single 15-minute window" to be a `warn`, while rule (a) makes 50 or more failures under 15 minutes old a `fail` fleet-wide; when that window is the latest one the AC contradicts the rule (and the owner's threshold), and the prose "a short burst on one machine is a warn" said the same. Fixed: the burst in AC-14 is 20 to 30 minutes old, a recent burst of 50 from one session is a `fail`, and the prose now says a burst is a `warn` only below 50 or once it ended over 15 minutes ago. | FR-029, FR-029-AC-14 | wrong-requirement |
| FND-002 | medium | FR-029 rule (b) did not say whether the four 15-minute windows are counted back from the check or aligned to the clock, so code and test could disagree at window edges; and with FR-030's dedupe a duplicate stream entry would inflate rule (a). Fixed: windows are counted back from the time of the check, and every count is by distinct record id (AC-14 gains a 50-entries/49-ids case). | FR-029, FR-029-AC-14, FR-029-AC-15, FR-030 | wrong-requirement |
| FND-003 | high | FR-030's at-most-once bullet forbade a second stream entry without naming any mechanism, and the owner's mechanism (a unique id per record, shipping and readers dedupe by it) was not in the text: no record carried an id, removal was "exactly the records shipped" with no key, nothing bounded the shipped-id memory (NFR-002), nothing kept dedupe inside the flush's chunk requests (NFR-001), and no reader was required to count an id once. Fixed: each record carries an `id` minted at append; removal is by id; each chunk ships in one atomic request that skips ids already recorded as shipped and records the new ones; that record keeps 24 h of ids up to 2,000; `doctor` and the watcher's metrics count an id once. AC-12 now asserts ids, plus a reader-dedupe case. | FR-030, FR-030-AC-12, NFR-001, NFR-002 | missing-requirement |
| FND-004 | high | FR-053 required a "prior session" notice line on every rendering path but did not say where the move records it or where the line goes. A separate leading line beginning `[ix-relay]` would break FR-016-AC-1 (only the first line begins with `[ix-relay]`), and a notice added at render time is outside the pop's weight (FR-012), so a batch of moved messages could exceed the 6 KiB hook-output and 16 KiB prompt bounds (NFR-002). Fixed: the move marks each entry atomically (a second move keeps the first mark; a later dead letter keeps the stored bytes, mark included); the notice is part of the signed first line; the pop weight includes it. New FR-053-AC-12 / TC-053-12. | FR-053, FR-016, FR-012, NFR-002, FR-031 | missing-requirement |
| FND-005 | medium | FR-053 required that a failure leave every message "in no more than one" inbox, and AC-6 asserted exactly-one under a bus failure injected at each step. The store's script discipline (FR-020) deliberately writes the destination before deleting the source so a script that fails part-way duplicates rather than loses; the requirement as written was unsatisfiable on Redis scripts and contradicted FR-020. Fixed: the move follows FR-020 (check keys first, destination before source); a crash or a second watcher leaves exactly one copy, a part-way script failure may leave two, never none; AC-6 and TC-053-06 say so. | FR-053, FR-053-AC-6, FR-020 | wrong-requirement |
| FND-006 | medium | FR-046 keeps hourly buckets and last-line members per Claude Code version with no bound on the number of versions or the version string's length, a map keyed by text read from another program's file (NFR-002), and that string is shown by `doctor`. Fixed: at most 16 versions (oldest newest-bucket dropped first, never failing a delivery), each string cut to 64 characters and neutralised. New FR-046-AC-19 / TC-046-19. | FR-046, NFR-002 | missing-requirement |
| FND-007 | low | Owner question, not changed: per the owner's rule, when a replaced pane's new session never registers (it is not a relay lead, or its hook never runs), FR-053 forwards nothing and the gone/dead-letter path never applies because the pane still hosts an agent, so that registration and its mail stay as before this change. Whether the wait should end (for example dead-letter after the 24-hour prune age) is the owner's call. | FR-053, FR-031, FR-042 | missing-requirement |
