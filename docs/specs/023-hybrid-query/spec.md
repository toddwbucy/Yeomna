# Specification: 023 Hybrid Query

Parent PRD: `docs/PRD-embedder.md` v0.2, Phase 3. Also answers the store
PRD's Phase 5 sentence "Hybrid is one statement", which has been a claim
since that PRD was drafted and is now either true or not.
Owner: the `hybrid` flag on `query`, which has refused since spec 012.
The tenth and last PR in R21's order.
Status: draft, 2026-09-10. Stacked on spec 022, which supplies the vector.

Editorial rules: ASCII only, no em-dashes, no semicolons, never the
words genuinely, honestly, or actually. These govern prose. Rust and SQL
keep their syntax.

---

## Overview

`query --hybrid` has refused since spec 012, first naming H4 and then,
once H4 filled, naming this spec. The embedder exists, the vectors are in
the store with their cohort on the row, and the flag can stop refusing.

Two things make this small and one makes it interesting.

It is small because both halves already work. Full-text ranking with
`ts_rank_cd` over the generated tsvector has been live since spec 012, and
`embeddings.vec` is `halfvec(2048)` under an HNSW cosine index with the
corpus's cohort recorded beside every row. What is missing is the fusion.

It is interesting because **the corpus and the query have to be
comparable, and that is a check rather than an assumption.** A vector
embedded under one LoRA adapter and a query embedded under another produce
a ranking that is plausible and wrong, with nothing in the result saying
so. R26 put `model`, `model_revision`, and `task` on the row for exactly
this moment: the verb reads the graph's cohort, refuses a graph holding
more than one, and embeds the query at the task that pairs with the
corpus's rather than at a fixed one.

## Task Scope

- **Reciprocal rank fusion in one statement.** One CTE ranks by
  `ts_rank_cd`, one ranks by `vec <=> query`, and the fusion is
  `sum(1.0 / (K + rank))` over the union, ordered by the fused score.
  One round trip, which is the store PRD's claim.
- **The query vector comes from the verb**, computed through the session's
  embedder at the task that pairs with the corpus's. Never accepted from
  the caller (PRD D7).
- **The cohort check**, before the fusion: the graph's distinct
  `(model, model_revision, task)` triples, refusing zero and refusing more
  than one, each with a different message because they are different
  problems.
- **The pairing table**: which query task goes with which corpus task, in
  one place, with `code` marked as waiting on R28.
- **`K` as a named constant**, not a request field.
- **The response says what it fused**: each hit carries its rank from each
  source and the fused score, so a caller can see why something ranked
  where it did.

## Out of Scope

- **`structural`**, which still refuses and still names H9.
- **Reranking of any kind.** RRF is rank-only fusion by design and takes
  no model.
- **Tuning.** `K` is 60, which is the constant the RRF literature uses and
  the value docling-rag used where the ledger's harvest pointer read it.
  Whether 60 is right for this corpus is a measurement (M1), not a spec.
- **A `hybrid` mode for any verb but `query`.**
- **M1.** Whether relevance is good enough on a firm's document set needs a
  firm's document set.

## Files to Modify

- `crates/yeomna-verbs/src/read.rs`: `query` gains the hybrid path.
- `crates/yeomna-verbs/src/embed.rs`: the pairing table, since it is about
  tasks and that is where the task default lives.
- `crates/yeomna-verbs/tests/read_verbs.rs`: the tests.
- `docs/holes.md`, `CLAUDE.md`, `docs/PRD-embedder.md` (Phase 3 built).

## Files to Reference

- `crates/yeomna-verbs/src/read.rs`: the existing FTS statement, which the
  hybrid path must rank identically so the two modes agree about keyword
  relevance.
- `crates/yeomna-store/schema.sql`: `embeddings_cohort`, the index the
  cohort check reads, and `embeddings_hnsw` for the vector half.
- `crates/yeomna-store/src/sink.rs`: the `$N::text::halfvec` literal
  convention, which is how a vector crosses into SQL here.

## Patterns to Follow

