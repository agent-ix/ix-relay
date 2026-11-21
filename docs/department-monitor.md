---
type: how-to
title: Monitor a department
---

# Monitor a department

Open the terminal dashboard for an explicit team, project or saved department report.
Run these commands in a terminal with `team-leader-dashboard` on your path.
For live reads, configure Linear authentication with `ix-board auth login` first.
An approved saved credential works without `LINEAR_API_KEY`.

## Open a scope

```bash
team-leader-dashboard --team SWM
team-leader-dashboard --project <project-id> --project <another-project-id>
```

Status shows structured project progress, blockers and available lane observations.
The project selectors are source-native project IDs.
Add `--lead <server:workspace:tab>` to include verified lead activity.
A missing or ambiguous lead leaves the selected department scope open with an identity diagnostic.
Restart the dashboard explicitly after a bound session is replaced.

To collect Health and Retro with configured policies and source parameters, supply an existing retro configuration:

```bash
team-leader-dashboard --team SWM --retro-config <retro-config.json>
```

The configuration's department team and project populations must exactly match the command's selectors.
Without a configuration or saved report, Health and Retro show one "Not configured" message that names `--retro-config <file>`.
Retro also states that its previous and baseline comparisons need a retained snapshot history.
Before the first read of a supplied configuration they say they are waiting for it.
Neither state is a source failure or an authentication failure, neither starts a collection, and neither shows a zero or green value.

## Open a saved report

```bash
team-leader-dashboard --report <saved-report.json>
```

Accepted formats are an ix-board retro snapshot JSON, a snapshots JSONL history, or a retro delta JSON.
The selected snapshot must contain a supported department envelope.
History uses the shared previous and baseline selection rules.
One snapshot has unavailable previous and baseline comparisons.

Saved mode reads the supplied file without live authentication, network, Herdr or Redis setup.
Use `--report` by itself; live scope, lead, session and retro configuration options conflict with it.
Invalid data reports the path and decoding problem, leaves the bytes unchanged and exits nonzero.
Plan fields absent from a saved report remain unavailable.

## Navigate and inspect evidence

Every view shows the same header: the monitored lead, then a `View` selector with the current view and its key.
Press `v` to open the View dropdown and see all six views: Tickets, Agents, Scope, Status, Health and Retro.
Use Up, Down, `j` or `k` to move the highlight and Enter to select.
Escape or `v` closes the dropdown without changing anything.
Press F1 or `?` for help, which lists every key and the views where it applies.
Scroll help with Up, Down, PageUp and PageDown; F1, `?` or Escape closes it.
While a popup is open it receives every key, so `q` does not quit and nothing underneath changes.
Source refreshes continue while a popup is open.

The footer shows only `v View`, `F1 Help` and, when the width allows, `q Quit`.
The Scope view also shows its completed-projects key.
Source failures, stale snapshots, rate-limit countdowns and a replaced session appear in up to four evidence rows above the footer.
Each row keeps its label; long detail is cut with `…`, and a count shows any sources not shown.

The shortcuts remain: `1`, `2` or `3` for Status, Health or Retro, and `t`, `a` or `p` for Tickets, Agents or Scope.
Use arrows or `j` and `k` to select a row; `d` or Enter opens details.
Left and right arrows scroll department evidence details vertically.
Press `e` to inspect source errors, then select a source and open its full diagnostic with `d`.
Press `r` to refresh and `q` or Escape to quit when no popup is open.

## Know which population each view shows

Tickets and Scope describe the lead's reported focus, or say that no lead is bound.
Agents lists every reported assignment and is not limited by scope.
Status, Health and Retro name the department selection instead: the explicit team and projects, the saved report's scope, or the team derived from the lead's workspace across all its projects.
The department selection is chosen independently of the lead's reported focus; see Scope (`p`).
Status also shows a short purpose line.

Source ages belong to the evidence timestamps and remain visible in department views.
A failed refresh retains the last snapshot with its original timestamps and a visible error.
Red severity can coexist with incomplete evidence.
Unknown, unavailable, unassessed, unrouted and `query_changed` retain their shared meanings.

Quitting a valid saved report succeeds without authentication.
Failed requested live collection returns nonzero after the terminal is restored.
Use [the CLI reference](cli-reference.md#team-leader-dashboard) for the option list.
