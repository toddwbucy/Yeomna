# Review notes: 014 write verbs and the audit transaction

Status: build complete 2026-09-09, awaiting CodeRabbit. Rulings R16
through R18 agreed by Todd 2026-09-09 before the build. The gate ran
three times at 327 green with cluster tests running.

Editorial rules: ASCII only, no em-dashes, no semicolons, never the
words genuinely, honestly, or actually.

## Build findings, and what they amended

Three places where the spec as agreed met the repository as built. Each
is recorded here rather than silently absorbed.

**1. FR3 and FR4 are amended: the cascade wins.** The spec drafted
delete as head-removal with the diff log surviving. The schema says
otherwise: `node_log.node_id` carries `ON DELETE CASCADE`, and the
store PRD rules the point explicitly ("The deletion cascade is
inherited... Foreign keys with ON DELETE CASCADE do it instead"),
retiring the reference's six-step manual purge. A ruled decision
outranks a drafted FR, so:

- `delete` removes one node and its cascade closure, log included, with
  the sweep measured in the same snapshot and reported
  (edges, chunks, embeddings, log_entries). The durable record of the
  act is the audit row, which is the table built to be exactly that.
- `purge` is the force-gated subtree erasure: the node named by key
  plus every node in the graph whose payload names it as parent
  (`file_key` or `doc_key`, the `list` verb's parent convention read in
  reverse). EC-4 holds: a re-insert starts a fresh lineage at seq 1.
- History surviving deletion was also weaker than it read: node ids are
  surrogates, so a log row for a deleted head is unreachable by any
  verb. Keeping it would have been storage, not history.

**2. The relation CHECK moved into the partitions.** FR6's open
asserted vocabulary collided with the edges table's closed relation
CHECK, which the spec's file list missed. Resolution: the parent drops
the CHECK, `edges_declared` and `edges_structural` keep the closed
five-relation list, and `edges_asserted` checks identifier shape
(`^[a-z][a-z0-9_]{0,62}$`), mirrored by `check_relation` in write.rs.
The trust boundary that partitions the table now also scopes the
vocabulary, which reads like the design finding its own shape. This is
a schema shape change, so `SCHEMA_VERSION` bumped to 1.1.0 and the dev
cluster took the documented no-migration path: objects dropped and
re-applied, `yeomna_template` re-stamped, the dogfood graph re-ingested
(now 1276 calls edges, rust-analyzer enriched).

**3. The session gained an endpoint.** The `sql` verb opens a per-call
connection to its target database, and a session built from an already
connected client does not know where its cluster answers. `Session`
carries an optional `(socket_dir, port)` via `with_endpoint`, and a
session without one refuses `sql` with an internal error naming the
gap. The daemon (Phase 5) will set it from its own listen config.

## What landed

- R16: `Session` holds `client: tokio::sync::Mutex<Client>`. The call
  lock now yields the `&mut Client` transactions need. Verb modules
  take a borrowed `Exec` view with the same `client()`, `graph()`, and
  `as_provision()` surface, so their bodies did not change.
- The audit transaction: mutation, diff-log entries, and the
  `outcome = 'ok'` mark commit together. `audit::finish` gained an
  `outcome IS NULL` guard so the in-transaction mark is terminal and
  the generic pass after dispatch is a no-op for committed writes. The
  crash-shaped test lives in write.rs beside `insert_txn`, runs every
  product statement, drops the transaction where a crash would kill
  the process, and observes no head, no log, and a NULL outcome.
- Seven verbs: insert, update, delete, purge, edge.assert,
  edge.retract, sql. The contract grew to 42 wire names (R18), with
  the two edge verbs at the end of the R4 table so earlier positions
  are stable.
- The diff shapes are the sink's, byte for byte
  (`{"op":"insert","to":...}` and update's from/to), one log dialect.
- `sql` (R17/R17a): name gates, then the connection, then the
  structural probe (any of graphs, nodes, edges, node_log, audit_log
  in schema public refuses), then `SET ROLE yeomna_provision`, then
  `simple_query`. The per-call connection drops at the end, which is
  why the spec 013 retirement machinery is not needed on this path.

## Test inventory

- `tests/write_verbs.rs` (12): FR1 through FR7, EC-1 through EC-6,
  including the concurrent-writer race (both sessions land after a
  retry, log seqs dense and distinct) and the structural-edge refusal.
- `tests/sql_verb.rs` (4): FR8, EC-7 (a hand-stamped plain database
  refuses on structure), EC-8 (SQLSTATE in the detail, kind in the
  audit row).
- `src/write.rs` in-module (1): the FR5 crash shape.
- `tests/audit.rs`: the G1 completeness list grew by seven,
  refusal-shaped so the shared fixture stays unmutated.
- `tests/read_verbs.rs`: the shrinking refusal list dropped Phase 4.

## Riding items

- EC-6 notes rather than resolves M3: the seq race is real under
  concurrent sessions and surfaced as a retryable failure. A verb-level
  retry was considered and declined to keep the failure honest.
- M4 is named in purge's documentation and stays open: ZFS snapshots
  beneath Postgres retain purged bytes.
- The gh CLI credentials on this machine expired mid-build, so the
  issue and PR were opened once auth returned rather than at branch
  time.
