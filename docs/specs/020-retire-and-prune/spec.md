# Specification: 020 Retire and Prune, and T3

Parent PRD: `docs/PRD-verb-layer.md` v0.4, Phase 6, second half. Ruled
by R21 D3 (`retire` removes nodes whose sources are gone, `prune`
removes orphans, both naming what they swept). The seventh PR in R21's
order, and **the one that answers T3's falsifying clause.**
Status: draft, 2026-09-09. **R22 proposed below and needs Todd**, but
nothing in this spec waits on it.

Editorial rules: ASCII only, no em-dashes, no semicolons, never the
words genuinely, honestly, or actually. These govern prose. Rust and
SQL keep their syntax.

---

## Overview

Two verbs, and a thesis.

The charter names the exact sentence that makes T3 false: *"A human
running `codebase retire` today produces no verb-layer record of what
was swept."* Six commands reached the store without constructing a
verb, and the destructive ones were the problem. Five of the six are
verbs already (spec 019 took ingest, drift, and validate, and the
document ingest with them). These are the last two destructive paths,
and when they land **no product path reaches the store around the verb
layer**, every destructive operation leaves an audit row naming its
actor and its request, and T3's clause is answered.

That is a claim to make carefully, and the section below says exactly
what flips and what does not.

## The rulings

**D3, as it applies here.** `retire` removes what the source no longer
has: a file node whose file is gone from the tree, and the symbol nodes
that file declared. `prune` removes orphans, chiefly the symbol nodes
whose declaring file node is absent, which is the class `retire` itself
creates if it ever fails halfway and the class a hand-written insert can
create at any time. Both are force-gated, both report what they swept,
and both do the measuring and the deleting in one statement so the
counts and the deletion share a snapshot, which is `graph.drop`'s
pattern from spec 013.

**Why symbols are not a cascade.** A symbol node names its file through
`payload->>'file_key'`, not a foreign key, because the key is derived
rather than referential. So deleting a file node leaves its symbols
standing, and that is the orphan class these two verbs exist around:
`retire` sweeps the family together, and `prune` is the mop for
whatever arrives orphaned by another route.

**R22, proposed, not blocking.** The audit row records who, when, which
verb, the request's args, and a terminal outcome. It does not record
the substance of what happened, so "retire swept 47 nodes under
`src/old`" is durable only as "retire was called with prefix
`src/old` and succeeded". The enumerated sweep rides the response
envelope, which is not durable. For a regulated buyer that is a gap,
and the shape that fits is a `result` jsonb column written by
destructive verbs only, inside the same transaction as the mutation, so
the record of a sweep is as durable as the sweep. This is a schema
change and a contract question, so it waits for a ruling rather than
arriving inside this spec. Related and recorded twice already: no verb
reads the audit log.

## What T3's flip rests on, precisely

**What becomes true.** Every path in this product that writes to or
deletes from the store goes through a verb, and every verb call leaves
exactly one audit row with the actor the kernel named (G1, proven since
Phase 2, and the daemon proved the peercred half). The charter's
sentence about `codebase retire` no longer describes this codebase.

**What is still open, and is not a bypass.** Three verbs refuse by
name: `embed.text` waits on H4, the three `schema.*` verbs on H7, and
the three `graph-embed.*` verbs on H9. Those are capabilities the
appliance does not have yet, not surfaces that route around the verb
layer, and T3's falsifiable clause is about the second. If one of them
ever ships as a side door instead of a verb, T3 goes false again and
this paragraph is where to look.

**What this spec does not claim.** T3 says an agent is governed like a
user. The proof of that is one audited entry point, which now holds.
Whether the *record* is rich enough for a regulator is R22's question
and stays open.

## Task Scope

- `codebase.retire`: given a graph and a path prefix, sweep the file
  nodes under that prefix whose files are gone from the working tree,
  and the nodes those files declared. Force-gated. Reports the swept
  counts and the retired paths.
- `codebase.prune`: sweep orphans in a graph, chiefly symbol nodes whose
  declaring file node is absent. Force-gated. Reports counts and a
  bounded sample.
- Both audited, both atomic, both reporting through the envelope.
- The charter's T3 paragraph and the ledger's T3 rows updated to say
  what changed and what did not.

## Out of Scope

- R22's durable sweep record, until it is ruled.
- `graph-embed.update` (H9).
- Any retention or scheduling policy. Charter says retention is cron
  plus cascade, inherited, not authored.
- M4. Purging bytes from beneath Postgres is still unsolved and this
  spec does not pretend otherwise.

