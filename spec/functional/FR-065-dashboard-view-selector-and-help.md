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
    type: "depends_on"
---
# FR-065: One View selector and F1 help across the dashboard's six views

## Description

The existing `team-leader-dashboard` SHALL offer one View selector that lists all
six existing views (Tickets, Agents, Scope, Status, Health, Retro) and one F1 help
popup, identical in every view. Detailed key lists move from the persistent
screen into help. This requirement changes navigation and labeling only. It adds
no data source, collector, telemetry, metric, classification, scope calculation,
credential or setup behavior, and leaves source polling, binding fences, the
`ix-cli-kit` terminal lifecycle and Ratatui unchanged. The six views, their data
and their selection, scroll and detail semantics remain owned by
[FR-048](./FR-048-session-work-monitor.md),
[FR-059](./FR-059-department-monitor-adapter.md) and
[FR-060](./FR-060-focused-scope-presentation.md).

### Precedence

For presentation of the view switch only, this requirement takes precedence over
the following statements, and only to the extent listed:

- FR-048-AC-4 "The selected tab is underlined, including its hotkey": there is no
  persistent tab strip; the current view is named by the View selector.
- FR-059-AC-4 "Existing compact Tickets/Agents/Scope fixtures ... retain their
  outputs": those fixtures change only in the header selector row, the footer and
  the Status/Health/Retro labels stated here.
- FR-059 and FR-048 statements that name `1`, `2`, `3`, `t`, `a`, `p`: these keys
  remain, as compatibility shortcuts with the same actions.

Everything else in those requirements, including the Scope footer filter text of
FR-060 and Escape quitting when no popup is open, is retained.

## Behavior

### View selector

- Every view SHALL show, in its header, the same selector labelled `View` with the
  current view name and the key that opens it. The selector SHALL NOT depend on the
  view, on data availability or on startup mode. Selecting a view whose data is
  unavailable is allowed; that view shows its own unavailable or empty state.
- The key `v` (no modifier) SHALL open a dropdown anchored below the selector. It
  collides with no existing monitor key. The dropdown lists all six views in the
  fixed order Tickets, Agents, Scope, Status, Health, Retro, each with its
  compatibility key. The highlight starts on the current view.
- While the dropdown is open, Up, Down, `j` and `k` move the highlight (clamped,
  no wrap); Enter selects the highlighted view and closes the dropdown; Escape and
  `v` close it. Escape SHALL NOT quit and SHALL NOT change the view, selection,
  scroll or details state.
- Selecting a different view has exactly the effect of that view's compatibility
  key (FR-059: preserve scope, reset row selection and scroll). Selecting the
  current view closes the dropdown and changes nothing.
- This requirement adds no mouse capture mode. If the existing terminal lifecycle
  delivers mouse events, a click on the selector SHALL open the dropdown, a click
  on an entry SHALL select it as Enter does, and a click elsewhere SHALL cancel as
  Escape does. When mouse events are not delivered, keyboard access is complete.
- If the terminal is too small for the whole dropdown, the list SHALL scroll to
  keep the highlight visible. All six entries are reachable at 80x24.

### Help popup

- F1, or `?`, SHALL open a help popup from every view. F1 and Escape close it
  without quitting and without changing any view state.
- Help SHALL list, with the views where each applies:
  - view access: `v` selector and the compatibility keys `t`, `a`, `p`, `1`, `2`,
    `3`;
  - navigation: Up, Down, `j`, `k`, PageUp, PageDown;
  - details: `d` and Enter toggle details; Left and Right scroll department
    details;
  - refresh: `r`;
  - filter and focus: `c` in Scope (completed projects), and `f` where named
    focuses are active;
  - diagnostics: `s` source timestamps and `e` source errors;
  - quit: `q` and Escape when no popup is open;
  - help itself: F1 and `?`.
- Help SHALL mark the keys that apply in the current view. If help is taller than
  the terminal, Up, Down, PageUp and PageDown scroll it; every line is reachable at
  80x24.

### Modal isolation

- At most one popup is open. While one is open, all key and mouse input is
  consumed by it. No key reaches the table, details, refresh, source toggles, view
  keys, the Scope filter or quit. A key that would open the other popup is ignored,
  except that F1 closes the help popup.
- Closing a popup by Escape leaves the underlying view, selection, scroll, details
  and filter state as they were before it opened.
- Source polling, refresh results and the terminal lifecycle continue while a popup
  is open. Newly arrived data and errors appear behind it and, if applicable, in
  the footer. Terminal resize re-lays the popup out and keeps it open.

### Header identity and footer

- The identity row SHALL show the same monitored lead identity, binding state and
  activity markers in all six views. Switching views SHALL NOT change them. With no
  lead, it says so plainly and keeps any binding diagnostic. Identity is not
  inferred from the selected view.
- The normal footer SHALL be one compact row that always shows `v View` and
  `F1 Help` in every view, and `q Quit` when width allows. It SHALL NOT list other
  shortcuts, except the Scope filter key and mode that FR-060 requires.
- Source, stale, error, last-snapshot, rate-limit countdown, binding and filter
  evidence SHALL be shown outside any popup, on their own row or rows, and SHALL
  NOT be shortened, hidden or replaced by the controls footer. When the terminal is
  too narrow for everything, this evidence keeps priority over `q Quit`, and
  `v View` and `F1 Help` are never dropped.

