---
id: SR-053
title: "Optional status and footer hotkey code and Rust review"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay@208ca9b44b3377a4a6d57fe69eeea5e8a8634f6b; crates/relay-tui/README.md, crates/relay-tui/src/monitor.rs, crates/relay-tui/src/monitor_ui.rs, spec/functional/FR-048-session-work-monitor.md; diff against 9a5c16d1514359e3a430d33218367329a04a4ca0"
review_set: subset
---
## Summary

Reviewed the full PR diff with repository conventions and the Rust checklist. No CI or source polling logic changed. The default App value hides source ages. The s key toggles that display state, and the layout reserves one extra row only when requested. Errors are assembled independently from the display toggle and retain their error color. Hotkeys use distinct primary spans; descriptions and separators inherit the muted paragraph style. Text remains sanitized and bounded. No new transport, external payload, compatibility mechanism or copied dependency source was added.

## Verdict

**PASS** — no findings. Eighteen dashboard tests pass. The new test measures each footer character and its rendered color, including error color. The status test applies the actual key handler, measures ages hidden/shown/hidden, and checks errors stay visible with status closed. Full gate evidence is recorded in the PR.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | low | No findings (placeholder) | - |

## Evidence limitations

The installed quire lacks matrix. The scoped requirement retains its pre-existing description EARS warnings. Remote Terra connectivity is independent of this display-only change and is not claimed as verified.
