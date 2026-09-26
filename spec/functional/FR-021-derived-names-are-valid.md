---
id: FR-021
title: "Derived bus names are always valid"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-003"
    type: "implements"
---
# FR-021: Derived bus names are always valid

## Description

When the name deriver derives a bus name from any workspace label, tab label, tab
id and pane id herdr can report, the name deriver shall produce a name that
matches `[a-z][a-z0-9_-]{0,31}` and is not `all`.

## Behavior

- The name deriver shall build the name from the workspace label; then the tab
  label when the tab is labelled, or `t<tab id>` when the tab is unlabelled and not
  the workspace's first tab; then `p<pane id>` when the tab holds more than one
  pane.
- The name deriver shall use herdr's never-reused tab and pane id numbers, not
  herdr's display numbers.
- The name deriver shall lowercase the text, map every character outside
  `[a-z0-9_]` to `-`, collapse runs of `-`, trim leading and trailing `-`, use
  `lead` for an empty result, prefix `a` when the result does not start with a
  letter, cap the length at 32 characters, and append `-lead` to a result equal to
  `all`.
- If the tab's label starts with `esc-`, then the name deriver shall produce no
  name.
- The chosen name shall follow the precedence explicit name > Claude `/rename`
  title > herdr agent name > herdr location name; a location name that differs
  from the chosen name stays reachable as an alias.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-021-AC-1 | Every slug of any text parses as a bus name, is at most 32 bytes long and is not `all`. | Test |
| FR-021-AC-2 | Every name derived from any workspace label, tab label, tab id, pane id, first-tab flag and lone-pane flag is a valid bus name, or is absent exactly when the tab label starts with `esc-`. | Test |
| FR-021-AC-3 | Given workspace `QSL`: the first unlabelled tab's lone pane is `qsl`; tab `review` is `qsl-review`; an unlabelled non-first tab with id 3 is `qsl-t3`; a second pane with id 5 in that tab is `qsl-t3-p5`. | Test |
| FR-021-AC-4 | Given the texts `""`, `---`, `1lead`, `ALL` and 50 `x` characters, slugify yields `lead`, `lead`, `a1lead`, `all-lead` and 32 `x` characters. | Test |
| FR-021-AC-5 | Every slug of any text is unchanged when slugified again. | Test |

## Dependencies

- **Upstream**: [US-003](../usecase/US-003-be-reachable-without-configuring-a-name.md)
- **Downstream**: [FR-022](./FR-022-name-collisions-resolve-to-a-free-name.md)
- Delivered by AGE-2018.
