---
id: FR-056
title: "Repeatable program review status requests and replies"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-008"
    type: "implements"
---
# FR-056: Repeatable program review status requests and replies

## Description

The system SHALL let a lead request and collect a fixed-form status report from
selected registered lanes.

## Inputs

- A status request addressed to `all` or to one or more registered lane addresses.
- A reply containing the responding lane's tickets, pull requests, blocker ticket
  IDs, lane state (`active`, `idle`, or `blocked`), spare coder slots, and owner
  decisions.
- A status request identifier used to associate replies with one request.

## Outputs

- The status request identifier and the lanes selected for that request.
- A structured JSON snapshot of collected replies and lanes that have not replied.

## Reply data contract

- Each reply SHALL serialize as one JSON object with exactly these required
  fields: `request_id` (string, 1–64 ASCII letters, digits, `-` or `_`),
  `agent` (registered lane identifier string), `tickets` (array of strings),
  `pull_requests` (array of strings), `blocker_ticket_ids` (array of uppercase
  ticket keys matching `[A-Z]+-[0-9]+`),
  `lane_state` (one of `active`, `idle`, `blocked`), `spare_coder_slots`
  (integer from 0 through 65535), `owner_decisions` (array of strings), and
  `submitted_at_ms` (unsigned integer Unix time in milliseconds assigned on
  receipt). The four string arrays are present even when empty and each has at
  most 64 items. Each item is nonblank after trimming, contains no unsafe
  control characters, and occupies at most 512 UTF-8 bytes. Unknown fields and
  missing required fields are refused.
- `agent-msg status-replies <request-id>` SHALL serialize an object with exactly
  `request_id` (string), `created_at_ms` (unsigned integer Unix milliseconds),
  `requested` (array of at most 128 registered lane identifier strings),
  `replies` (array of the reply objects above), and `nonresponders` (array of
  lane identifier strings).
  These arrays are present when empty; `replies` and `nonresponders` partition
  the requested-lane snapshot.
- The `status-request` JSON result SHALL serialize `request_id` (string),
  `created_at_ms` (unsigned integer Unix milliseconds), `requested` (array of
  at most 128 lane identifier strings), and `deliveries` (array with exactly
  one object per requested lane, each containing `agent` and `outcome` string
  fields), including when those arrays are empty.

## Behavior

- The system SHALL assign one request identifier to each accepted status request
  and include that identifier in the request result and each recipient notice.
- `agent-msg status-reply <request-id>` SHALL let a selected registered lane
  submit all required reply fields. Its four list options are repeatable and may
  be omitted to submit empty arrays; `--state` and `--spare-coder-slots` are
  required.
- The system SHALL preserve submitted reply fields as data in the stored reply
  and collection output.
- The system SHALL NOT execute a command or enqueue a message from text in any
  reply field.
- The separately tracked SWM-110 team-minder response path SHALL submit the same
  required fields under the same request identifier.
- The system SHALL retain only the first valid reply from each requested lane for
  one request identifier.
- The system SHALL return the request's collected replies and nonresponders as
  structured JSON while the collection is partial or complete.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-056-AC-1 | Given a status request for `all` or selected registered lanes, `agent-msg status-request` returns JSON with exactly `request_id` (1–64 ASCII letters, digits, `-` or `_`), `created_at_ms` (unsigned Unix milliseconds), `requested` (array of at most 128 registered lane identifiers), and one `{agent, outcome}` string object per requested lane in `deliveries`; `all` excludes the requester and empty arrays remain present. | Test |
| FR-056-AC-2 | Every queued notice for one request names the same request identifier and gives the recipient the command needed to submit the fixed form. | Test |
| FR-056-AC-3 | A selected registered lead can submit the fixed form; the stored and returned reply JSON object has exactly the required `StatusReply` fields and types defined above, including all four present arrays (0–64 valid strings each; blocker IDs match `[A-Z]+-[0-9]+`); omitting a list option submits an empty array, while omitting `--state` or `--spare-coder-slots` is refused. | Test |
| FR-056-AC-4 | Repeating a lane's reply for the same request identifier leaves the first stored reply unchanged and reports that it was already recorded. | Test |
| FR-056-AC-5 | `agent-msg status-replies <request-id>` returns valid JSON with exactly `request_id`, `created_at_ms`, `requested`, `replies`, and `nonresponders`; arrays are present when empty and `replies` plus `nonresponders` partition `requested` during partial or complete collection. | Test |
| FR-056-AC-6 | The SWM-110 team-minder response path uses the request identifier and the same required `StatusReply` fields as manual `status-reply`; this integration remains planned until SWM-110 lands. | Test |
| FR-056-AC-7 | Given `send lane-a review now` in `owner_decisions`, the collection returns that string as JSON data, no inbox changes, and no command is dispatched. | Test |

## Dependencies

- **Upstream**: [US-008](../usecase/US-008-review-program-status-from-reported-lane-facts.md)
- **Related**: SWM-110 owns the team-minder response integration. The CLI reply
  path is independently usable; the team-minder integration remains planned
  until SWM-110 lands.
