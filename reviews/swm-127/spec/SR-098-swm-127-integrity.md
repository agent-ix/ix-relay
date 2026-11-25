---
id: SR-098
title: "Structural and cross-requirement integrity of the SWM-127 named focus contract"
type: SpecReview
analysis: integrity
scope: "agent-ix/ix-relay; spec/functional/FR-061-named-focus-definitions.md; spec/functional/FR-062-session-active-focuses-and-adoption.md; spec/functional/FR-063-shared-focus-membership-and-summaries.md; spec/functional/FR-064-named-focus-dashboard-reader.md; spec/non-functional/NFR-004-prerelease-no-compatibility-layers.md; spec/functional/FR-008-tolerant-registry-decoding.md"
review_set: subset
---

## Summary

Ticket: SWM-127. PR ix-relay#32 at frozen 218a0ff3157d30022689b3c275fc72fd9e503961. Integrity of FR-061..064 against each other, FR-048, FR-060, FR-008 and NFR-004, plus the domain-object lens (spec-object-review has no `SpecReview.analysis` value in the installed schema, so its findings FND-006..FND-008 are carried here). Five cross-requirement contradictions and three object-definition gaps.

## Verdict

**CONDITIONAL** - the named-mode versus legacy-mode separation, empty-never-whole-team rule and FR-060 precedence list are coherent. Contradictions remain on stored-format tolerance, record expiry, partial marks, department views, and three undefined objects.

## Findings

| ID | Severity | Summary | Refs |
|---|---|---|---|
| FND-001 | medium | FR-061 rejects stored records with an unknown field and embeds a versioned ix-board selection, against FR-008 (unknown fields ignored) and NFR-004-AC-1 (no stored version field), with no stated precedence | spec/functional/FR-061-named-focus-definitions.md:197 |
| FND-002 | medium | The partial-mark reasons omit the team boundary, so an initiative row that drops other-team tasks shows part of the initiative as its whole progress | spec/functional/FR-063-shared-focus-membership-and-summaries.md:42 |
| FND-003 | medium | A live session's named record expires 30 days after its last write, and an absent record means legacy mode, which silently turns a named session into the whole-team fallback | spec/functional/FR-062-session-active-focuses-and-adoption.md:55 |
| FND-004 | medium | FR-064 says department views restrict to the named view's leaves, and later says departments have no named mode | spec/functional/FR-064-named-focus-dashboard-reader.md:92 |
| FND-005 | low | Typed error codes are enumerated for FR-061 only; FR-062 refusals `stale_revision`, `already_active`, `not_active` and the activation limit and archived reasons have no stable code list | spec/functional/FR-062-session-active-focuses-and-adoption.md:78 |
| FND-006 | medium | The adoption pin is used by FR-061 retention and FR-062 release but is never defined as an object: key, owner of the write, fields, and the release owner | spec/functional/FR-062-session-active-focuses-and-adoption.md:97 |
| FND-007 | medium | The active record holds up to 16 full snapshots of up to 32 KiB (512 KiB) with no bound, while the spec cites the existing 64 KiB per-field read bound | spec/functional/FR-062-session-active-focuses-and-adoption.md:48 |
| FND-008 | low | The owner record's "last verified binding" has no named fields, so refreshing and comparing it cannot be tested | spec/functional/FR-061-named-focus-definitions.md:117 |

## Review Context

Reviewer model claude-sonnet-5-5; harness session ID unavailable; run id unavailable. Linear marker id=SR-098 method=spec-review/integrity lang=spec (object lens folded in; recorded skip: no `analysis: object` in the installed SpecReview schema, quire reports it is not one of 13 candidates). No AssuranceProfile applies. The repo uses no `object:` frontmatter, so missing `object:` is not raised. The 64 KiB per-field bound and 128-agent and 512-byte limits exist in crates/relay-core/src/store/work.rs and src/work.rs.

## Examined Scope

