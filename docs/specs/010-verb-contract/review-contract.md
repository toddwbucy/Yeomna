# Review Notes: the verb contract

Reviewer: Claude, with Todd. Date: 2026-08-13. H2 Phase 1, the first
construction of the verb-layer era.

## What exists now

`yeomna-verbs`, the ninth crate: the closed `Verb` enum (40 variants,
serde-tagged `{"verb", "args"}`), a strict request struct per verb
(`deny_unknown_fields` everywhere), the captured envelope, and the
five-kind error taxonomy whose kind strings double as audit outcome
failure names. Types only: the crate depends on serde, serde_json,
thiserror, and chrono, and on nothing of the workspace.

Also landed in `yeomna-store`: the `audit_log.outcome` column (nullable,
no default, NULL reads as an attempt whose completion was never
recorded) and the column-scoped `GRANT UPDATE (outcome)`, with claim 6
extended to prove the app role can mark an outcome on the same row where
rewriting `actor` still dies with 42501.

## R4, closed as executed

The 40 wire names match the spec table exactly, proven by the
table-driven test, in order, with uniqueness asserted and a mythology
screen (`db.`, `aql`, `arango`, `collection`, and the brand names) over
every name. The disposition table for the 16 captured commands that are
not verbs stands as specced with no changes found necessary during
execution.

## Findings from execution

1. **`deny_unknown_fields` does the actor-rejection work by itself.**
   No allowlist code was needed: EC-2's smuggled `actor` dies in serde
   on both a fielded request and an empty one. The contract stays
   declarative.
2. **The completeness guard is the exhaustive match.** Adding a variant
   without updating the tests fails at `Verb::wire_name`, whose match
   the compiler forces open, and the comment there points at the test
   list. The count assertion (40) catches a list that drifts.
3. **The no-SQL lint carries its own negative test.** A planted
   violation is caught by the same function the workspace scan uses, so
   a green lint means detection works rather than detection is broken.
   The heuristic (a quote plus a statement fragment on one line) found
   zero false positives across the seven upstream crates.
4. **One clippy round: an unused import in the test file.** Removed.

## The M3 and phase notes

Nothing in this phase touches a connection, so the M3 statements are
unchanged. The `sql` verb's KG refusal is deliberately representable in
types (EC-3) and lands as runtime policy in Phase 4, where the audit
transaction pattern exists to log the refusal as `denied`.

## Verification

- 8 contract tests, 2 lint tests, all green with no cluster needed.
- Claim 6 extension green live against the sealed cluster, schema
  re-applied idempotently with the new column and grant.
- Workspace gate 267 tests green, clippy clean, fmt clean, editorial
  sweep clean, brand grep clean (the mythology test's own banned list is
  the single expected match).
