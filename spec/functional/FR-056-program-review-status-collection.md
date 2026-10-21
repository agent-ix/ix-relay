---
id: FR-056
title: "Collect fixed-form program review status from selected leads"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-001"
    type: "implements"
---
# FR-056: Collect fixed-form program review status from selected leads

## Description

`agent-msg status-request` shall create a request with a unique request id and a snapshot of selected registered lanes, then enqueue one request notice per lane. A lane shall submit at most one validated fixed-form reply for that request. `agent-msg status-replies` shall print the request, its replies, and the requested lanes that have not replied as JSON.

## Behavior

- `status-request all` selects every registered lane except the requester; a comma-separated list selects only the named registered lanes. Unknown, duplicate, or self-targeted lanes are refused.
- The request id is included in the request report and notice. Each delivery outcome is reported without retrying an uncertain enqueue.
- A reply includes ticket IDs, pull requests, blocker ticket IDs, lane state, spare coder slots, and owner decisions. Only a requested lane may reply, and the first valid reply is retained.
- Replies remain available for seven days. The collection report includes the requested lanes, stored replies, and nonresponders.

## Dependencies

- **Upstream**: [US-001](../usecase/US-001-message-a-lead-by-where-it-sits.md)

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-056-AC-1 | Given `all` or explicit registered addresses, `status-request` creates a request id, reports the selected lanes, and reports each enqueue outcome; unknown, duplicate, and self lanes are refused. | Test |
| FR-056-AC-2 | Given a request id, a selected lane can store one valid fixed-form reply; an unrequested lane is refused and a later reply cannot replace the first. | Test |
| FR-056-AC-3 | Given a partly answered request, `status-replies` emits JSON with the request id, requested lanes, replies, and nonresponders. | Test |