- **The verb owns its SQL** and the no-SQL lint's allowlist does not
  change.
- **A refusal names what would fix it.** The cohort refusals name the
  graph's cohorts and what to do, the way `InstallError::NoSource` names a
  ruling.
- **Cluster-gated and service-gated separately**, with a named skip for
  each, because a hybrid test needs both Postgres and the embedder and a
  run missing one should say which.

## Functional Requirements

- **FR1** `query --hybrid` returns hits ranked by RRF over both sources,
  in one statement, verified by the statement being one statement.
- **FR2** A hit carries `rank` (the fused score), `text_rank` and
  `vector_rank` (its position in each source, absent when it appeared in
  only one), and the fields the keyword path already returns.
- **FR3** A document matching on meaning and not on words is returned, and
  a document matching on words and not on meaning is returned. Both are
  what fusion buys and either alone would be a mode this already had.
- **FR4** The query is embedded at the task that pairs with the corpus's,
  read from the row rather than assumed.
- **FR5** A graph with no embeddings refuses with a message saying to
  ingest with embedding on, and does not silently fall back to keyword
  ranking. A caller who asked for hybrid and got keyword would have no way
  to know.
- **FR6** A graph holding more than one cohort refuses and names them,
  because there is no single query vector comparable to all of them.
- **FR7** `hybrid` without a session embedder refuses the way `embed.text`
  does, naming the config key.
- **FR8** `structural` still refuses and still names H9.
- **FR9** `K` is a compiled-in constant and no request field reaches it.

## Edge Cases

- **EC-1** A term that matches no chunk at all and a vector far from
  everything: an empty hit list, not an error.
- **EC-2** A chunk that ranks in both sources appears once, with both
  ranks, and scores higher than either source alone would put it. This is
  the whole point of fusion and it is the assertion worth making.
- **EC-3** `limit` is honored after fusion, not before, so a document
  ranked eleventh in each source and first fused is not lost to a
  per-source cut. The per-source CTEs take a wider slice than `limit`.
- **EC-4** A graph with chunks and no embeddings, which is what an
  over-ceiling document leaves: FR5's refusal, and the chunks stay
  findable by keyword without the flag.
- **EC-5** The corpus task is `code`, whose query pairing is what R28 asks
  about. The pairing table uses `code` for both halves and the table says
  it is provisional, so a ruling changes one line.
- **EC-6** An embedder that answers but has not loaded its weights:
  refused before any SQL runs, since a query that cannot have a vector
  cannot be hybrid.

## Implementation Notes

DO:

- DO rank the keyword half with the same `ts_rank_cd` and
  `websearch_to_tsquery` the non-hybrid path uses, so the two modes cannot
  disagree about what a keyword match is worth.
- DO read the cohort from the graph rather than from config, because the
  graph is what the vectors are in.
- DO take a wider slice per source than `limit` (EC-3), and say why in the
  code.
- DO put the query vector in as one parameter cast the way the sink does
  it.
- DO run the gate three times, and run the local review before the PR.

DON'T:

- DON'T fall back to keyword ranking when the vector half is impossible.
  A silent downgrade of a requested ranking mode is what spec 012 refused
  and the reason `hybrid` errored rather than being ignored.
- DON'T accept a vector, a `k`, or a per-source weight from the caller.
- DON'T add a second statement, a Rust merge, or a temporary table.
- DON'T touch `structural`.

## Success Criteria

1. `query --hybrid` answers over a real graph, and a document findable
   only by meaning comes back.
2. One statement, one round trip.
3. Every refusal in FR5 through FR8 has a test.
4. The verb layer's last refusal that names a filled hole is gone.
5. Gate three times green with the cluster and the embedder up.

## QA Acceptance Criteria

- `cargo build`, `cargo test` (3x, cluster and embedder up), `cargo clippy
  --all-targets`, `cargo fmt --check`, all clean.
- Tests cover FR1 through FR9 and EC-1 through EC-6.
- The no-SQL lint passes with its allowlist unchanged.
- The editorial sweep is clean.
