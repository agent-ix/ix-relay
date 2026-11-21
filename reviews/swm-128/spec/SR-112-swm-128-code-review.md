---
id: SR-112
title: "Code review (spec delta) - credential phase before bounded focus read, FR-061 and FR-063 (SWM-128)"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay; spec/functional/FR-061-named-focus-definitions.md; spec/functional/FR-063-shared-focus-membership-and-summaries.md"
review_set: subset
---

## Summary

Ticket: SWM-128. Branch codex/swm-128-auth-read-budget at immutable source d7ad96a (base accepted main 0d62c93). Code-review method over a Markdown-only delta: the FR-061 bounded-read deadline bullet and FR-061-AC-20, and the FR-063 bounded-read bullet. The change moves the read-deadline clock from "start of the command" to "after the credentials are ready, before the cache lookup, the read and the selection validation", and says the normal operating-system keychain wait is not bounded. No finding. The 120-second default, the 1..600 range, the five-minute cache age, `source_unavailable` reasons, exit status 1, no publication on failure, no detached read and the SWM-178 ownership statement are unchanged.

## Verdict

**PASS** - no finding. The delta states one clock start in three places (FR-061 deadline bullet, FR-061-AC-20, FR-063 bounded-read bullet) and the three agree with each other; no second statement of the SDK contract was added.

## Findings

| ID | Severity | Summary | Refs |
|---|---|---|---|
| FND-001 | low | No findings (placeholder) | - |

## Review Context

Reviewer model claude-sonnet-5-5; harness session ID cc4870fa-138a-459e-b41c-c2e001e7722a (root verified it from the finished result JSON); native run id unavailable. Root-supplied marker metadata: repository visibility public (confirmed by the root with gh), quoin 0.36.7, module spec-artifacts-process@v0.29.2, PR ix-relay#41. Marker id=SR-112 method=code-review lang=spec. No Rust, Cargo or test file is in the diff, so rust-review, gap-analysis and `quire matrix` have nothing to run. No AssuranceProfile was looked up under spec/ beyond the repo guidance (CLAUDE.md); none is claimed to apply.

What was and was not run: I read the installed dev-team reviewer, dev-tools code-review and quoin spec-review skills, repo CLAUDE.md and AGENTS.md, the brief, and the author's source-ready note. I had no Bash, Git or Quire CLI and computed no diff myself. In a resumed pass the root supplied the actual `git diff origin/main...HEAD` verbatim at /Users/peter/dev/.dev-dept-recovery/swm-128/auth-phase-source.diff, and I read that file. It changes exactly two files, FR-061 (the deadline bullet, hunk at line 144, and FR-061-AC-20, hunk at line 418) and FR-063 (the bounded-read bullet, hunk at line 159), and nothing else. My clean verdict stands on that actual delta. The root reports Quire validation of SR-112 and of both changed files with exit 0; those runs are the root's, not mine. I ran no Jev client and no criterion-strength analysis.

Actual-diff observations: (1) the old FR-061 text read "120 seconds from the start of the command by default" and "the command returns within the deadline plus one request timeout"; both are replaced as described in Summary, and no other line of FR-061 changed except AC-20, which adds "measured from the point the credentials are ready". (2) The FR-063 hunk also changes "a timeout publishes nothing" to "a timeout or any validation failure publishes nothing". That is a strengthening that matches FR-061 (no store write before every reference validates) and the author's brief ("validation failure always no publication"); it adds no new behavior and no contradiction, so it is not a finding. (3) The diff touches no limit, default, range, cache age, reason list, exit status or ownership sentence.

Duplication check (P0): searched `spec/` for "start of the command", "read-timeout", "120 s", "within the deadline" and "never waits". The clock-start statement appears in FR-061 (deadline bullet, AC-20) and FR-063 (bounded-read bullet), each pointing to or consistent with FR-061 as the owner; FR-063-AC-15 says "bounded as in FR-061" and restates nothing. The remaining old-clock candidates in other documents are unrelated (FR-020, FR-029, FR-039, FR-045, NFR-001 deadlines). No FR-062 or FR-064 text states a read bound or a command-start clock.