## Files to Modify

- `crates/yeomna-verbs/src/ingest.rs`: the two verbs beside the four.
- `crates/yeomna-verbs/src/execute.rs`: dispatch, and the Phase 6
  refusal arm removed entirely.
- `crates/yeomna-verbs/tests/ingestion_verbs.rs`: the destructive tests.
- `crates/yeomna-verbs/tests/audit.rs`: two more completeness entries.
- `README.md` (T3), `docs/holes.md` (the T3 rows), `CLAUDE.md`.

## Files to Reference

- `crates/yeomna-verbs/src/graph.rs`, `drop`: the measured-then-deleted
  single WITH, which is the pattern both verbs use.
- `crates/yeomna-verbs/src/write.rs`, `purge`: the family sweep by
  `file_key` and `doc_key`, which `retire` reuses in shape.
- `crates/yeomna-pipeline/src/codebase.rs`, `drift`: the definition of
  gone, which `retire` must agree with. A file the walk could not
  assess is present and must never be retired, which spec 019's round
  one established for exactly this reason.

## Functional Requirements

- **FR1** `codebase.retire` with `force: false` is `Denied` naming what
  force acknowledges, and sweeps nothing.
- **FR2** With force, it sweeps file nodes under the prefix whose files
  are absent from the tree, plus the nodes those files declared, and
  reports `nodes`, `edges`, `chunks`, `embeddings`, `log_entries`, and
  the retired paths.
- **FR3** A file that is present but could not be assessed is never
  retired. `retire` walks the tree the same way `drift` does and treats
  unassessable as present, because the alternative deletes the graph's
  knowledge of source that is right there.
- **FR4** A prefix matching nothing sweeps nothing and succeeds, since
  an empty sweep is a fact.
- **FR5** `codebase.prune` with `force: false` is `Denied`. With force
  it sweeps orphaned nodes and reports counts plus a bounded sample.
- **FR6** Both are atomic: the reported counts and the deletion share
  one snapshot, so the number an operator reads is the number that went.
- **FR7** Both leave one audit row with the actor and the request args,
  which is what T3's clause asks for.
- **FR8** The dispatch has no Phase 6 refusal arm left.

## Edge Cases

- **EC-1** `retire` on a prefix whose files all still exist: zero swept,
  success, and the graph unchanged.
- **EC-2** `retire` where the tree path does not exist at all: every
  file under the prefix is gone by definition, which is a large sweep,
  so the tree must be a readable directory or the verb refuses
  (`InvalidArgs`). A missing tree is not license to empty the graph.
- **EC-3** `prune` on a clean graph: zero swept, success.
- **EC-4** `prune` after a `retire` that swept a family: nothing left to
  prune, because retire took the symbols with the file.
- **EC-5** A symbol node whose `file_key` names no file node **in its own
  graph** is an orphan and is swept, even when some other graph holds a
  node under that key. The parent lookup is graph-scoped on purpose:
  `file_key` is derived from the path alone, so two graphs over the same
  tree hold identical keys, and a cross-graph lookup would let each hide
  the other's orphans. Proven by removing the scope and watching the
  planted-orphan test sweep nothing.
- **EC-6** `retire` with a prefix that is not a path prefix of anything,
  including an empty string: an empty string would mean the whole graph,
  so it is `InvalidArgs`. A destructive verb does not accept a wildcard
  by omission.

## Implementation Notes

DO:

- DO reuse `drift` to decide what is gone, so `retire` and `drift` can
  never disagree about it.
- DO measure and delete in one statement.
- DO report the retired paths, not only counts, since an operator
  reviewing a sweep needs to know which files.
- DO run the gate three times before calling it done.

DON'T:

- DON'T retire on a tree that cannot be walked (EC-2).
- DON'T accept an empty prefix (EC-6).
- DON'T write the sweep into the audit row yet. That is R22 and it is
  not ruled.
- DON'T claim T3 is true beyond what the section above says.

## Success Criteria

1. Both verbs sweep what they should and refuse without force.
2. A present-but-unassessable file survives a retire, proven by test.
3. The Phase 6 refusal arm is gone from dispatch.
4. The charter and ledger say what T3's flip rests on and what stays
   open.
5. Gate three times green with cluster tests running.

## QA Acceptance Criteria

- `cargo build`, `cargo test` (3x, cluster up), `cargo clippy
  --all-targets`, `cargo fmt --check`, all clean.
- Tests cover FR1 through FR8 and EC-1 through EC-6.
- The G1 completeness list covers all 42 verbs' implemented set.
