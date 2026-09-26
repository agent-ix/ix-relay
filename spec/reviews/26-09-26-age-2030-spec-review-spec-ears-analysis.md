---
id: SR-008
title: "EARS conformance of the FR-044 statements edited in ix-relay#26 round 3"
type: SpecReview
analysis: ears-conformance
scope: "agent-ix/ix-relay@36c2c46626cdf092005a5a05bd525f7a9780e29b; spec/functional/FR-044-watcher-delivers-through-claude-codes-inbox-socket.md (Behavior and Acceptance Criteria)"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay/FR-044"
    type: "reviews"
---
# SR-008: EARS conformance of FR-044 (round-3 edits)

## Summary

Ticket: AGE-2030. PR: ix-relay#26 at 36c2c46. The requirement statements added or edited
in round 3 were checked against the EARS patterns: the settlement retry, notifications
after writes, the delivered grace, adoption checks, the tick budget, dead-letter reason
expiry and ordering. Statements carried unchanged from earlier rounds were read for
context only.

The new one-line statements ("The watcher shall not discard the decisions still owed
when a connection error ends the tick", "When Claude Code has reported a held message
delivered and its transcript line is not found within 5 minutes, the watcher shall
acknowledge the batch anyway…", "The watcher shall not acknowledge a batch on a
`delivered` receipt alone before that grace ends", "The watcher shall not dead-letter an
approved message as never approved") are well-formed ubiquitous or event-driven EARS and
testable. Three edited statements are not.

## Findings

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-001 | low | "The watcher shall keep a decision about a batch (acknowledge, put back, dead-letter) until the write to the bus has succeeded, returning it on each tick until then, for at most five failed tries with one log line per episode, then a `settlement_failed` note." This is compound: it carries retention, a retry bound, a logging rule and a note. It also leaves the post-give-up outcome unstated: the batch's lease lapses and it is redelivered, so a denied message is re-prompted (SR-006 FND-003). Split it into atomic statements, including an `If … then` unwanted-behavior clause for the give-up. | spec/functional/FR-044-watcher-delivers-through-claude-codes-inbox-socket.md (Behavior) |
| FND-002 | low | "The watcher shall show a notification only after its write succeeded." The referent of "its" is unclear, and the scope is too wide as written: the bypass-mode "held" notification is not tied to any bus write. The intended rule applies to the `hold_expired` notice. Suggest: "When the bus write that dead-letters a batch as `hold_expired` succeeds, the watcher shall show one notification." | spec/functional/FR-044-watcher-delivers-through-claude-codes-inbox-socket.md (Behavior) |
| FND-003 | low | The acknowledgement and tick-budget bullet mixes six obligations with rationale in one paragraph: the ack rule, the 3 s wait, post-only-if-budget, the deferral semantics, the ack wait fitting the budget, and the rotating 32-per-tick checks in 512 KiB windows. "So … none starves" is a rationale with no measurable bound. Split it, and restate starvation as a bound (for example "a lead with mail is posted to within N ticks, where N is the number of leads ahead of it whose socket is not suspect"). | spec/functional/FR-044-watcher-delivers-through-claude-codes-inbox-socket.md (Behavior) |

## New findings (disposition pass 1)

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-004 | low | The round-4 split produced an unwanted-behavior clause with no system response: "If a batch's status file is lost, or its renewal is missed for more than 120 s (the watcher or the bus down or unreachable), then the batch degrades to at-least-once redelivery: ...". It states a consequence, not a "shall", so it cannot be verified as a requirement. Reword as a note (a known limit) or give it a `shall` (for example "then the watcher shall ... and `doctor` shall say so"). | spec/functional/FR-044-watcher-delivers-through-claude-codes-inbox-socket.md (Behavior) |

## New findings (disposition pass 2)

Checked at dc4ccd52dd29a424c73ce3defc81b7a2fa26eba5.

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-005 | low | The reworded clause: "If a batch's status file is lost, or its renewal is missed for more than 120 s (...), then the watcher shall accept at-least-once redelivery of that batch (...), shall keep the lead marked held, popping nothing for it, until the quiet period ends, and `doctor` shall show the lead as held." Under its own first trigger this cannot hold. A restarted watcher learns that a lead is held only from the status file (`state.held` is filled by adoption and by live receipts), so with the file lost it neither keeps the lead held nor can `doctor` show it. "Shall accept at-least-once redelivery" is also not an observable response. Scope the held and `doctor` obligations to the renewal-missed branch, and state the lost-file branch as a known limit. The new "A postponement shall leave ..." statement has no system as its subject (EARS wants "The watcher shall ..."). | spec/functional/FR-044-watcher-delivers-through-claude-codes-inbox-socket.md:112-116, spec/functional/FR-044-watcher-delivers-through-claude-codes-inbox-socket.md:188-189, crates/relay-watcher/src/socket/deliverer.rs:523 |

## Dispositions

| FND | outcome | sha/reason |
|-----|---------|------------|
| FND-001 | fixed | e002abf: split into atomic statements: keep pending until the write succeeds; renew and retry with a doubling wait; notify and note after five failures; an `If ... then` clause for a write that keeps failing; clear the note on success; keep decisions across a connection error. |
| FND-002 | fixed | e002abf: "When the bus write that dead-letters a batch as `hold_expired` succeeds, the watcher shall show one notification and shall not post the batch again." |
| FND-003 | fixed | e002abf: the ack, wait, window, budget, accounting, postponement and rotation obligations are separate statements, and starvation is a bound: "a lead with mail is reached within as many ticks as there are leads". |
| FND-004 | still-open | New this round; not addressed. An `If ... then` clause with no `shall`. |
| FND-004 | fixed | dc4ccd5: the `If ... then` clause now has system responses ("then the watcher shall accept ..., shall keep the lead marked held ..., and `doctor` shall show the lead as held"). Whether those responses can hold is FND-005. |
| FND-005 | still-open | New this round; not addressed. The new `shall`s cannot hold when the status file is lost. |
| FND-005 | fixed | 1df698c (squashed; checked at that head): the clause is scoped to the renewal-missed branch ("If a batch's renewal is missed for more than 120 s ... and its lease lapses, then the watcher shall keep the lead marked held ... and `doctor` shall show the lead as held"); the lost-file branch is two event-driven statements ("When a restarted watcher finds no status file, the watcher shall treat the lead as not held" / "shall redeliver the batch, marked `(redelivered)`, after its lease expires") plus a "Known limit" note; the postponement statement now has a subject ("When the transport postpones a batch, the watcher shall leave ..."). |