### Scope labels

- Tickets, Agents and Scope SHALL name their population as the lead's reported
  scope, using the FR-060 labels unchanged.
- Status, Health and Retro SHALL name their population as the department
  selection: the explicit team and projects, or the saved report's scope. They
  SHALL state that this is department scope and may differ from the lead's
  reported focus, which is shown in Scope. They SHALL NOT call a department
  population the lead's focus, and SHALL NOT compute or assert that the two are
  equal or narrower. No scope is calculated for this label.
- Status SHALL show the plain-language purpose line "Status: progress, blockers and
  lane activity for the selected department scope." It may wrap at 80 columns.

### Empty Health and Retro

- When no retro configuration or saved report supplies Health or Retro fields, the
  view SHALL show one concise state, not a table of placeholders: first the state
  (not configured), then the requirement. For a live startup, it names
  `--retro-config <file>` whose team and project populations match the selectors;
  for Retro it also names that previous and baseline comparisons need a retained
  snapshot history. For a saved report, it says the report does not supply the
  fields and that a report containing them is required.
- Not configured, failed and stale remain distinct. A failed read shows the
  classified source reason; a stale source keeps its stale label and its retained
  values. None of them is shown as zero, green or complete.
- The empty state starts no collection, retries no authentication and writes
  nothing.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-065-AC-1 | In each of Tickets, Agents, Scope, Status, Health and Retro, at 80x24 and 120x40, the header shows the same `View` selector; `v` opens a dropdown listing all six views with their keys; Down/Up/`j`/`k` then Enter selects and the target view renders with selection and scroll reset and scope preserved. Selecting the current view changes nothing. | Test |
| FR-065-AC-2 | Escape or `v` with the dropdown open closes it and leaves view, selection, scroll and details unchanged, and does not quit; Escape with no popup open still quits. Highlight is clamped at both ends. A dropdown taller than the terminal keeps the highlight visible and all six entries reachable. | Test |
| FR-065-AC-3 | F1 and `?` open help in all six views; F1 and Escape close it without quitting or state change. Help lists every key in the Behavior list with applicable views, marks the current view's applicable keys, and every line is reachable at 80x24 by scrolling. `f` appears only where named focuses are active. | Test |
| FR-065-AC-4 | With a popup open, a fixture sending every existing key (`q`, `r`, `d`, `Enter`, `s`, `e`, `t`, `a`, `p`, `1`, `2`, `3`, `c`, `f`, `j`, `k`, arrows, PageUp, PageDown) leaves table selection, scroll, details, view, refresh flag, status toggle and filter mode unchanged, and does not quit; unclaimed keys do not open the other popup. If mouse events are delivered, clicks outside the dropdown cancel it and never reach the table. | Test |
| FR-065-AC-5 | The footer in all six views contains `v View` and `F1 Help` and no other shortcut list except the Scope filter key and mode required by FR-060, and no persistent tab strip appears. At 80x24 with a source error, a stale source, a rate-limit countdown and an active filter present, all of that evidence remains on screen outside the popup and `v View` and `F1 Help` remain visible. | Test |
| FR-065-AC-6 | The identity row (lead identity, binding state, activity markers, or the explicit no-lead and binding-diagnostic text) is identical across all six views for each of: verified lead, missing or ambiguous lead with explicit scope, and replaced session. | Test |
| FR-065-AC-7 | A fixture where the department team/projects and the lead's reported scope differ renders Status, Health and Retro labeled as department scope, with the statement that it may differ from the lead's reported focus, and Tickets, Agents and Scope labeled as the lead's reported scope. No department population is called lead focus; a saved-report fixture labels the report's scope. Inspection confirms no scope comparison or calculation was added. | Test and Inspection |
| FR-065-AC-8 | Status shows the specified purpose line at 80x24 and 120x40 without hiding source or error evidence. | Test |
| FR-065-AC-9 | Health and Retro fixtures with no configuration, with a saved report lacking the fields, with a failed source and with a stale retained model show the not-configured requirement text, the report-lacks-fields text, the classified failure, and the stale-labeled retained values respectively. No zero, green or complete value appears for missing data; Retro names the history requirement. Adapters for credentials, network, Herdr and Redis record no call caused by the empty state or by opening either popup. | Test |
| FR-065-AC-10 | Existing FR-048-AC-1 binding, FR-059 source-failure, replacement-fence, quit, terminal-restoration and FR-060 scroll/selection/toggle fixtures pass with only the header, footer and label differences stated above. An open popup while a source refresh completes or fails keeps the popup open and updates the evidence behind it. | Test |

## Dependencies

- [FR-048](./FR-048-session-work-monitor.md), the views, Tickets/Agents/Scope keys
  and selection reset; [FR-059](./FR-059-department-monitor-adapter.md), the
  department views, startup modes and source diagnostics;
  [FR-060](./FR-060-focused-scope-presentation.md), Scope labels and filter footer.
  [FR-064](./FR-064-named-focus-dashboard-reader.md), the `f` key when named
  focuses are active.
- Out of scope: metric formulas, new telemetry, credential or setup changes,
  named-focus activation or adoption, changes to tracker initiatives, source
  timing, a new renderer or framework, dependency repins and any other service.