Unrequested ceremony check: no manifest, ledger, pin or new gate was added.

## Examined Scope

```yaml
scope:
  - {id: "FR-061 deadline bullet", path: "spec/functional/FR-061-named-focus-definitions.md", role: examined, excerpt: "The read has an overall deadline: 120 seconds by default, adjustable with\n  `--read-timeout <seconds>` from 1 to 600. The deadline clock starts after the\n  credentials are ready and before the cache lookup, the read and the selection\n  validation. Obtaining credentials is outside the deadline: the normal\n  operating-system keychain flow can wait for user interaction before a client\n  exists, and the deadline does not bound that wait."}
  - {id: "FR-061-AC-20", path: "spec/functional/FR-061-named-focus-definitions.md", role: examined, excerpt: "Given a source that never completes, `create` exits with status 1 and `source_unavailable` with reason `timeout` within the deadline plus one request timeout, measured from the point the credentials are ready (the default and a `--read-timeout` of 1 and 600 both honored), starts no background read, and leaves the store byte-identical."}
  - {id: "FR-063 bounded read bullet", path: "spec/functional/FR-063-shared-focus-membership-and-summaries.md", role: examined, excerpt: "A snapshot read can be bounded by cache age and an operation-wide deadline and\n  returns a typed timeout, so once the credentials are ready a command or a\n  refresh never waits unboundedly on the read, and a timeout or any validation\n  failure publishes nothing. The deadline clock starts after the credentials are\n  ready; obtaining them is outside it."}
  - {id: "FR-063-AC-15", path: "spec/functional/FR-063-shared-focus-membership-and-summaries.md", role: context_only, excerpt: "`agent-msg focuses show <focus> --members` prints the rows and states of the current revision from one snapshot read bounded as in FR-061, stores nothing, and with a failed read prints the definition with `source_unavailable` and its reason and exit status 1."}
  - {id: "FR-061 ownership bullet", path: "spec/functional/FR-061-named-focus-definitions.md", role: context_only, excerpt: "The bounded shared read is an **explicit implementation prerequisite** of\n  FR-061-AC-19 and FR-061-AC-20 and of FR-063-AC-15, carried by the single ix-board\n  specification ticket SWM-178 and by ix-board FR-004 (cache and polling)."}
  - {id: "FR-064 responsiveness", path: "spec/functional/FR-064-named-focus-dashboard-reader.md", role: context_only, excerpt: "Scroll, selection, view switching, `f`, `c` and quit SHALL NOT wait on a source\n  read or a computation."}
```

## Verdict notes

- Contradiction: none. FR-061 says the command "returns within the deadline plus one request timeout" only "once the credentials are ready", FR-061-AC-20 measures from the same point, and FR-063 uses the same phrase; the old "from the start of the command" wording is gone from all of `spec/`.
- Ambiguity: none introduced. The phrase "before the cache lookup, the read and the selection validation" fixes where the clock starts relative to the three SDK steps, and "outside the deadline" is stated for credentials only.
- Verifiability: AC-20 names its measurement point, so a test with an injected credential-ready point and a source that never completes has a defined start. I did not review test design; none exists yet and none is claimed.
- Boundary: the relay still defines no SDK API, adds no auth workaround and starts no read that outlives the command; the FR-061 ownership and prerequisite text for SWM-178 and ix-board FR-004 is untouched. FR-004/010/012/016 ownership is unchanged.
- Preserved: 120 s default and 1..600 range, five-minute cache, `source_unavailable` reasons `missing|stale|partial|failed|timeout`, exit status 1, no store write before validation, the one atomic write outside the deadline (FR-061 lines 158-163, unchanged). FR-063 keeps "a timeout or any validation failure publishes nothing".
- Non-blocking layout note, not a finding (confirmed against the actual diff, added line 52): FR-063 line 165 (`ready; obtaining them is outside it. The owning lane verified that the current shared read`) is a longer line than the rest of the wrapped bullet. It has no effect on meaning or rendering.

## Verdict after disposition pass 1

Not run. No finding exists to dispose.
