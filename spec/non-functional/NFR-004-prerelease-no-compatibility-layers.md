---
id: NFR-004
title: "Prerelease: no compatibility layers for stored formats"
type: NFR
quality_attribute: maintainability
relationships:
  - target: "ix://agent-ix/ix-relay/FR-008"
    type: "constrains"
---
# NFR-004: Prerelease: no compatibility layers for stored formats

## Statement

The ix-relay codebase shall contain no versioned reader, migration, shim or
fallback path for an earlier stored format of a message, registration, alias,
cache key or escalation.

## Scope

- Every type stored in Redis and every key layout under the bus prefix.

## Rationale

ix-relay is prerelease software with one operator and three machines. Every
stored record is rewritten by its owner within a session (registrations at
`SessionStart`, cache keys at their next write) or drains within minutes (inbox
entries). A compatibility layer would cost more to build and keep correct than the
state it preserves. Forward tolerance is limited to what
[FR-008](../functional/FR-008-tolerant-registry-decoding.md) states: unknown fields
are ignored and undecodable records are skipped and reported.

## Measurement and Evaluation

| Metric | Target | Threshold | Method |
|--------|--------|-----------|--------|
| Stored-format version fields, migration functions or legacy readers in the source | 0 | 0 | inspection |

## Verification

Review of every change to a type stored in Redis confirms that it adds no
version field, migration or legacy reader.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| NFR-004-AC-1 | No stored type carries a format-version field, and no code path branches on the age or version of a stored record. | Inspection |
| NFR-004-AC-2 | A change to a stored type ships without a data migration; records the new code cannot decode are handled only by [FR-008](../functional/FR-008-tolerant-registry-decoding.md)'s skip-and-report rule. | Inspection |

## Dependencies

- **Upstream**: [FR-008](../functional/FR-008-tolerant-registry-decoding.md)
- AGE-2027 phase A audited the source for legacy readers and found none. The
  paths that are tolerant on purpose, and are not compatibility layers, are:
  - `Registration` and `Place` ignore unknown members
    ([FR-008](../functional/FR-008-tolerant-registry-decoding.md)), and
    `Relay::set_place` splices the place into the stored JSON as an untyped
    `Value` so a member another binary added survives the rewrite. Removing it
    drops those members on the next place refresh and fails FR-008-AC-1.
  - `SpoolRecord` ignores unknown members, so an older binary ships a newer
    binary's record instead of dropping it.
  - `HookInput` and the herdr and `linear` outputs are owned by other programs.
  - The escalation `meta` blob ignores unknown members.
  - `MachineName` normalises a stored machine name as it decodes, and the id
    types refuse a value outside their shape rule (AGE-2027 phase B). Neither reads
    an earlier format: the machine rule is the comparison every reader already
    made, now made in one place. The id rules are stricter than `main`'s writers
    were (the hook stored a session id unchecked), so a record `main` could have
    written may now be refused: the registry read skips and reports it (FR-008),
    a single-record read treats its name as held, and the watcher prunes it when
    stale. That is tolerance for an undecodable record, not a reader of an old
    format. Per type: [spec.md section 15.2](../spec.md#152-typed-ids-and-the-tolerance-rule-per-type).

  The wire-format inventory is [spec.md section 15](../spec.md#15-wire-format-inventory).
