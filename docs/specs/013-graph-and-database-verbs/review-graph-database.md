# Review Notes: graph and database lifecycle verbs

Reviewer: Claude, with Todd. Date: 2026-08-17. H2 Phase 3, and the first
destructive verb.

## What exists now

Ten verbs. The traversal family (`graph.traverse`, `graph.neighbors`,
`graph.shortest-path`) over the D7 contract, the registry family
(`graph.list`, `graph.create`, `graph.drop`), `graph.materialize`
refusing under R14, and the V-Q4 lifecycle family (`database.list`,
`database.create`, `database.drop`) running as the new provision role.

The dev cluster gained its two provisioning steps, run and verified
during the build: the `yeomna_provision` role (CREATEDB, NOLOGIN,
granted to `yeomna_app`, no grant on any KG table) and the
`yeomna_template` database (the schema applied by the owner, marked
IS_TEMPLATE). Both are recorded in the schema's prerequisites comment
and the CLAUDE.md cluster table.

## The response shapes as built

| Verb | `data` |
|---|---|
| `graph.traverse` | `graph`, `start`, `nodes` as key, kind, and minimum depth, `truncated` |
| `graph.neighbors` | `graph`, `key`, `neighbors` as key, kind, relation, basis, direction |
| `graph.shortest-path` | `found`, `path` as keys or null, `length`, `truncated` |
| `graph.list` | `graphs` as name, created_at, node and edge counts |
| `graph.create` | `graph`, `created` |
| `graph.drop` | `graph`, `dropped`, `swept` counts of nodes, edges, chunks, embeddings |
| `database.list` | `databases` as name, owner, is_template, size where readable |
| `database.create` | `database`, `kind`, `created` |
| `database.drop` | `database`, `dropped` |

## The rulings, as executed

**R13 held, and the escalation is scoped by construction.** Lifecycle
statements run inside a session helper that issues `SET ROLE
yeomna_provision`, runs the one statement, and resets on every path, so
no error leaves the session escalated (EC-5, asserted by test). The
primary database is protected twice: by name before any SQL, and by
mechanism, since the provision role does not own it and Postgres
refuses the drop itself. The foreign-database test proves the second
layer with a database the owner created.

**R13a held.** A kg stamp is `CREATE DATABASE ... TEMPLATE
yeomna_template`, and the structural comparison passes: ten tables, the
vector extension, `audit_log` owned by `yeomna_audit`, and the
`edges_identity` index, all present in the stamp because Postgres
copied them rather than because code re-created them.

**R14 held.** `graph.materialize` refuses, naming the consumer it waits
for, and the refusal is audited like everything else.

## Findings from execution

1. **The pruning proof reached the verb level, through GENERIC_PLAN.**
   Claim 1 proved partition pruning on a literal query. The verb
   parameterizes everything except the validated basis literals, and
   `EXPLAIN (GENERIC_PLAN)` plans exactly that statement with its
   placeholders unbound, so the test EXPLAINs the SQL the verb exposes
   for the purpose rather than a copy that drifts. The extended
   protocol demands parameter values, so the EXPLAIN goes through
   `simple_query`. The asserted partition is absent from the plan.
2. **The traversal terminates on the real cycles.** On the dogfood
   graph's `calls` edges, from a live starting symbol, the walk
   returns each node exactly once and completes well inside the bound.
   D7's dedup doing its work on the corpus M2 wanted, though M2 itself
   stays open until its benchmark is run and reported.
3. **EC-3 fired on the tests themselves.** The lifecycle tests held
   their own inspection connections open while asking for the drop,
   and the in-use refusal they got was the verb behaving correctly.
   The tests now release the connection first and retry briefly, since
   the backend notices teardown asynchronously.
4. **Phase 2's own guard caught the phase landing.** The EC-1 test
   asserting later-phase verbs refuse still listed `graph.list`, so it
   failed deterministically the moment Phase 3 implemented it. The
   case list now names verbs that remain future, with a comment saying
   the list shrinks as phases land. This is that test doing its job.
5. **A stamped database is owned by the provision role**, which is what
   makes `database.drop` work without superuser and what protects the
   primary. Recorded because it is load-bearing and invisible: the
   ownership comes from `current_user` at CREATE DATABASE time under
   SET ROLE.

## CodeRabbit round (2026-08-17)

Eight findings. Six fixed, one partially corrected, one skipped.

**The session pair, and it was the round's real value.** Two findings
together showed the escalation was safe only under polite use. The
client pipelines concurrent queries, so a read verb sharing an
`Arc<Session>` could have run between SET ROLE and RESET ROLE, and a
call future dropped mid-escalation would never poll the reset at all.
The session now serializes whole calls behind a `tokio::sync::Mutex`,
and an escalation flag is raised before SET ROLE and lowered only when
RESET ROLE completes: a failed or cancelled reset leaves it raised, and
a raised flag retires the session, which refuses every further call.
That is the session-level form of discarding a connection that cannot
prove what role it holds, and it is cancellation-safe because the flag
is raised eagerly rather than lowered in a destructor. The earlier
claim that the helper resets on every path was too strong and is
withdrawn. The contract in full: an error from SET ROLE itself clears
the flag and surfaces, since the role never changed and retiring the
session over a missing role would brick it for nothing. After a
successful escalation, the flag lowers only when RESET ROLE completes,
and a failed or cancelled reset retires the session. A cancellation
while SET ROLE is in flight also retires it, since the statement may
have taken effect server-side with nobody left polling.

