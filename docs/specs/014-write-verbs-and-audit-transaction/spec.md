# Specification: 014 Write Verbs and the Audit Transaction

Parent PRD: `docs/PRD-verb-layer.md` v0.4, Phase 4, under V-Q1 (the audit
row is terminal) and V-Q4 (full utility on plain databases, refusal on
the kg pattern). The fourth of H2's seven phases.
Status: built, 2026-09-09. **Rulings R16 through R18 agreed by Todd
2026-09-09.** The build amended FR3, FR4, and the schema against ruled
decisions the draft missed, recorded in `review-write-verbs.md`
alongside: the store PRD's inherited cascade takes the diff log with a
deleted node, and the relation vocabulary moved into per-partition
CHECKs so asserted edges carry the caller's vocabulary.

Editorial rules: ASCII only, no em-dashes, no semicolons, never the words
genuinely, honestly, or actually. These govern prose. Rust and SQL keep
their syntax.

---

## Overview

The mutation half of the verb layer. Five contract verbs land: `insert`,
`update`, `delete`, `purge`, and the scoped `sql` verb. Two more,
`edge.assert` and `edge.retract`, are proposed as a contract amendment
under R18.

This phase has a first customer. WeaverTools needs a knowledge graph
tracking its moving parts (databases, services, agents, the edges
between them), and that graph is written by hand and by agents through
these verbs, not by the code-ingest pipeline. The customer is also what
exposed R18: a deployment graph is mostly edges, and the 40-verb
contract can write nodes but not edges.

Two firsts ride along. The audit transaction is proven here (the
mutation, the diff-log entry, and the ok outcome commit or vanish
together, shown by a crash-shaped test), and the `sql` verb is the first
surface that touches a database other than the session's own.

## The rulings, as proposed

**R16. The session owns its client inside the lock.** Spec 013 left the
question open: `Client::transaction` demands `&mut Client`, and the
session holds the client behind `&self` plus a serial `Mutex<()>`. The
answer is to merge the two fields: `client: tokio::sync::Mutex<Client>`.
The lock the session already takes to serialize whole calls becomes the
owner of the client, and holding it yields the `&mut` that transactions
need. One mechanism instead of two, and no behavioral change to
serialization. Considered and declined: `&mut self` on `call` (forces
exclusivity onto every future holder and fights the daemon's
shared-session shape), and a second mutex beside the first (two locks
guarding one invariant). The `escalated` flag and the retirement rules
from spec 013 carry over unchanged.

**R17. The `sql` verb executes as `yeomna_provision` on a per-call
connection to the target database.** The PRD requires a role that
Postgres itself gives no KG grants, and R13 already built one: the
provision role owns every plain database it created (full utility there)
and holds no grant on any KG table by design (the backstop holds if
detection ever misses). No fifth role. The verb opens a short-lived
connection to the target database as `yeomna_app`, issues `SET ROLE
yeomna_provision`, runs the statement, and drops the connection. The
drop is the reset: a per-call connection cannot leak an escalation into
a later call, so the spec 013 retirement machinery is not needed on this
path.

**R17a. The kg pattern is detected by structure, not by registry.** The
refusal asks the target database what it is: a catalog probe for the
schema's signature tables (`graphs`, `nodes`, `edges`, `node_log`,
`audit_log`) in schema `public`. Any of them present means the target is
kg-pattern and the verb refuses with `Denied` before executing anything.
A registry of created databases was considered and declined: it is a
second copy of what `pg_database` and the catalog already know, it
drifts when a database is stamped by hand, and the house already ruled
against stored markers once (the schema version). Structure decides and
provenance does not, so a kg-shaped database is protected however it was
born, which errs on the side of refusal. Name gates run first and are
cheaper: the session's own database, `yeomna`, `postgres`, and any
`IS_TEMPLATE` database are refused by name before a connection is made.

**R18. The contract grows two edge verbs: `edge.assert` and
`edge.retract`.** This is a contract amendment, 40 verbs to 42, and the
cheapest moment for one: R4 froze the wire names, but a name is frozen
only while something deployed speaks it, and nothing speaks the wire
until the Phase 5 daemon exists. The charter's basis rule makes the
design almost mechanical: an edge written by hand is `asserted` by
definition, so neither verb carries a basis field. `edge.assert` writes
(src, dst, relation) between two existing nodes in the session's graph
with an optional payload, under R6 identity. `edge.retract` removes one
edge and refuses unless its basis is `asserted`: declared and structural
edges belong to ingest, re-ingest would recreate them, and retracting
one by hand asserts something false about the source. Relation names are
validated as identifiers but not enumerated, per the
no-vocabulary-ahead-of-a-consumer rule: WeaverTools brings its own
relation vocabulary, and closing the list now would be authoring it.

## Files to Modify

- `crates/yeomna-verbs/src/verb.rs`: the R18 variants and their request
  types, wire names `edge.assert` and `edge.retract`, `deny_unknown_fields`
  at both levels like every other request.
