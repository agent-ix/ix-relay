---
id: FR-065
title: "One View selector and F1 help across the dashboard's six views"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/FR-048"
    type: "constrains"
  - target: "ix://agent-ix/ix-relay/FR-059"
    type: "constrains"
  - target: "ix://agent-ix/ix-relay/FR-060"
    type: "constrains"
  - target: "ix://agent-ix/ix-relay/FR-064"
    type: "constrains"
  - target: "ix://agent-ix/dev-team/FR-004"
    type: "depends_on"
---
# FR-065: One View selector and F1 help across the dashboard's six views

## Description

The existing `team-leader-dashboard` SHALL offer one View selector that lists all
six existing views (Tickets, Agents, Scope, Status, Health, Retro) and one F1 help
popup, identical in every view. Detailed key lists move from the persistent
screen into help. This requirement changes navigation, labeling and the display of
existing states only. It adds no data source, collector, telemetry, metric,
classification, scope calculation, credential or setup behavior, and leaves source
polling, binding fences, the `ix-cli-kit` terminal lifecycle and Ratatui
unchanged. The six views, their data and their selection, scroll and detail
semantics remain owned by [FR-048](./FR-048-session-work-monitor.md),
[FR-059](./FR-059-department-monitor-adapter.md) and
[FR-060](./FR-060-focused-scope-presentation.md). The Status, Health and Retro
contents remain owned by [dev-team FR-004](ix://agent-ix/dev-team/FR-004).

### Terms

- **Work area**: the table and details region of a view, excluding the header
  rows (identity row with the View selector, and scope label), the evidence rows
  and the controls footer row.
- **Evidence rows**: the rows that show source, stale, last-snapshot, error,
  rate-limit countdown, binding and filter state.

### Permitted presentation differences

This is the one list of differences that FR-065 allows in existing fixtures. A
fixture of FR-048, FR-059, FR-060 or FR-064 may differ only in:

1. the header View selector row, and its key styling;
2. the identity row, including in the department views;
3. the controls footer row;
4. the placement and bounding of the evidence rows;
5. the population labels stated under Scope labels;
6. the Status purpose line;
7. the Health and Retro state display stated under Empty Health and Retro,
   replacing the placeholder table and the "Unavailable: no shared department
   report" text; and
8. removal of persistent tab strips.

Data, row order, counts, keys' actions, selection, scroll and details semantics
remain as those requirements state.

### Precedence

FR-065 takes precedence over the following statements, only to the extent of the
list above:

- FR-048-AC-4 "The selected tab is underlined, including its hotkey" (no persistent
  tab strip) and "Only footer hotkeys use the bright primary blue" (see Styling).
- FR-059-AC-4 "Existing compact Tickets/Agents/Scope fixtures ... retain their
  outputs".
- FR-060 "Scope fixtures change only for the rows, labels and key above", and
  "The Tickets and Agents views, the department views and the existing keys are
  unchanged" (the keys remain unchanged).
- FR-064 "The department views ... are unchanged" and FR-064-AC-1 "Scope view output
  identical to the FR-060 fixtures". FR-064-AC-13's equality between legacy and
  named mode still holds, because both modes receive the same differences.
- FR-048 and FR-059 statements that name `1`, `2`, `3`, `t`, `a`, `p`: these keys
  remain, as compatibility shortcuts with the same actions.

Everything else in those requirements, including the Scope filter text of FR-060 and
Escape quitting when no popup is open, is retained.

## Behavior

### View selector

- Every view SHALL show in its header the same selector labelled `View`, with the
  current view name and the key that opens it. The selector SHALL NOT depend on the
  view, on data availability or on startup mode.
- The dashboard SHALL allow selecting a view whose data is unavailable; that view
  shows its own state.
- The key `v` (no modifier) SHALL open a dropdown at the selector's column, on the
  first rows of the work area. The key collides with no existing monitor key.
- The dropdown SHALL list all six views in the fixed order Tickets, Agents, Scope,
  Status, Health, Retro, each with its compatibility key, and SHALL start the
  highlight on the current view.
- While the dropdown is open, Up, Down, `j` and `k` SHALL move the highlight.
- The dropdown SHALL clamp the highlight at the first and last entry without wrap.
- While the dropdown is open, Enter SHALL select the highlighted view and close the
  dropdown.
- While the dropdown is open, Escape and `v` SHALL close it without changing the
  view.
- Closing the dropdown by Escape or `v` SHALL NOT quit.
- Closing the dropdown by Escape or `v` SHALL NOT change selection, scroll,
  details, or filter state.
- Selecting a different view SHALL have exactly the effect of that view's
  compatibility key (FR-059: preserve scope, reset row selection and scroll).
- Selecting the current view SHALL close the dropdown and change nothing else.
- If the work area is too short for the whole dropdown, the dropdown SHALL scroll to
  keep the highlight visible. All six entries are reachable at 80x24.
- This requirement SHALL NOT add a mouse capture mode. If a later change makes the
  terminal lifecycle deliver mouse events, a click on the selector SHALL open the
  dropdown, a click on an entry SHALL select it as Enter does, and a click elsewhere
  SHALL cancel as Escape does. Without mouse events, keyboard access is complete.

### Help popup

- F1 or `?` SHALL open a help popup from every view.
- While help is open, F1, `?` and Escape SHALL close it without quitting and
  without changing any view state.
- While help is open, Up, Down, PageUp and PageDown SHALL scroll it; every line is
  reachable at 80x24.
- Help SHALL name each key by the requirement that owns its action, with the views
  where it has a visible effect, as follows. Help SHALL NOT mark a key as applicable
  in a view where it has no visible effect.

  | Keys | Action (owner) | Views |
  |---|---|---|
  | `v` | View selector (FR-065) | all |
  | `t`, `a`, `p`, `1`, `2`, `3` | view shortcuts (FR-048, FR-059) | all |
  | Up, Down, `j`, `k`, PageUp, PageDown | row selection and paging (FR-048) | all |
  | `d`, Enter | details (FR-048) | all |
  | Left, Right | scroll details (FR-059) | Status, Health, Retro, with details open |
  | `r` | refresh (FR-048) | all |
  | `s` | source timestamps (FR-048) | Tickets, Agents, Scope |
  | `e` | source errors (FR-059) | Status, Health, Retro |
  | `c` | completed projects (FR-060) | Scope |
  | `q`, Escape | quit, when no popup is open (FR-048) | all |
  | F1, `?` | help (FR-065) | all |

- Help SHALL mark the keys that apply in the current view.
- The key `f` (FR-064) is not part of this table or of FR-065's acceptance. When
  FR-064 delivers `f`, that delivery adds its Help row (Scope view, named mode only)
  and its popup-sweep entry, and FR-064 verifies them. FR-065 does not wait on it.
- This requirement SHALL NOT change what `s`, `e`, Left or Right do in any view.

### Modal isolation

- The dashboard SHALL allow at most one popup open at a time.
- While a popup is open, the dashboard SHALL deliver every key and mouse event to
  that popup only.
- Each popup SHALL act only on the keys it owns: the dropdown owns Up, Down, `j`,
  `k`, Enter, Escape and `v`; help owns Up, Down, PageUp, PageDown, Escape, F1 and
  `?`.
- A popup SHALL ignore every other key, including a key that would open the other
  popup, without changing any state, and SHALL stay open.
- While a popup is open, `q` SHALL NOT quit, and Escape SHALL close the popup
  rather than quit.
- Source polling, refresh results and the terminal lifecycle SHALL continue while a
  popup is open.
- A popup SHALL draw only within the work area. It SHALL NOT cover the header rows,
  the evidence rows or the controls footer row.
- Closing a popup SHALL leave view, selection, scroll, details and filter state as
  before it opened, except for changes from refresh results or terminal resize
  received while it was open. Those apply the existing clamping rules (FR-059,
  FR-060) as if no popup were open.
- A terminal resize SHALL re-lay the popup out, keep it open, and clamp its
  highlight or scroll position into range.

### Header identity and footer

- The identity row SHALL show the same monitored lead identity, binding state and
  activity markers in all six views, and switching views SHALL NOT change them.
- With no lead bound, the identity row SHALL say so plainly and keep any binding
  diagnostic.
- The normal footer SHALL be one row showing `v View` and `F1 Help` in every view,
  and `q Quit` when width allows.
- The footer SHALL NOT list other shortcuts, except the filter key and state text
  that FR-060 requires in the Scope view.

### Evidence rows

- The dashboard SHALL show evidence rows outside any popup, together with the
  identity row and the controls footer, at every size from 80x24 up.
- At 80x24 the evidence rows SHALL occupy at most 4 rows in total.
- Each evidence item SHALL show its classified label intact: source name, state
  (failed, stale, last snapshot, waiting, rate-limit countdown, binding), reason
  class, and filter state. A label SHALL NOT be truncated.
- Free-form detail text after the label MAY be truncated to fit. Truncation SHALL end
  with a visible marker, and SHALL NOT remove the label.
- If more sources have evidence than fit, the evidence rows SHALL show the highest
  severity first and a visible count of the sources not shown.
- Where a view already has a diagnostic path, the full text SHALL remain reachable
  there (Status, Health and Retro: `e` lists sources and `d` opens the full
  diagnostic, FR-059). This requirement adds no new diagnostic view; in Tickets,
  Agents and Scope the truncated detail is accepted.
- The supported minimum size is 80x24. Below it, the dashboard SHALL truncate
  without panic or quit; no other guarantee applies.
- The `v View` and `F1 Help` hints SHALL NOT be dropped at 80 columns or wider.
- Evidence rows SHALL take priority over `q Quit` when width is short.

### Styling

- The key hint in the View selector, the dropdown, the help popup and the footer
  SHALL use the same bright primary blue; their descriptions and separators are
  muted. Nothing else uses the blue for a key.

### Scope labels

- In Tickets and Scope, the label SHALL be the population label that FR-060 gives
  for the current mode (Focused, whole-team fallback, ticket-only), verbatim; in
  named mode the label SHALL follow FR-064.
- If no lead is bound, then Tickets, Agents and Scope SHALL say that no lead is bound
  and SHALL NOT say or imply a reported lead focus.
- In Agents, the label SHALL say that every reported assignment is listed, not
  limited by scope (FR-048-AC-6). It SHALL NOT call the population a scope.
- In Status, Health and Retro, the label SHALL name the department selection. The
  selection is one of: the explicit team and projects; the saved report's scope; or,
  when only a lead is given and no team or project, the team derived from the bound
  lead's workspace across all its projects, labeled "derived from lead workspace".
- Status, Health and Retro SHALL show the fixed text "Department scope, chosen
  independently of the lead's reported focus; see Scope (p)."
- Label text SHALL NOT state or imply that the department population and the lead's
  scope are equal, the same, narrower or a subset.
- This requirement adds no calculation of either population.

### Status purpose

- Status SHALL show the adapter line "Status: progress, blockers and lane evidence
  for the selected department scope." It may wrap at 80 columns.
- This line is relay-local orientation text. It SHALL NOT define, add to or replace
  the Status contents owned by dev-team FR-004, and Status content is unchanged by it.

### Empty Health and Retro

- Health and Retro SHALL each choose one state from their own fields and show it as
  one concise message in the work area, not a table of placeholders:
  - **Not configured**: no retro configuration or saved report supplies the fields.
    For a live startup, the message names `--retro-config <file>` whose department
    team and project population must match the department selection shown in the
    label. For Retro it also names that previous and baseline comparisons need a
    retained snapshot history. For a saved report, it says the report does not
    supply the fields and that a report containing them is required.
  - **Waiting**: a configuration was supplied and its first read has not returned.
    The message says it is waiting for the first read of the retro configuration.
  - **Failed**: a read failed with no retained model. It shows the classified source
    reason.
  - **Stale**: a retained model exists. It shows the retained values with their
    stale or last-snapshot label.
- The Not configured and Waiting states SHALL NOT be stored, labeled or styled as a
  source failure. They SHALL NOT appear in the source-failure rows or the `e` source
  list, SHALL NOT be described as an authentication failure, and SHALL NOT by
  themselves affect the exit status. FR-059's rules for real collection failures are
  unchanged.
- The dashboard SHALL NOT show zero, green or complete values for missing, waiting,
  failed or not-configured data.
- Showing the Not configured or Waiting state, and opening or closing either popup,
  SHALL NOT cause the dashboard to start collection.
- Showing those states, and opening or closing either popup, SHALL NOT cause the
  dashboard to retry authentication.
- Showing those states, and opening or closing either popup, SHALL NOT cause the
  dashboard to write any file, report, tracker or retro history.
- Scheduled source polling and completed refreshes continue as FR-048 and FR-059
  state, and are unchanged by these states and popups.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-065-AC-1 | In each of Tickets, Agents, Scope, Status, Health and Retro, at 80x24 and 120x40, the header shows the same `View` selector; `v` opens a dropdown listing all six views with their keys and the highlight on the current view; Down/Up/`j`/`k` then Enter selects, and the target view renders with selection and scroll reset and scope preserved. Selecting the current view changes nothing. Health and Retro are selectable with no data. | Test |
| FR-065-AC-2 | Escape or `v` with the dropdown open closes it and leaves view, selection, scroll, details and filter unchanged, and does not quit; Escape with no popup open still quits. The highlight is clamped at both ends. At 80x24 all six entries are reachable, and a dropdown taller than the work area keeps the highlight visible. Resizing from 120x40 to 80x24 with it open keeps it open with the highlight in range. | Test |
| FR-065-AC-3 | F1 and `?` open help in all six views; F1, `?` and Escape close it without quitting or state change. For every view, the keys marked applicable equal exactly that view's column of the Help table, and `s`, `e`, Left/Right and `c` are marked only where the table says. Every line is reachable at 80x24 by scrolling. Resizing with help open keeps it open with scroll in range. Nothing about `f` is asserted. | Test |
| FR-065-AC-4 | For each popup, a fresh popup is opened per key and each key it does not own is sent, drawn from the keys defined by FR-048, FR-059 and FR-060 plus F1, `?` and `v`. After each, the popup is still open and quit, view, selection, scroll, details, refresh flag, status toggle, filter mode and the other popup are unchanged. Owned keys are asserted by AC-1 to AC-3. `q` while a popup is open does not quit. | Test |
| FR-065-AC-5 | In all six views the footer contains `v View` and `F1 Help`, and no other shortcut list except the FR-060 Scope filter text; no persistent tab strip appears. At 80x24, with a source error, a stale source, a rate-limit countdown, a replaced session and an active filter present, all classified labels are intact and visible in at most 4 evidence rows, with `v View` and `F1 Help` on the footer. A long multi-source error fixture shows labels intact, detail truncated with a marker, a count of sources not shown, and, in Status/Health/Retro, the full text reachable through `e` then `d`. A below-80x24 fixture truncates without panic. | Test |
| FR-065-AC-6 | The identity row (lead identity, binding state, activity markers, or the explicit no-lead and binding-diagnostic text) is identical across all six views for each of: a verified lead; a missing or ambiguous lead with explicit scope; a replaced session; explicit scope with no `--lead`; and a saved-report startup. | Test |
| FR-065-AC-7 | One fixture per label case asserts the exact caption. Tickets and Scope: Focused, whole-team fallback, ticket-only and named mode (per FR-060 and FR-064), and no lead bound. Agents: every reported assignment listed, not a scope. Status, Health and Retro: explicit team and projects, saved report, and lead-only startup with the derived-from-lead-workspace label. With department and lead populations differing, all three department views show the fixed text and no label contains "equal", "same", "narrower" or "subset". Inspection confirms no scope comparison or calculation was added. | Test and Inspection |
| FR-065-AC-8 | Status shows the specified purpose line at 80x24 and 120x40 without hiding evidence rows. Every Status row other than that line is unchanged by it for a shared fixture. | Test |
| FR-065-AC-9 | Health and Retro fixtures show exactly one message each for: no configuration (live), saved report lacking the fields, configuration supplied before its first read, a failed read with no model, and a stale retained model. Not configured and Waiting do not appear in source-failure rows or the `e` list, are not styled as errors, and are not called authentication failures; Retro names the history requirement; the live message names the label's population. No zero, green or complete value appears for missing data. | Test |
| FR-065-AC-10 | In the saved-report case, fail-if-called credential, network, Herdr and Redis adapters record no call while Health or Retro shows Not configured and while either popup is opened and closed. In the live case, with worker clocks held, adapter call counts, files and report, tracker and history stores are equal before and after the same interaction, and no authentication retry occurs. | Test |
| FR-065-AC-11 | With help open at 80x24, a refresh that fails, a rate-limit countdown and a session replacement each appear in the evidence rows and the footer while the popup stays within the work area. A refresh that shrinks rows while a popup is open clamps selection as FR-059 and FR-060 state, and closing the popup leaves exactly that state. Source polling continues while it is open. | Test |
| FR-065-AC-12 | The fixtures of FR-048-AC-1, FR-059, FR-060 and FR-064-AC-1/AC-13, quit and terminal restoration pass with differences only from the Permitted presentation differences list. The selector, dropdown, help and footer key hints use the primary blue, and descriptions and separators are muted. | Test |
| FR-065-AC-13 | Inspection of the terminal setup shows it is unchanged and enables no mouse capture. The click behavior is verified only when a later change delivers mouse events. | Inspection |

## Dependencies

- [FR-048](./FR-048-session-work-monitor.md), the views, Tickets/Agents/Scope keys
  and selection reset; [FR-059](./FR-059-department-monitor-adapter.md), the
  department views, startup modes and source diagnostics;
  [FR-060](./FR-060-focused-scope-presentation.md), Scope labels and filter footer.
  These are delivered and are the prerequisites.
- [dev-team FR-004](ix://agent-ix/dev-team/FR-004) owns the Status, Health and Retro
  contents. FR-065 changes none of them and opens no upstream change.
- [FR-064](./FR-064-named-focus-dashboard-reader.md) is not a prerequisite. FR-065
  only coordinates its presentation, and the `f` key is verified by FR-064.
- Out of scope: metric formulas, new telemetry, credential or setup changes,
  named-focus activation or adoption, changes to tracker initiatives, source
  timing, a new renderer or framework, dependency repins and any other service.