**The traversal bound, partially corrected.** The finding claimed a
recursive CTE is materialized in full before an outer LIMIT applies,
which is not how this query shape executes: the capped subquery pulls
rows directly from the walk, and Postgres evaluates a WITH query only
as far as the parent fetches, which is the documented stop-recursion
idiom. The fragility half of the finding stands, since the same
documentation cautions against relying on the idiom, so the fix is
structural: traversal depth is clamped to a fixed maximum whatever the
caller asks, and the cap doubles as the fetch bound. Shortest-path was
also restructured into one walk, answering found and not-found from the
same capped enumeration through a lateral join, where the absence
branch previously paid for the walk twice.

**The atomic sweep.** The swept counts and the graph delete ran as two
statements, so a concurrent writer could make the reported counts lie
about a destructive act. Both now run as one statement, counts and
delete as sub-statements of one WITH, which share one snapshot.

**The leaked SQL builder.** `traverse_sql` was exported so the pruning
test could EXPLAIN the exact statement, and the export was a second
contract beside the verbs, which charter section 6 forbids. It is
crate-private again and the pruning proof moved into the module's own
tests, where it EXPLAINs the same string without anything outside the
crate seeing a query surface.

**Two test defects.** The EC-5 block asserted nothing: it computed a
constant through a query that always errors, and its status check would
have passed escalated or not. `status` now reports `current_user`,
which runs on the session's own connection, the only place role state
lives, and the test asserts it is the app role after every failure
path. And the cycle-termination test used `Vec::dedup`, which removes
only consecutive repeats while the rows are ordered by depth first, so
a repeated key would have survived it. A set proves uniqueness now.

**Skipped: restructuring this file as a phase spec.** The guideline
that `docs/specs/**` files carry the spec sections applies to the
specs. Review notes alongside them are this repository's own documented
convention, stated in CLAUDE.md and practiced by every spec since 001,
and moving them would break the record's shape for the linter's
comfort.

## CodeRabbit round two (2026-08-17)

Five findings. Two fixed, one answered with a proof, two skipped.

**Fixed: the flag outlived a failed SET ROLE.** Round one's retirement
logic raised the escalation flag before SET ROLE and never lowered it
when SET ROLE itself returned an error, so a missing provision role
would have permanently retired every session that tried. An explicit
error from SET ROLE means the role never changed, and the flag now
clears on that path. The cancellation window stays covered, as the
consolidated contract above states.

**Answered with a proof: the shortest-path bound.** The finding
repeated round one's claim that the recursion completes before the cap
applies, this time for the path walk, and proposed a
predecessor-reconstruction rewrite. Rather than argue the fetch
semantics again, the suite now proves them: a complete directed graph
on fifteen nodes, whose simple paths to depth ten are astronomically
many, answers a capped shortest-path in milliseconds, which could not
happen if the walk ran to completion. The test doubles as the
regression guard for the stop-recursion idiom: if Postgres ever
changes that behavior, the test hangs visibly instead of production
finding out. Two hardenings taken from the finding's spirit:
PATH_DEPTH dropped from twenty to ten, since the reference measured
real depths of one to three and every hop of headroom multiplies the
worst case should the fetch bound ever stop holding, and the cap's doc
comment now states both duties. The proposed rewrite is declined: a
node-deduplicated recursive CTE cannot carry a predecessor without the
predecessor defeating the dedup, which is why the path-array form is
the standard Postgres idiom.

The first draft of the dense-graph test taught something worth keeping:
it asserted truncation at a cap of 500 and failed, because UNION dedup
keeps the traverse walk to node-and-depth pairs, roughly 285 rows on
K15. The dedup is what makes traversal polynomial on a complete graph,
which is D7's whole point, and the test now asserts both halves.

**Skipped: the test environment fallback.** The finding asked the
in-crate pruning test to skip when YEOMNA_TEST_DB is unset and to take
its port from configuration. The fallback to the dev cluster's socket
and the fixed port are the standing cluster-gate pattern of every test
since spec 008, stated in those tests' own docs. Changing one instance
would diverge from the convention, and changing the convention is not
this PR.

**Skipped: removing SQL from `read.rs`.** The finding asked that the
verb layer obtain `current_user` through an approved data-access layer
rather than SQL, on the guideline that nothing outside the verb layer
holds SQL. `read.rs` is the verb layer: the charter says the verbs
emit the SQL, and the workspace lint names `yeomna-verbs` as one of
the two crates allowed to hold it. The guideline the finding cites is
the one this file exists to satisfy.

## What is deliberately not here

Naming swept contents inside the audit trail belongs to Phase 6's
`retire` and `prune` per G3, with the Phase 4 transaction pattern.
`graph.drop` reports its sweep in the response and is audited as an
attempt and outcome like every verb. The `sql` verb, kg-pattern
detection for its refusal, and the schema verbs stay in Phases 4 and
H7. M2 has its instrument now and still has no benchmark.

## Verification

- 7 graph-verb tests and 5 database-verb tests green against the
  cluster, including the dogfood-cycle traversal and the structural
  stamp comparison.
- The audit completeness list covers all 22 implemented verbs, chosen
  for Phase 3 so refusals and misses prove auditing without mutating
  the cluster.
- Workspace gate 309 tests, three consecutive runs, clippy clean, fmt
  clean, editorial sweep clean.