- `crates/yeomna-verbs/src/execute.rs`: the R16 session shape, dispatch
  arms for the seven verbs, the Phase 4 refusal arm retired (the
  shrinking-list comment updates to Phase 6 and later).
- `crates/yeomna-verbs/src/write.rs` (new): the node and edge mutation
  verbs and the audit transaction.
- `crates/yeomna-verbs/src/sql.rs` (new): the scoped `sql` verb.
- `crates/yeomna-verbs/src/audit.rs`: a transactional finish, so the
  outcome update can join a caller's transaction instead of committing
  alone.
- `crates/yeomna-verbs/tests/write_verbs.rs` (new),
  `crates/yeomna-verbs/tests/sql_verb.rs` (new).
- `crates/yeomna-verbs/tests/audit.rs`: the completeness list grows by
  seven, so G1 stays a total claim.

## Files to Reference

- `crates/yeomna-store/src/sink.rs`: the diff shapes
  (`{"op":"insert","to":...}`, `{"op":"update","from":...,"to":...}`),
  the `IS DISTINCT FROM` no-op discipline, `append_log`'s seq handling.
- `crates/yeomna-verbs/src/graph.rs`: the measured-then-deleted single
  WITH pattern from `graph.drop`, the force-gate refusal.
- `crates/yeomna-verbs/src/database.rs`: `check_name`, the by-name
  refusal list, the SET ROLE discipline.
- `crates/yeomna-verbs/src/read.rs`: `MAX_PAGE` and applied-limit
  reporting, `check_kind`.
- Specs 012 and 013 with their review notes: the session, attempt
  logging, the provisioning skip-gates.

## Patterns to Follow

- Attempt before act: the attempt row commits before the verb runs,
  unchanged from spec 012. What changes is where the ok lands.
- Kind-only outcomes, swept-count reporting, per-cause skip messages,
  cluster-gated tests, per-test actors.
- Editorial rules in every new document and comment.

## The audit transaction (the phase's namesake)

Spec 012 made every call leave an attempt row that commits before
dispatch, so a crash leaves NULL and a failure leaves its kind. That
stays. What Phase 4 adds: for a mutating verb, the mutation, its
diff-log entries, and the `UPDATE outcome = 'ok'` execute inside one
transaction. The invariant, and the sentence the crash-shaped test
proves: **an ok outcome on a write verb is durable if and only if the
mutation is.** A failure or crash after the mutation begins and before
commit rolls all three back, leaving the head untouched, the log
unappended, and an attempt row whose outcome is NULL (crash) or a
failure kind (error). No partial state is ever visible to another
connection at any point.

The PRD's Phase 4 sentence ("the audit row, the mutation, and the log
entry commit or roll back together") predates spec 012's attempt design
and is read through it: the row that joins the transaction is the
outcome update, not the attempt insert. The attempt committing first is
what makes the crash story tellable at all.

## The `sql` verb (V-Q4 applied)

Order of gates, cheapest first:

1. `check_name` on the target, then the by-name refusals: the session's
   current database, `yeomna`, `postgres`, any template. All `Denied`.
2. Connect to the target as `yeomna_app` on the same socket. Connection
   failure maps as `database.drop` does (absent database is `NotFound`).
3. The R17a structural probe. Signature table present means `Denied`,
   naming the kg pattern in the message.
4. `SET ROLE yeomna_provision`, then the statement text through
   `simple_query` (multi-statement allowed, that is the utility), then
   the connection drops.

Results render as one entry per statement result: column names and text
rows, rows capped at `MAX_PAGE` per result with the applied limit
reported, the read-verb precedent. The statement text reaches the audit
args verbatim because the args are the request and the request carries
it. Errors from Postgres surface through the taxonomy with the SQLSTATE
in the detail, and the attempt row still tells the story when the
statement was never reached.

## Functional Requirements

- **FR1 `insert`.** Requires a session graph, `InvalidArgs` without one.
  `check_kind` gates the kind. An existing (kind, key) in the graph is
  `InvalidArgs` naming the collision (insert does not upsert, that is
  what `update` is for). Writes the head row and appends
  `{"op":"insert","to":payload}` in the audit transaction.
- **FR2 `update`.** `NotFound` when the node is absent. A payload
  identical under `IS DISTINCT FROM` is an audited ok that touches
  nothing and appends nothing, and the envelope says `"changed": false`
  (the R8 discipline, held from ingest). Otherwise the head is replaced
  and `{"op":"update","from":...,"to":...}` appends.
- **FR3 `delete`** (amended at build, see the review notes). `NotFound`
  when absent. In one transaction: delete the head row and let the
  inherited cascade (store PRD) take its chunks, embeddings, edges in
  either direction, and `node_log` entries, with every count measured
  as a sub-statement of the same WITH so the reported sweep and the
  deletion share a snapshot. The diff log goes with the node it
  describes: node ids are surrogates, so a log row for a vanished head
  is unreachable by any verb, and the durable record of the act is the
  audit row, the table built to be exactly that. The draft's
  log-survives clause was withdrawn because the schema and the store
  PRD had already ruled the cascade.
