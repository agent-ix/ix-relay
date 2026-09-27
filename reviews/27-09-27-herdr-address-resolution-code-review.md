---
id: SR-044
title: "Code and Rust review — Herdr-authoritative address resolution"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay@3998dfc820abcd6a0d1f3ae403c367ab89df45c6; changed address and registration model (crates/relay-core/src/address.rs, message.rs, resolve.rs, error.rs); Herdr join and watcher refresh (crates/relay-ops/src/find.rs, naming.rs; crates/relay-watcher/src/herdr.rs, relink.rs, main.rs); CLI lookup/doctor (crates/agent-msg/src/find.rs, identity.rs, main.rs, doctor/); address, placement, relink and watcher tests; FR-001/002/003/005/006/009/033/035/047 and spec/tests.md; plugin address help and workflow diff"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay/FR-047"
    type: reviews
---

## Summary

Reviewed PR #2 at `3998dfc820abcd6a0d1f3ae403c367ab89df45c6`, including its Herdr
identity join, address resolution, cache refresh, CLI surfaces, tests, and spec changes.
No code or Rust design finding was identified in the reviewed scope; the requested server,
workspace, tab, and pane lookup behavior is explicit and covered by tests.

## Verdict

**PASS** — Linux formatting, all-features Clippy, touched-crate tests, and cargo-deny
policy checks passed on the exact pushed revision. The macOS sandbox run has unrelated
socket/OTLP test failures; the full Linux watcher suite passes 297/297.

## Findings

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-001 | low | No findings (placeholder) | - |

## Review notes

### Code review

- `server:workspace[:tab[:pane]]` is a prefix query over verified, registered
  recipients. A missing tab or pane resolves when exactly one registered lead matches
  and returns explicit ambiguity candidates when multiple leads match. Empty Herdr panes
  and unregistered Herdr agents cannot become recipients.
- The Herdr `agent get` response supplies the harness session identity and location
  together. The watcher requires an exact session/pane join before publishing a
  routable place; unresolved, conflicting, or legacy label-only records are not address
  targets.
- `find`, displayed addresses, and send resolution use the same verified placement
  model. Repeated panes no longer select the first registration. The watcher invalidates
  cached placement when a successful current layout cannot prove it.
- The relevant source and test changes contain no new TODO/FIXME/XXX placeholders,
  `todo!`, `unimplemented!`, or debug prints. No workflow lane was removed or narrowed.
- The repository has `spec/tests.md` but no targeted `plan/<id>-<slug>/plan.md`; the
  separate plan-based gap-analysis skill therefore could not verify task completion or
  produce its required artifact in this run.

### Rust review

- The placement enum is derived from registration evidence rather than serialized as a
  second source of truth. Redis compatibility decoding drops incomplete legacy Herdr
  identity, and routing accepts only `Verified` placement.
- Resolution and Herdr joins are pure over typed snapshots; Herdr/Redis effects remain
  behind the existing operations and store boundaries. No new `unsafe`, warning
  suppression, production panic path, or unbounded input loop was found in the reviewed
  changes.
- New address and placement tests use real domain logic and cover sessionless claims,
  mismatched sessions, stable Herdr IDs, ambiguous recipients, and address round trips.

### Gates

On Linux Luna, exact revision `3998dfc820abcd6a0d1f3ae403c367ab89df45c6`:

| Gate | Result |
|------|--------|
| `cargo fmt --all -- --check` | PASS (nightly-only rustfmt options warn under stable) |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | PASS |
| `cargo test -p relay-core -p relay-ops -p agent-msg -p relay-watcher` | PASS; watcher 297 passed, 0 failed; startup 13 passed, 0 failed; no test failures in touched crates |
| `cargo deny check` | Policy result: advisories, bans, licenses, and sources all OK; cargo-deny emits existing unresolved-workspace-dependency diagnostics for workspace path aliases |

The same package test command on macOS completed core and CLI tests but reported 28
watcher socket/OTLP test failures. The failures are not reproduced on Linux; relevant
address-resolution tests pass on both platforms. No source change was made in response
to those host-specific results.
