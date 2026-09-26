---
id: FR-015
title: "Send refuses control characters"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-002"
    type: "implements"
---
# FR-015: Send refuses control characters

## Description

If the text of a message to be sent contains an unsafe character under the shared
rule below, then the store shall refuse the send with an unsafe-text error naming
the code point and its byte offset, and shall queue and log nothing.

## Behavior

- The store shall treat as unsafe: every control character except `\n` and
  `\t` (C0, DEL and C1, including ESC, CR and NEL); the line and paragraph
  separators U+2028 and U+2029; the bidirectional controls U+061C, U+200E,
  U+200F, U+202A–U+202E and U+2066–U+2069; and the invisible format characters
  U+200B, U+180E, U+FEFF, U+2060–U+2064, U+FFF9–U+FFFB and the tag block
  U+E0000–U+E007F.
- The store shall treat as safe the joiners U+200C and U+200D, variation
  selectors, and U+FFFD.
- The store shall apply the same control-character rule to message text as to
  escalation text ([FR-025](./FR-025-escalation-text-safety.md)); one rule is
  shared by every check and every renderer in ix-relay.
- The refusal shall happen before any Redis write.
- When `resolve` builds the notification carrying a decision, `resolve` shall
  refuse a decision that contains an unsafe character before the escalation's
  status changes.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-015-AC-1 | Every send whose text contains a code point in U+0000–U+0008, U+000B–U+001F, U+007F or U+0080–U+009F is refused and leaves every inbox and the log unchanged. | Test |
| FR-015-AC-2 | Every send to a registered lead whose text is at most 4096 bytes of printable characters, `\n` and `\t` is accepted. | Test |
| FR-015-AC-3 | Given text `hi\r[ix-relay] from other: x`, the send is refused with a code point of 0x0d at byte offset 2. | Test |
| FR-015-AC-4 | Given text containing ESC (`\x1b[201~`), the send is refused. | Test |
| FR-015-AC-5 | Every send whose text contains U+2028, U+202E, U+200B or U+E0041 is refused, and every send whose text contains only printable characters plus U+200D is accepted. | Test |

## Dependencies

- **Upstream**: [US-002](../usecase/US-002-receive-messages-without-losing-the-turn.md)
- **Downstream**: [FR-016](./FR-016-render-neutralises-control-characters.md)
- Gap closed by AGE-2026: `relay_core::text` holds the one rule, and
  `Relay::send` refuses with `Error::UnsafeMessageText`.
- `Relay::notify_human` does not itself apply the rule; its two callers do
  (`escalate` builds the notification from validated fields, `resolve` checks
  the decision). Defence in depth at the store would make that hold by
  construction.
