---
id: FR-022
title: "No legacy aliases or collision fallbacks"
type: FR
---
# FR-022: No legacy aliases or collision fallbacks

Updated for 0.3.0. This replaces the earlier alias and name-precedence contract.

## Behavior

- The relay shall not create slugs, widened names, numbered variants, aliases or demotions. Ambiguity shall be rejected, and missing registration shall be repaired from genuine live evidence.
- A caller lacking genuine session proof shall not adopt or refresh another pane's registration.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-022-AC-1 | Concurrent cold sends share one recipient registration while preserving every distinct message. | Test |
| FR-022-AC-2 | Unverified or different sessions cannot register or send through a live pane. | Test |
