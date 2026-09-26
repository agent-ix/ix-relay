---
id: SR-002
title: "EARS conformance review of the ix-relay specification"
type: SpecReview
analysis: ears-conformance
scope: "spec/stakeholder, spec/functional, spec/non-functional"
review_set: subset
---
# SR-002: EARS conformance review of the ix-relay specification

## Summary

The engine check (`quire validate --summary`) reports 57 of 57 documents
grammar-clean with 0 findings, after the re-measurement against de799f9 and the
AGE-2033 additions (FR-028 to FR-037, whose multi-`shall` and passive statements
were split or given agents while authoring). The first pass reported 39 of 39, after the authoring pass split multi-`shall`
descriptions (FR-004, FR-012, FR-013, FR-014, FR-027), named agents for passive
statements (FR-011, FR-025), and replaced an open-ended `and so on` (FR-022). The
semantic pass below finds no statement whose ambiguity changes what gets built;
the residue is compound responses and pattern choice.

## Findings

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-001 | low | FR-008 Description carries two responses under one `shall` (ignore unknown fields; skip and report undecodable records). Both are verified by separate criteria, so traceability holds, but the statement could split. | FR-008 |
| FND-002 | low | FR-017 Description joins "exit 0" and "print nothing or one JSON object" under one `shall`; the two fail independently and are checked by one criterion (FR-017-AC-1). | FR-017 |
| FND-003 | low | FR-021 Description puts its universal quantification ("from any workspace label, …") inside a `When` trigger; it is a ubiquitous property of the deriver, better stated `The name deriver shall …, for every …`. | FR-021 |
| FND-004 | low | FR-014 Description is a prohibition written as a ubiquitous `only if`; the unwanted-behaviour pattern (`If … then the watcher shall not …`) states it more directly, as the Behavior list already does. | FR-014 |
| FND-005 | low | NFR-002 Statement ("within a fixed bound") is not itself measurable; the measurement table carries the concrete bounds, which the NFR archetype allows. | NFR-002 |
| FND-006 | low | FR-012 Description names the mechanism ("one atomic Redis operation"); the observable obligation is "no two pops return the same message and nothing leaves unreturned". Kept because atomicity under concurrency is the contract callers rely on. | FR-012 |
