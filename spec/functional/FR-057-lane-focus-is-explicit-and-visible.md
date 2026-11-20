---
id: FR-057
title: "Lane focus is explicit and visible"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-008"
    type: "implements"
---
# FR-057: Lane focus is explicit and visible

## Description

The system SHALL show a lane focus only after its lead explicitly reports it.

## Inputs

- Focus text supplied by the current registered lead, or a request to clear it.
- A caller-selected age threshold for marking a focus report stale.

## Outputs

- The explicitly reported focus and its set time, or an explicit unset state,
  for each registered lane.
- A stale marker when the report is older than the selected age threshold.

## Behavior

- `agent-msg focus` SHALL store the supplied focus only for the caller's
  registered lane.
- `agent-msg focus --clear` SHALL remove the caller's focus only.
- `agent-msg who --focus` SHALL show every registered lane, its reported focus
  or an explicit unset state, and the time the focus was set.
- `agent-msg who --focus --stale-after <age>` SHALL use the supplied age as its
  stale threshold. `<age>` SHALL be a nonnegative integer followed immediately
  by one suffix: `s` (seconds), `m` (minutes), `h` (hours), or `d` (days), as in
  `30s`, `15m`, `1h`, or `7d`. Fractions, signs, whitespace, other suffixes, and
  values that overflow seconds are refused.
- The CLI SHALL mark a focus stale only when its age in milliseconds is strictly
  greater than the threshold converted to milliseconds; equality and future
  timestamps are fresh. A stale focus remains visible.
- The system SHALL show a lane without an explicit focus as unset even when its
  open tickets or observed activity could suggest a focus.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-057-AC-1 | A lead's set command stores the supplied focus for its registered lane without changing another lane's value. | Test |
| FR-057-AC-2 | A lead's clear command removes its registered lane's focus without changing another lane's value. | Test |
| FR-057-AC-3 | `agent-msg who --focus` lists each registered lane with its explicit focus and set time, or says the focus is unset. | Test |
| FR-057-AC-4 | With `--stale-after 30s`, a focus age of 30,001 ms is marked stale and remains visible. | Test |
| FR-057-AC-5 | With `--stale-after 30s`, focus ages of exactly 30,000 ms and future timestamps are not marked stale. | Test |
| FR-057-AC-6 | `--stale-after` accepts integer durations using only `s`, `m`, `h`, or `d` (for example `30s`); fractional, signed, unitless, unknown-unit, whitespace, and overflowing values are refused. | Test |
| FR-057-AC-7 | A registered lane with no explicit focus remains unset even when its open tickets or observed activity could suggest a focus. | Test |

## Dependencies

- **Upstream**: [US-008](../usecase/US-008-review-program-status-from-reported-lane-facts.md)
- Lane focus is distinct from the per-session work reports described by [FR-048](./FR-048-session-work-monitor.md).
