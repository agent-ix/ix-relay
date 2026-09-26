---
id: SR-005
title: "Measured stability of herdr's ids across renames, moves, closes and restarts"
type: SpecReview
analysis: base
scope: "herdr 0.9.1 workspace, tab, pane and terminal ids; FR-032, FR-033, FR-034, FR-043"
review_set: subset
---
# SR-005: Measured stability of herdr's ids across renames, moves, closes and restarts

## Summary

Measured on herdr 0.9.1 (client and server, protocol 22, stable channel) on 2026-09-26
for AGE-2033. The question: which herdr id can a registration follow when a lead's
workspace, tab or pane is renamed, moved or closed around it, and when herdr restarts?

The lead's expectation was that workspace, tab and pane ids survive renames, that `pane
move` yields a new pane id, and that the harness session id is the durable join key.
Two of the three held. A `pane move` **within its workspace keeps the pane id**
(only the tab id changes); a move to **another workspace** gives the pane a new id in the
new workspace. In both cases the pane keeps its `terminal_id` and its
`agent_session.value`. The session id is therefore the key the relay follows
([FR-043](../functional/FR-043-moves-and-renames-are-followed-and-recorded.md)).

How it was run: a private `herdr server` under its own `HOME`, `XDG_*`, `HERDR_CONFIG_PATH`,
`HERDR_SOCKET_PATH` and `HERDR_SESSION`, started with `env -i`, in a scratch directory.
The operator's own herdr server (same pid before and after) was never addressed: every
command printed its socket path and it was under the scratch directory each time. The
panes ran a plain shell; agents were made visible with `herdr pane report-agent` and
`herdr pane report-agent-session --source herdr:claude`, the shape a harness integration
reports. The listings after each step are the fixtures in
`crates/relay-watcher/fixtures/herdr-0.9.1/` (working directories rewritten to `/w`).

## Findings

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-001 | medium | Renaming a workspace, a tab or a pane leaves every id (workspace, tab, pane, terminal) unchanged; only the label in the listing changes. A registration's place goes stale on a rename and its ids do not. | FR-034, FR-043 |
| FND-002 | medium | `pane move` to another tab of the same workspace keeps the pane id (`w1:p3` stays `w1:p3`); the tab id changes (`w1:t1` to `w1:t2`). A registration naming the pane still finds it, and its place (tab label) changes: a place change, not a relink. | FR-033, FR-043 |
| FND-003 | high | `pane move` to another workspace gives the pane a new id in the destination (`w1:p3` to `w2:p2`), with the same `terminal_id` and the same `agent_session.value`. The old id names nothing. The response names `previous_pane_id`, `previous_tab_id` and `previous_workspace_id`, but the watcher never sees a response, only listings: the session id in `agent list` is what links the two. | FR-033, FR-043 |
| FND-004 | medium | Ids are never reused within a run: after `w1:p3` left, the next pane of workspace `w1` was `w1:p4`; after `w1:t3` and `w1:t1` closed the next tab was `w1:t4`; after `w2` closed the next workspace was `w4`. Closing a neighbour changes no surviving id (only the positional `number` shifts). A stale pane id therefore never names a different, newer pane; it names nothing, or the same pane. | FR-033, FR-043 |
| FND-005 | high | A server restart keeps workspace, tab and pane ids, labels and the counters (the next pane was `w1:p7`), but every `terminal_id` is new and `agent list` is empty until a harness reports its session again. Right after a restart nothing can be concluded about any session: the relay must treat an empty answer as unknown, not as "the pane is gone". | FR-043 |
| FND-006 | low | `terminal_id` survives moves but not restarts, and registrations do not record it. The session id survives both when the harness re-reports it (not measured, see below). The relay follows the session id. | FR-043 |
| FND-008 | medium | herdr's `agent_session` carries a required `kind`, `id` or `path` (`herdr api schema`, `AgentSessionRefKind`). A path is a transcript file, not a session id. ix-relay reads a session only when `kind` is `id`. herdr 0.9.1's CLI could not be made to report a `path` session in the private server (`--agent-session-path` alone produced no `agent_session`), so the `path-kind` fixture is a real listing with one session's `kind` and `value` edited to the schema's path shape, and is synthetic in that member. | FR-043 |
| FND-007 | low | `herdr pane report-agent-session` with a `--source` that is not `herdr:<agent>` exits 0 and prints nothing, but no `agent_session` appears. Nothing in ix-relay calls it; recorded so a future test harness does not mistake the silence for success. | none |

## Measurements

Ids as `workspace / tab / pane / terminal`, for the lead in pane `w1:p3` (session
`sess-A3`) unless said otherwise. "same" means byte-identical to the step before.

| Step | Workspace | Tab | Pane | `terminal_id` | `agent_session.value` | Other |
|------|-----------|-----|------|---------------|-----------------------|-------|
| start: `wsA` (`w1`) has tabs `1` (`w1:t1`: `w1:p1`, `w1:p3`) and `tabB` (`w1:t2`: `w1:p2`); `wsB` (`w2`) has `w2:p1` | `w1` | `w1:t1` | `w1:p3` | `term_65c6235f3f5194` | `sess-A3` | fixture `start` |
| rename workspace `wsA` to `wsA-renamed` | same | same | same | same | same | `workspace list` label changed; all four agents unchanged (`workspace-renamed`) |
| rename tab `tabB` to `tabB-renamed` | same | same | same | same | same | `tab list` label changed (`tab-renamed`) |
| rename pane `w1:p2` (`pane rename`) | same | same | same | same | same | `pane get` shows `label`; `agent list` unchanged |
| move `w1:p3` to tab `w1:t2` (`--split right`), same workspace | `w1` | `w1:t1` to `w1:t2` | **same** `w1:p3` | same | same | response: `changed`, `previous_pane_id` `w1:p3`, `previous_tab_id` `w1:t1` (`pane-moved-same-workspace`) |
| move `w1:p3` to tab `w2:t1` (`--split down`), other workspace | `w1` to `w2` | `w1:t2` to `w2:t1` | `w1:p3` to **`w2:p2`** | same | same | response: `previous_pane_id` `w1:p3`, `previous_workspace_id` `w1`; old id absent from `agent list` (`pane-moved-other-workspace`) |
| new pane in `w1` after `w1:p3` left | | | `w1:p4` (not `w1:p3`) | new | none | ids not reused |
| new tab in `w1` | | `w1:t3` | `w1:p5` | new | none | |
| close pane `w1:p4`, tab `w1:t3`, tab `w1:t1` (neighbours of `w1:p2`) | | | survivors unchanged | survivors unchanged | survivors unchanged | tab `w1:t2` stays `w1:t2`; `tab list` `number` 2 stays 2 (`neighbours-closed` after `w2` still has `w2:p1`, `w2:p2`) |
| new tab in `w1` after those closes | | `w1:t4` (not `w1:t1`, `w1:t3`) | `w1:p6` | new | none | ids not reused |
| close workspace `w2`, then create one | `w4` (not `w2`; `w3` was taken) | | | | | ids not reused |
| stop the private server, start it again (`server stop`, `server`) | same | same | same (`w1:p2`, `w1:p6`, `w3:p1`, `w4:p1`) | **all new** | **none**: `agent list` is `[]` (`server-restarted`) | pane label kept; next pane `w1:p7`, next workspace `w5` |

## Not measured

- A real harness (Claude Code) re-reporting its session into a restored pane after a
  restart. In the private server, `report-agent` and `report-agent-session` sent to a
  restored shell pane produced no agent in `agent list`, so the post-restart state
  after a harness reports again was not observed. The relay's behaviour does not
  depend on it: it acts only on a session it sees.
- `pane swap`, `pane move --new-tab`, `pane move --new-workspace`, and a live handoff
  update (`herdr update --handoff`).
- ids on a remote machine through `herdr --machine`, and any herdr other than 0.9.1.
- A new agent starting in a pane that a lead left (the FR-032 case): that changes the
  session in a pane, not an id, and was simulated only by the fixtures' session values.

## Consequence for the design

- The pane id is a locator that can go stale on a cross-workspace move; it never
  points at a different pane later. The session id is the key to follow.
- A same-workspace move and every rename change the place, not the pane id: they are
  recorded as renames.
- After a restart the relay sees no agents. It changes nothing then: no relink, no
  replaced-pane marker, and the 24-hour pruning rule is unchanged.
