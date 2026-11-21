---
id: SR-113
title: "Failure-domain analysis - credential phase and read-clock boundary, FR-061 and FR-063 (SWM-128)"
type: SpecReview
analysis: failure-domain
scope: "agent-ix/ix-relay; spec/functional/FR-061-named-focus-definitions.md; spec/functional/FR-063-shared-focus-membership-and-summaries.md"
review_set: subset
---

## Summary

Ticket: SWM-128. Branch codex/swm-128-auth-read-budget at immutable source d7ad96a (base main 0d62c93). Failure-domain analysis of the one changed boundary: the deadline clock for the bounded snapshot read now starts after the credentials are ready, and the credential wait (which can include operating-system interaction) is explicitly outside it. No finding. The changed statement keeps every failure outcome that the accepted contract defined (typed `timeout`, `source_unavailable`, exit status 1, no store write or publication, no background read) and adds no auth behavior.

## Verdict

**PASS** - no finding. The new wait is named, placed outside the deadline, and not given a bound it cannot keep; the failure outcomes after the boundary are unchanged.

## Findings

| ID | Severity | Summary | Refs |
|---|---|---|---|
| FND-001 | low | No findings (placeholder) | - |

## Review Context

Reviewer model claude-sonnet-5-5; harness session ID cc4870fa-138a-459e-b41c-c2e001e7722a (root verified it from the finished result JSON); native run id unavailable. Root-supplied marker metadata: repository visibility public (confirmed by the root with gh), quoin 0.36.7, module spec-artifacts-process@v0.29.2, PR ix-relay#41. Marker id=SR-113 method=spec-review/failure-domain lang=spec. This is the one spec-review analysis chosen for this tiny delta, by the reviewer's judgement of what the change touches. The quoin spec-review `base` checklist, ears-conformance, integrity, dependency, evidence, risk-complexity and scope-boundary analyses were not run as separate documents. Cross-reference consistency and ownership boundary were checked by reading inside this document's scope only and are stated below as manual checks, not as separate analyses. No ix-flow enforced run was started and no `review_doc` was registered; this is a direct review. No Jev client or criterion-strength analysis ran. I had no Bash or Quire CLI and executed none. In a resumed pass I read the actual `git diff origin/main...HEAD` that the root supplied at /Users/peter/dev/.dev-dept-recovery/swm-128/auth-phase-source.diff (two files: the FR-061 deadline bullet and AC-20, and the FR-063 bounded-read bullet) and confirmed this clean verdict on it. One point visible only in the actual diff: FR-063 now says "a timeout or any validation failure publishes nothing", which matches FR-061's no-write-before-validation rule and adds no new failure behavior. The root reports Quire validation of SR-112 and the changed files with exit 0; those are the root's runs, and validation of this SR-113 file is still pending with the root. No AssuranceProfile was looked up beyond repo guidance; none is claimed to apply.

## Checks (failure-domain checklist applied to the change)

1. Extension points and trust boundary. The credential source (the operating-system keychain, outside the relay and outside the SDK read) is the only new boundary the text names. Its failure behavior inside the relay's command is not given a new rule: the FR-061 text keeps that a failed read refuses with `source_unavailable` and exit status 1 and that no store write precedes validation, so any command that does not reach a validated reference changes nothing. The relay is not said to read, copy or bypass credentials, and no ACL, signing or profile change appears.
2. Identity. Not touched; no entity key changed.
3. Evaluation purity. The deadline is enforced at the source boundary of the shared read, which returns the typed timeout; the relay still runs no uncancellable thread and starts no read that outlives the command. The credential phase begins no read, so an interrupted credential wait leaves nothing in flight.
4. Termination and boundedness. Previously the whole command was bounded from its start. The text now bounds only the post-credential phase and states the credential wait is not bounded by the deadline. That is an honest statement of a fact the owner verified (the normal keychain flow can wait for the user), not a lost guarantee the implementation could have kept: the SDK owner under SWM-178 bounds read, cache and selection validation only after credentials are ready. The "within the deadline plus one request timeout" promise is qualified by "once the credentials are ready" in FR-061 and by "measured from the point the credentials are ready" in AC-20, so no sentence still promises the old bound.
5. Fail-safe outcomes. Unchanged: a timeout or any validation failure publishes nothing (FR-063) and leaves no definition, index, name or pin changed (FR-061). The single atomic write stays outside the deadline.
6. Dashboard interaction. FR-064 keeps scroll, selection, view switching, `f`, `c` and quit off the source read and recompute path, so a pending credential wait during a refresh does not block input; FR-063 now says only that, once credentials are ready, a command or refresh does not wait unboundedly on the read.

