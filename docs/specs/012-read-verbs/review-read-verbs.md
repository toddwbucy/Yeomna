# Review Notes: the read verbs

Reviewer: Claude, with Todd. Date: 2026-08-15. H2 Phase 2, and the first
time a verb has executed.

## What exists now

`yeomna-verbs` stops being types. It carries the session, the dispatch,
the audit write, and eleven verbs that read, each owning the SQL it
emits. It is now the second crate the no-SQL lint admits, which is what
that allowance was reserved for.

Three things arrived that are not reads, and every later phase inherits
them: `Session`, which holds one connection, one actor, and an optional
graph scope; the exhaustive dispatch, where a verb added later fails to
compile until it is handled or refused by name; and the audit write.

**The audit contract, as built.** The row commits before the verb runs,
which is what makes V-Q1 attempt logging: a call that dies mid-flight
leaves a row with a NULL outcome, and a row written afterwards would
leave no trace of the attempt at all. The outcome mark records the
taxonomy's kind and never the detail, since an audit column is a
vocabulary and the detail rides the envelope. A call whose attempt
cannot be recorded is not made (EC-5), which is what one audited entry
point costs when the log is unavailable.

## The response shapes as built

| Verb | `data` |
|---|---|
| `orient` | `schema_version`, and per graph: `nodes_by_kind`, `edges_by_relation_and_basis`, `chunks`, `embeddings`, `embedding_models`, `last_ingest` |
| `status` | `store`, `database`, `graphs`, `schema_version`, `session_graph` |
| `health` | `documents_without_chunks`, `chunks_without_embeddings`, `nodes`, `chunks` |
| `check` | `key`, `exists`, `found_in` as graph and kind pairs |
| `stats` | `scope`, then counts of `nodes`, `edges`, `chunks`, `embeddings`, `graphs` |
| `codebase.stats` | `graph`, `nodes_by_kind`, `edges_by_relation`, `analyzers` |
| `get` | one node: `graph`, `key`, `kind`, `payload`, `ingested_at` |
| `list` | `nodes` as above, plus the `limit` and `offset` that produced them |
| `count` | `count`, `kind`, `graph` |
| `recent` | `nodes`, newest `ingested_at` first |
| `query` | `search_text` and `hits`, each with `graph`, `key`, `kind`, `chunk_index`, `text`, `rank` |
| `schema.version` | `version` |

`health` reports counts rather than a verdict on purpose. A document
with no chunks is normal in one corpus and a defect in another, and the
caller is the one who knows which.

## Findings from execution

1. **Two tests in one binary shared an actor, and raced.** The audit
   tests each clear their own rows by actor, and both used
   `audit-test`, so one test's DELETE wiped another's rows between the
   write and the count. It passed when the binary ran alone and failed
   under the workspace, which is the worst way for a test bug to
   present. Each test names its own actor now, and the full gate was
   run three times to confirm.
2. **The first EC-5 test revoked a cluster-wide grant.** Proving that a
   verb declines to run when the log refuses meant making the log
   refuse, and the first version did that with
   `REVOKE INSERT ON audit_log`, which every other connection sees. It
   passed alone and broke each sibling that called a verb inside the
   window, the same shape of mistake as the schema deadlock in spec
   011. It sets `default_transaction_read_only` on its own connection
   instead, which no other session can observe.
3. **`tokio-postgres` needed its chrono feature.** `timestamptz` has no
   `FromSql` for `DateTime<Utc>` without it. The alternative was
   formatting timestamps into strings in SQL, which puts a date format
   in three queries instead of a feature flag in one manifest.
4. **The dogfood graph earned its place in the suite.** `codebase.stats`
   and `query` are asserted against `yeomna_self`, where the point is
   surviving data nobody wrote for the test. Those cases skip when the
   H3 operation has not been run, since that graph is written by an
   operation rather than by the suite.

## The amendments, as executed

**A1 held.** `query` is search. There is no caller-supplied field or
operator anywhere in the implementation, so there is no allowlist to
escape, and the only input reaching SQL structure is `kind`, checked
against the schema's CHECK set. An unknown kind is refused rather than
answered with an empty result, which would be a false answer.

`hybrid` and `structural` refuse by name rather than degrading, and the
test asserts the message names H4 and H9. A search that quietly ignores
a requested ranking mode is worse than one that says it cannot.

**A2 held.** The audit write is here, and Phase 4 still owns the
transactional coupling to a mutation.

**A3 held.** `embed.text` refuses, naming H4.

**The schema version ruling held.** `SCHEMA_VERSION` is a compiled-in
constant in `yeomna-store` with no stored marker, and both
`schema.version` and `orient` report it. This was the last gap holding
Phase 2 and it closed without a schema change.

## What is still open

The M-questions are untouched: this phase reads, so M3's concurrency
window is not exercised, and M1's FTS relevance question needs a real
firm's document set rather than a code graph. M2 now has its corpus and
still has no benchmark.

`hybrid` and `structural` are contracts with typed refusals until H4 and
H9. The graph verbs, the write verbs, and the daemon are Phases 3, 4,
and 5.

## Verification

- 11 read-verb tests and 3 audit tests green against the cluster, over
  both a seeded scratch graph and the live `yeomna_self`.
- Workspace gate 296 tests, run three times because the bug found here
  was a race.
- Clippy clean, fmt clean, and the no-SQL lint still passes now that
  `yeomna-verbs` holds SQL, which is the allowance working as intended.