- **FR4 `purge`** (amended at build). The force gate first:
  `force: false` is `Denied` stating what force acknowledges. Then the
  subtree erasure: the node named by key plus every node in the graph
  whose payload names it as parent (`file_key` or `doc_key`, the `list`
  verb's parent convention read in reverse), each with its cascade,
  counts reported including nodes and log entries. This is erasure of
  the live store only, and the spec states what it does not solve: ZFS
  snapshots beneath Postgres retain purged bytes, which is M4, named
  here and left open.
- **FR5 the audit transaction.** As the section above states it, proven
  by a crash-shaped test: induce a failure after the mutation executes
  and before commit, observe no head change, no log entry, and a
  non-ok attempt row. The test may drive the store directly to stage the
  fault, but the observation is made through verbs and owner queries.
- **FR6 `edge.assert`** (under R18). Requires a session graph. Both
  endpoints must exist as nodes in it, `NotFound` naming the missing
  side. Writes basis `asserted` explicitly, R6 identity, `ON CONFLICT`
  on `edges_identity` updates the payload (asserting twice is not an
  error, it is a restatement). Relation names validate as lowercase
  identifiers, length-bounded, not enumerated.
- **FR7 `edge.retract`** (under R18). Removes the (src, dst, relation,
  basis=asserted) edge. `NotFound` when no such asserted edge exists,
  and the message distinguishes "no edge" from "the edge exists with a
  basis retract does not touch," because the second answer teaches the
  caller about ingest.
- **FR8 `sql`.** The gate order above. Full utility on plain databases,
  structural refusal on kg-pattern ones, provision role as the grant
  backstop, statement text in the audit args, per-result row caps.
- **FR9 G1 holds.** All seven verbs join the audit completeness list,
  successes and failures both, and every row carries kind-only outcomes.

## Edge Cases

- **EC-1** A write verb with no session graph: `InvalidArgs` before any
  SQL, and the attempt row records the refusal.
- **EC-2** `update` with an identical payload: ok, `"changed": false`,
  `node_log` count unchanged (the assertion checks the count, not the
  absence of an error).
- **EC-3** `delete` on a node with edges in both directions: all swept,
  counts correct, proven on the seeded A/B/C/D graph shape.
- **EC-4** `purge` then `get`: `NotFound`. `purge` then `insert` of the
  same key: a fresh node whose log starts at seq 1 with an insert op,
  because purge left nothing to continue from.
- **EC-5** `edge.retract` against a structural edge: refused, and the
  edge still present afterward.
- **EC-6** Two sessions writing the same node concurrently: `node_log`
  seq derives from `MAX(seq) + 1`, so the loser of the race hits the
  (node_id, seq) uniqueness inside its transaction. The verb surfaces
  this as a retryable failure rather than corrupting order, and the test
  proves both writers eventually land with distinct seqs. This is M3's
  neighborhood, noted, not resolved.
- **EC-7** `sql` against a database stamped kg by hand (owner applies
  the schema to a plain database outside `database.create`): the
  structural probe still refuses, which is the reason R17a chose
  structure over a registry.
- **EC-8** `sql` whose statement fails mid-batch: Postgres aborts the
  batch, the envelope carries the taxonomy kind with SQLSTATE detail,
  and the audit row says failed with the kind.

## Implementation Notes

DO:

- DO reuse the spec 013 escalation vocabulary in `sql.rs` comments so
  the two SET ROLE sites read as one discipline.
- DO keep the diff shapes byte-compatible with `sink.rs`, since one log
  is read by whatever replays history and two dialects is a bug.
- DO name the shrinking refusal list by phase when dispatch updates.
- DO run the full gate three times before calling the phase done.

DON'T:

- DON'T resolve M4 or claim purge does. Name it and move on.
- DON'T add a basis field, a registry table, an edge log, or any new
  stored marker. Each was considered and declined above.
- DON'T let `sql` reach the session's own connection or database under
  any argument shape.
- DON'T close the relation vocabulary.
- DON'T touch the contract beyond the two R18 variants.

## Success Criteria

1. The seven verbs execute against the dev cluster with every FR
   observable in tests.
2. The crash-shaped test exists and fails if the transaction is split.
3. G1's completeness list covers 42 verbs (or 40, if R18 is declined,
   with the two files unwritten).
4. T3 moves but stays false: Phase 4 closes the document-mutation
   destructive paths, and the thesis waits on Phase 6 (`retire`,
   `prune`) and the Phase 7 repoint.
5. The workspace gate is green three times with cluster tests running,
   not skipping.

## QA Acceptance Criteria

- `cargo build`, `cargo test` (3x, cluster up), `cargo clippy
  --all-targets`, `cargo fmt --check`, all clean.
- `tests/write_verbs.rs` covers FR1 through FR7 and EC-1 through EC-6.
- `tests/sql_verb.rs` covers FR8, EC-7, and EC-8, with provisioning
  skip-gates naming their causes.
- `tests/audit.rs` proves FR9 over the grown list.
- The no-SQL lint still passes with `write.rs` and `sql.rs` inside the
  allowed crate.