Manual cross-reference checks: the old "from the start of the command" wording is absent from all FR-06x files and `spec/`; FR-061, AC-20 and FR-063 name the same clock start; FR-063-AC-15 refers to FR-061 rather than restating it; the SWM-178 and ix-board FR-004 prerequisite text and the "blocked until the bounded shared read exists" statements on AC-19, AC-20 and FR-063-AC-15 are unchanged.

## Examined Scope

```yaml
scope:
  - {id: "FR-061 deadline bullet", path: "spec/functional/FR-061-named-focus-definitions.md", role: examined, excerpt: "The deadline clock starts after the\n  credentials are ready and before the cache lookup, the read and the selection\n  validation. Obtaining credentials is outside the deadline: the normal\n  operating-system keychain flow can wait for user interaction before a client\n  exists, and the deadline does not bound that wait. The deadline is enforced at\n  the source boundary of the shared read, which returns a typed timeout; the\n  relay does not run the read on a thread it cannot cancel and starts no read that\n  outlives the command."}
  - {id: "FR-061 failure outcome bullet", path: "spec/functional/FR-061-named-focus-definitions.md", role: examined, excerpt: "A failed read refuses with `source_unavailable`, whose `reason` is one of\n  `missing`, `stale`, `partial`, `failed` or `timeout`, naming the source, and\n  exits with status 1. The relay performs no store write before resolution has\n  finished and validated every reference."}
  - {id: "FR-061-AC-20", path: "spec/functional/FR-061-named-focus-definitions.md", role: examined, excerpt: "Given a source that never completes, `create` exits with status 1 and `source_unavailable` with reason `timeout` within the deadline plus one request timeout, measured from the point the credentials are ready (the default and a `--read-timeout` of 1 and 600 both honored), starts no background read, and leaves the store byte-identical."}
  - {id: "FR-063 bounded read bullet", path: "spec/functional/FR-063-shared-focus-membership-and-summaries.md", role: examined, excerpt: "so once the credentials are ready a command or a\n  refresh never waits unboundedly on the read, and a timeout or any validation\n  failure publishes nothing. The deadline clock starts after the credentials are\n  ready; obtaining them is outside it."}
  - {id: "FR-064 responsiveness", path: "spec/functional/FR-064-named-focus-dashboard-reader.md", role: context_only, excerpt: "Scroll, selection, view switching, `f`, `c` and quit SHALL NOT wait on a source\n  read or a computation."}
  - {id: "FR-061 ownership bullet", path: "spec/functional/FR-061-named-focus-definitions.md", role: context_only, excerpt: "The relay defines no SDK API for it, adds no raw Linear client and runs\n  no detached thread it cannot cancel. Until that support exists these criteria\n  are blocked, not satisfied by relay-side workarounds."}
```

## Verdict notes

An open question outside this delta, not a defect of it: the accepted FR-061 text has never named which `source_unavailable` reason a credential failure (denied prompt, locked or missing keychain item) reports. That was equally unspecified before this change, the changed text neither introduces nor worsens it, and the author's brief asks for no new auth behavior. The SWM-178 or SWM-128 implementation owner can decide it when the credential-failure path is built; it is recorded here only so it is not lost, and it is not a finding.

## Verdict after disposition pass 1

Not run. No finding exists to dispose.