```yaml
scope:
  - {id: "FR-061", path: "spec/functional/FR-061-named-focus-definitions.md", role: examined, excerpt: "A stored record that fails to decode, carries an unknown field, or embeds a\n  shared-model selection with an unsupported ix-board schema version SHALL be\n  reported as a typed unavailable definition naming the focus ID."}
  - {id: "NFR-004-AC-1", path: "spec/non-functional/NFR-004-prerelease-no-compatibility-layers.md", role: context_only, excerpt: "No stored type carries a format-version field, and no code path branches on the age or version of a stored record."}
  - {id: "FR-063", path: "spec/functional/FR-063-shared-focus-membership-and-summaries.md", role: examined, excerpt: "**partial** (the row's counts cover only part of the\n  Linear object, with the reason: a milestone-only or ticket-only inclusion, or an\n  exclusion)."}
  - {id: "FR-062", path: "spec/functional/FR-062-session-active-focuses-and-adoption.md", role: examined, excerpt: "The record expires 30 days after the last named-focus write by that incarnation,\n  as the session's report does; a read never extends it."}
  - {id: "FR-064", path: "spec/functional/FR-064-named-focus-dashboard-reader.md", role: examined, excerpt: "**Legacy mode**: the session has no named record. The Scope view follows FR-060\n  unchanged."}
  - {id: "FR-064", path: "spec/functional/FR-064-named-focus-dashboard-reader.md", role: examined, excerpt: "The Tickets\n  and Agents views and the department views keep their keys and behavior and\n  restrict their scope to the selected view's leaves in named mode"}
  - {id: "FR-064", path: "spec/functional/FR-064-named-focus-dashboard-reader.md", role: examined, excerpt: "[FR-059](./FR-059-department-monitor-adapter.md) departments and explicit-scope\n  startup have no named mode."}
  - {id: "FR-062", path: "spec/functional/FR-062-session-active-focuses-and-adoption.md", role: examined, excerpt: "A session holds at most 16 active focuses (one per focus ID)."}
  - {id: "FR-061", path: "spec/functional/FR-061-named-focus-definitions.md", role: examined, excerpt: "A definition has one owner. The owner record holds an owner ID, an owner\n  epoch, and the last verified binding of the owner."}
  - {id: "FR-008", path: "spec/functional/FR-008-tolerant-registry-decoding.md", role: context_only, excerpt: "unknown fields are ignored and undecodable records are skipped and reported (as summarized by NFR-004)"}
```

## Dispositions

Round 1 against source head e4931a965ef90d3ccbd3a37bee169ea8c0ebd611 (fix commits a39c0bd and e4931a9 on the reviewed frozen head 218a0ff3157d30022689b3c275fc72fd9e503961). Original findings above are unchanged. No new finding for this method.

| FND | outcome | reason |
|---|---|---|
| FND-001 | fixed a39c0bd | FR-061 "Stored-record tolerance" (lines 303-321) says unknown members are ignored on read and kept on rewrite, and names the embedded ix-board `schema_version` as the only version exception, with precedence over NFR-004-AC-1. FR-061-AC-18 and FR-064-AC-5 assert it. |
| FND-002 | fixed a39c0bd | The partial reasons now include the team boundary and apply to initiative and project rows alike (FR-063 lines 61-66, 101-106). FR-063-AC-6 and AC-7 assert both row kinds. |
| FND-003 | fixed a39c0bd | The active record has no time-to-live and a `mode` field (FR-062 lines 59-75). A record read earlier and now absent shows "named record removed" and never falls back (FR-064 lines 70-74). FR-062-AC-13 and FR-064-AC-4 assert it. |
| FND-004 | fixed a39c0bd | FR-064 states that department views never read a named focus and are unchanged (lines 38-40, 122-123); FR-064-AC-13 asserts byte-identical output in both modes. |
| FND-005 | fixed a39c0bd | One stable code table covers the whole contract (FR-061 lines 352-366). FR-062 reuses it (lines 43-44) and FR-062-AC-20 asserts that every refusal carries a listed code. |
| FND-006 | fixed a39c0bd | FR-061 "Adoption pins" (lines 231-257) defines the pin key, value fields, the only writer (`pin put`), the only releaser (`pin remove`) and atomicity. FR-061-AC-15 asserts them. |
| FND-007 | fixed a39c0bd | One field per focus: revision at most 56 KiB, focus field at most 58 KiB, active record read at most 18 fields (FR-061 lines 277-284; FR-062 lines 59-67). FR-061-AC-12 asserts the boundary values. |
| FND-008 | fixed a39c0bd | The owner record names `owner_id`, `epoch`, `binding` (`machine`, `native_session_id`, `process_id`, `address`, `verified_at`) and `change` (FR-061 lines 156-163). FR-061-AC-6 asserts the refresh without changing owner or epoch. |

## Verdict after disposition pass 1

**PASS** for this method. All eight findings are fixed; no new finding.
