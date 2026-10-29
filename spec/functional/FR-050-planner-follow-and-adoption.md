---
id: FR-050
title: "Follow and adopt a planner focus"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/FR-048"
    type: "constrains"
---
# FR-050: Follow and adopt a planner focus

## Description

The relay SHALL associate a follower with one verified planner incarnation and an explicitly adopted planned-focus revision. When a planner publishes a newer revision, the relay SHALL retain the follower's adopted snapshot until explicit adoption.

## Behavior

A follow target is a full `server:workspace:tab`, resolved through verified relay directory/session evidence and Herdr process evidence. A relationship stores the planner's measured WorkBinding and last verified address; address is the discovery selector, not proof of identity. Existing native process/session binding requirements remain in force. An observed label rename updates the displayed address only when the same incarnation is verified; an unverifiable rename stays unknown. The command does not invent a nearby pane or session.

The first release permits one direct planner per follower. A publisher with a current published plan cannot itself follow until it explicitly withdraws publication and has no followers, and a follower cannot publish a shared plan; this rejects chains and cycles without recursive reads. Relationship creation validates both endpoints and any concurrent role change atomically. Self-follow, missing/ambiguous targets and absent published plans are rejected. Follow creates an unadopted relationship; it never auto-adopts the current plan. A follower must supply the planner's exact current revision to adopt it.

Adoption compares the current planner incarnation and revision atomically with the submitted expectation. If either changed, adoption returns a typed conflict and leaves the follower unchanged. The accepted snapshot is copied into the follower report, including source binding and revision, so it survives source expiry and disconnection. A newer published revision produces update pending; it does not change the effective adopted scope. An adopted paused plan has an explicitly empty work scope.

States are: not following; awaiting adoption; current (verified same source and adopted revision equals current); update pending (verified same source and newer revision); source unavailable (lookup/read failed or report expired); source replaced (verified different incarnation). Missing evidence never proves replacement. During source loss/replacement, retained adoption is labeled last adopted and its currentness is unknown. A verified replacement requires explicit follow with the replacement binding and a new adoption, even when the address or native session ID was reused.

Local work scope is preserved while following but is not an override for effective scope. Before initial adoption effective scope is unknown, never whole-team. Unfollow removes only the relationship and adopted snapshot; a saved local scope becomes effective again. If no local scope exists, the existing whole-team fallback is visibly labeled unknown focus. Source reports and concurrent activity/assignments remain unchanged.

Withdrawing publication is explicit and fails while followers remain; it does not delete historical revisions. A withdrawn source is unavailable to new follows. Rebinding does not transfer old followers to a new source incarnation.

Proposed CLI contract: `agent-msg work follow <full-address> [--replace-source]`, `agent-msg work adopt --revision <n>`, `agent-msg work unfollow`, and `agent-msg work plan withdraw --expected-revision <n>`. Repeated follow of the same incarnation is idempotent and preserves adoption; changing planner requires explicit `--replace-source` and clears adoption. Repeated unfollow is idempotent. Reads and writes use bounded session-qualified Redis records; no inbox is consumed and no tracker field is changed.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-050-AC-1 | Given a verified planner P and lead L, following P records their measured bindings with awaiting-adoption state and no effective scope; no shared focus is republished by L. | Test |
| FR-050-AC-2 | Given P current at revision 2, adoption expecting 2 stores that exact snapshot and becomes current; adoption expecting 1 changes nothing and reports conflict. | Test |
| FR-050-AC-3 | Given L adopted revision 2 and P publishes 3, L becomes update pending while scope and goals remain revision 2 until successful adoption of 3. | Test |
| FR-050-AC-4 | Given planner lookup failure, expired reports or Redis failure, adoption is retained and labeled source unavailable; verified new PID/session instead produces source replaced and requires explicit rebind. | Test |
| FR-050-AC-5 | Given a verified rename of the same incarnation, following continues without changing revision; a different session at the old address is never treated as the source. | Test |
| FR-050-AC-6 | Given self-follow, an existing follower chosen as planner, a current publisher trying to follow, or concurrent reciprocal follows, no cycle or chain is persisted. | Test |
| FR-050-AC-7 | Given local scope S, followed scope T and concurrent activity updates, adoption makes T effective without deleting S or activity; unfollow restores S and leaves source unchanged. | Test |
| FR-050-AC-8 | Given no saved local scope, unfollow exposes the whole-team fallback as lead-focus-unknown; an adopted paused plan stays empty rather than becoming whole-team. | Test |
| FR-050-AC-9 | Given source replacement or follow of a different planner, adoption is cleared only by explicit replacement; repeating the same follow preserves adoption and repeating unfollow succeeds. | Test |

## Dependencies

[FR-049](./FR-049-revisioned-planner-focus.md) defines revisions. [FR-048](./FR-048-session-work-monitor.md) defines binding and reports. SWM-129 and SWM-130 implement following and adoption.
