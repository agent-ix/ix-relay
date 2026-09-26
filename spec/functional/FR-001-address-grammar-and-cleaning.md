---
id: FR-001
title: "Address grammar and segment cleaning"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-001"
    type: "implements"
---
# FR-001: Address grammar and segment cleaning

## Description

When text is parsed as an address, the address parser shall accept exactly the
forms `server:workspace` and `server:workspace:tab` with every segment non-empty,
and shall refuse every other text with an invalid-address error that carries the
input verbatim.

## Inputs

- Text typed by a sender, or a machine name and herdr workspace and tab labels
  captured at registration.

## Outputs

- An address with a server, a workspace and an optional tab; or an
  invalid-address error.

## Behavior

- The address parser shall split the input on `:` and accept two or three
  segments, each non-empty.
- The address builder shall replace every whitespace character, every control
  character and every `:` inside a segment with `-`.
- If a segment is empty after cleaning, then the address builder shall store the
  segment as `-`.
- The address builder shall normalise the server segment as a machine name:
  lowercased, and cut at the first `.`.
- The address printer shall print `server:workspace` when the address has no tab
  and `server:workspace:tab` when it has one.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-001-AC-1 | Given each of `""`, `hub`, `hub:`, `:EA`, `hub::1`, `hub:EA:` and `a:b:c:d`, parsing returns an invalid-address error whose input equals the given text. | Test |
| FR-001-AC-2 | Every address built from any server, workspace and optional tab text parses back from its printed form to an address equal to itself. | Test |
| FR-001-AC-3 | For every address built from any text, no printed segment contains whitespace, a control character or `:`, and no printed segment is empty. | Test |
| FR-001-AC-4 | Given the server text `Hub.local`, the built address's server is `hub`. | Test |
| FR-001-AC-5 | Given workspace `My Work\nspace` and tab `a:b` on server `hub`, the printed address is `hub:My-Work-space:a-b`. | Test |

## Dependencies

- **Upstream**: [US-001](../usecase/US-001-message-a-lead-by-where-it-sits.md)
- **Downstream**: [FR-003](./FR-003-address-matching-ignores-case.md),
  [FR-005](./FR-005-shown-address-rule.md)
- Delivered by AGE-2022.
