---
id: FR-025
title: "Escalation text is safe text"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-004"
    type: "implements"
---
# FR-025: Escalation text is safe text

## Description

If an escalation's context, any option, its recommendation, or a resolution
decision contains an unsafe character of the shared rule
([FR-015](./FR-015-send-refuses-control-characters.md)), then ix-relay shall
refuse the escalation or the resolution with an unsafe-text error naming the code
point and its byte offset, and shall write nothing.

## Behavior

- If an escalation field is refused, then the error shall also name the field
  (`context`, `option` or `recommendation`).
- The store shall check every text field before any Redis write, so a refused
  escalation is never partly stored.
- The `investigate --fresh` prompt shall be built only from the escalation id,
  the lead's bus name and the ticket id, never from lead-supplied text.
- The escalation commands shall pass no escalation text through a shell.
- Every renderer of escalation text shall apply
  [FR-016](./FR-016-render-neutralises-control-characters.md)'s neutralisation.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-025-AC-1 | Every escalation or resolution whose context, option, recommendation or decision contains a code point of the shared rule is refused with an unsafe-text error giving the code point and offset (and the field, for escalation fields), and leaves every escalation key unchanged. | Test |
| FR-025-AC-2 | Every escalation whose text fields hold only printable characters, `\n` and `\t` is filed, and `escalation show` prints each field back unchanged apart from indentation. | Test |
| FR-025-AC-3 | Every `investigate --fresh` prompt, for any escalation, contains none of that escalation's context, option or recommendation text. | Test |
| FR-025-AC-4 | Given a context containing U+009B (a C1 control), `escalate` is refused. | Test |

## Dependencies

- **Upstream**: [US-004](../usecase/US-004-escalate-a-decision.md)
- Delivered in part by AGE-2014 and AGE-2015.
- Gap closed by AGE-2026: `validate_safe_text` uses the shared rule, and
  `resolve` refuses an unsafe decision before the status changes.
