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
Health and Retro show unavailable guidance when no configuration or saved report supplies those fields.

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

Press `1`, `2` or `3` for Status, Health or Retro.
Use arrows or `j` and `k` to select a row; `d` or Enter opens details.
Left and right arrows scroll department evidence details vertically.
Use `t`, `a` and `p` for the lead's Tickets, Agents and Scope views.
Press `r` to refresh and `q` or Escape to quit.

Source ages belong to the evidence timestamps and remain visible in department views.
A failed refresh retains the last snapshot with its original timestamps and a visible error.
Red severity can coexist with incomplete evidence.
Unknown, unavailable, unassessed, unrouted and `query_changed` retain their shared meanings.

Quitting a valid saved report succeeds without authentication.
Failed requested live collection returns nonzero after the terminal is restored.
Use [the CLI reference](cli-reference.md#team-leader-dashboard) for the option list.
