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

1. **`deny_unknown_fields` on the request structs is only half the
   actor rejection, and the first review round caught the other half
   missing.** Measured 2026-08-14: serde's adjacently tagged enum
   accepts unknown fields beside `verb` and `args` unless the enum
   itself denies them, so `{"verb": "status", "args": {}, "actor":
   "root"}` deserialized cleanly while the inside-args form was already
   refused. That was a spec violation, since FR 5 and EC-2 both require
   top-level rejection. The enum now carries `deny_unknown_fields` too,
   the EC-2 test gained the envelope-level arm, and the contract is
   declarative at both levels. The original wording of this finding
   claimed serde handled it unaided, which was wrong.
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

## CodeRabbit round (2026-08-14)

Six findings. Five fixed, one skipped.

The one that mattered is finding 1 above, rewritten rather than
appended to, since the original text asserted something measurement
disproved. The rest:

- **Claim 6 targeted its audit row by `verb`,** so every rerun updated
  every prior run's row (audit_log is append-only and never cleaned).
  It now captures the id with RETURNING and asserts exactly one row
  marked.
- **Claim 6's app-connection skip was blanket,** which meant a real
  authentication or database failure would silently bypass the
  permission assertions and pass. Narrowed once to SQLSTATE class 28,
  then narrowed again in the second round (below) to the precise
  question.
- **The outcome format was stated in two places and only one was
  precise.** `schema.sql` and the error module now both say the same
  thing: the column records `failed: <kind>` using
  `VerbError::kind()`, never the detail, while `Display` renders
  `kind: detail` for the response envelope. An audit column is a
  vocabulary, a response is a message.
- **The wire-form doc said requests may omit `args`.** They may not,
  including the empty ones. Corrected.
- **The lint now walks every Rust file in each non-allowed crate**
  (`tests/`, `benches/`, `examples/`, `build.rs`), not only `src/`,
  skipping `target/`, and asserts it visited a nonzero number of files
  so a broken walk cannot report a clean workspace. Verified by
  planting a violation in `yeomna-pipeline/tests/`, which the previous
  `src/`-only walk would have missed: 47 files scanned, violation
  caught, clean again after removal.

**Skipped:** removing `yeomna-verbs` from the lint's allowlist. Spec
FR 7 names both crates, and the verb-layer PRD makes `yeomna-verbs` the
SQL author above the sink from Phase 2 on, so the line this lint draws
is around the two crates that own the store. Removing it would also
false-positive immediately on the contract test's `sql` verb example.

## CodeRabbit round two (2026-08-14)

One finding, and it was right about the fix from round one: SQLSTATE
class 28 covers every authorization failure, not only the missing
`pg_ident` mapping the guard was written for, so a broken `pg_hba`
would still have skipped the append-only assertions silently.

The suggested remedy was an environment opt-in. Declined in that form
and replaced with a sharper one, because a variable nobody exports
converts an occasional silent skip into a permanent one: the claim
would stop being tested on every developer box at once.

**The skip condition, as it now stands, in both the schema claims and
the sink tests:** ask `pg_roles` whether `yeomna_app` exists. An
unprovisioned cluster skips with that reason named. A cluster where
the role exists must connect, and a failure there panics, because on a
provisioned cluster it means misconfiguration rather than absence.
The catalog answers the environmental question exactly, where an error
code only approximates it.

Verified live: 17 store tests green with the role present, clusterless
runs still skip-pass.

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
