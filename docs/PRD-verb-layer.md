# PRD: The Verb Layer

ASCII only. No em-dashes. No semicolons. Never the words genuinely,
honestly, or actually.

## Revision History

| Version | Date | Changes |
|---|---|---|
| 0.1 | 2026-08-13 | First draft. Architecture, audit, transport, naming, phases. |
| 0.2 | 2026-08-13 | Todd's review round. V-Q1 ruled (attempt logging), V-Q2 ruled (inherit role policy from Postgres, daemon authors nothing), crate layout confirmed. V-Q3 restated as structured self-description, pending nod. New V-Q4 (multi-database utility) replaces the removal of the database commands. |
| 0.3 | 2026-08-13 | V-Q3 ruled: orient is a per-graph survey, welcome text is deployment config (H10). V-Q4 clarified (the yeomna database means each appliance's own KG database, customer data, nothing of ours ships), Option B pending final nod. |
| 0.4 | 2026-08-13 | V-Q4 ruled: immutable in structure, not in content. Databases are the customer's, the yeomna pattern is stampable, the scoped sql verb serves plain databases and refuses KG-pattern ones. All open questions closed. |

## Executive Summary

The verb layer is the only surface anyone touches. An agent calls verbs, a
human calls the same verbs, both are logged the same way, and nobody above
the line writes SQL, JSONB path expressions, traversals, or search calls.
The charter calls this the largest single block of work in the build, and
T3 stays false until it exists, because today the destructive paths reach
the store without producing any record of what they did.

This PRD fixes the architecture, the audit contract, the transport, and
the naming, and phases the construction. The verb inventory itself was
fixed in the store PRD (32 kept commands plus 6 unverbed ingestion
operations) and is not reopened here. This is the implementation PRD the
store PRD named as separate and later. Later is now.

Everything here is built from Yeomna's own contracts: the inventory, the
captured CLI surface and envelope in `yeomna-cli`, charter section 6, and
the live schema of specs 008 and 009. The reference's dispatch, service,
and daemon code is excluded from the port and is not consulted.

## Background and Context

Three facts shape the design.

**The store exists and is sealed.** Specs 008 and 009 put the schema and
the sink on a cluster that binds no network listener. The verb layer is
the second consumer of that schema and the first reader of it.

**The audit table exists and is empty.** Spec 008 shipped `audit_log`
owned by `yeomna_audit`, append-only for the app role, per charter
section 6's ruling that integrity mechanisms are shipped defaults. The
verb layer is what writes it, and defines `actor`.

**The surface is already captured as holes.** `yeomna-cli` (PR #19)
carries 56 commands, every one a self-reporting hole, census
machine-checked. The verb layer is what turns those holes into behavior,
and this PRD owes each one a disposition.

The reference is not consulted, but its measured shape motivates the
requirement: roughly 25 of 57 query sites sat outside the verb boundary,
including all the destructive ones, and six CLI commands reached the
store directly. A high-fidelity port would reproduce that bypass. On this
surface the behavior is kept and the path is replaced.

## User Stories

- **As an auditor**, I ask who did what and get one log and one command
  set to reason about. Every mutation and every read arrived through a
  verb, every verb call is a row in `audit_log`, and the log's owner is
  not the operator I am auditing.
- **As an agent**, I call verbs and receive JSON envelopes. I cannot
  write SQL, and nothing I can say to the daemon produces a query the
  verb vocabulary does not already name.
- **As the human operator**, I run the same verbs through the CLI and am
  logged identically. My convenience surface is table-formatted output,
  not a side door.
- **As the developer of the next era**, I swap what sits below a verb
  (recursive CTE to graph extension, native FTS to pg_search) and no
  caller notices, because callers depend on the contract at the socket
  and never on the substrate.

## Goals

1. **G1. One audited entry point.** Every verb call, read or write, human
   or agent, writes one `audit_log` row carrying actor, verb, and args.
   Mutating verbs write it in the same transaction as their mutation.
2. **G2. A closed vocabulary.** The verb set is a closed enum with
   exhaustive dispatch. Adding a verb is a code change and a review, not
   a runtime registration.
3. **G3. T3 flips true.** The six unverbed operations get verbs, and the
   destructive ones (`retire`, `prune`, ingest overwrite, embed update)
   produce audit records naming what was swept.
4. **G4. The trust boundary is the kernel's.** Transport is the Unix
   socket only. Actor identity comes from `SO_PEERCRED`, not from a
   client-supplied field.
5. **G5. Naming is Yeomna-native at birth.** R4's one standing option
   (rename) is exercised now, at the cheapest moment, before anything
   deployed speaks the old names.
6. **G6. Every CLI hole gets a disposition.** Filled by a verb, absorbed
   into another verb, or removed with the removal recorded.

## Non-Goals

- **No raw query surface.** `DbAql` stayed behind and nothing equivalent
  arrives. The bounded query verb takes a structured filter, never a
  string of SQL.
- **No MCP.** Parked by the charter. The socket daemon is where a future
  front-end would attach, and that is the whole concession.
- **No network transport.** Not behind a flag, not disabled by default.
  Absent.
- **No task management, no smell checking.** Methodology stays dropped.
- **No RLS or directory integration yet.** Access segmentation via the
  customer's directory is a deployment-era inheritance. The appliance is
  single-operator today and the session policy below is sized for that.
- **No graph-embed implementation.** `graph-embed` verbs get contracts
  and return a named unimplemented error until H9 supplies the model.

## Feature Specifications

### Phase 1: The contract crate

The verb vocabulary as types. A `Verb` enum (closed, exhaustive), typed
request and response structs per verb, the JSON envelope preserved from
the captured CLI convention (`success`, `command`, `data`, `timestamp`,
and `error` on failure), and the error taxonomy (not found, invalid
args, unimplemented, denied, internal). Serialization round-trip tests
pin every shape. No I/O in this phase.

### Phase 2: Read verbs over the live schema

Orientation and document read: `orient`, `status`, `health`, `check`,
`stats`, `get`, `list`, `count`, `recent`, and the bounded `query`. Each
verb owns the SQL it emits. `query` takes a structured filter (field,
operator, value, limit, kind) validated against a field allowlist, and
its contract test proves a hostile filter cannot escape the allowlist.
FTS search rides `query` via a `match` operator over the tsvector
column, ranked with `ts_rank_cd`.

### Phase 3: Graph verbs

`graph traverse`, `graph neighbors`, `graph shortest-path`, `graph
list`, `graph create`, `graph drop`, `graph materialize`. Traversals
emit the D7 `UNION` node-dedup form with basis filters that prune
partitions (claim 1 is the guarantee). Path-returning forms are the
named exception, enumerating with the hard row cap the store PRD fixed.
`graph drop` is the first destructive verb and sets the audit pattern
for the rest. Database lifecycle rides here too: `database list`,
`database create` (kg pattern applies the schema at birth, plain comes
up empty), `database drop`, all audited.

### Phase 4: Write verbs and the audit transaction

`insert`, `update`, `delete`, `purge`, plus the diff-log write path:
every node mutation appends to `node_log` and the head row stays
materialized convenience. The audit row, the mutation, and the log entry
commit or roll back together. This phase is where G1's same-transaction
requirement is proven by a crash-shaped test (fail after mutation,
before commit, observe neither). The scoped `sql` verb lands here as well,
carrying the V-Q4 ruling: full utility against plain databases with
statement text in the audit args, refusal against KG-pattern databases,
running under a role that Postgres itself gives no KG grants.

### Phase 5: The daemon

The Unix-socket server. Length-prefixed JSON frames carrying one verb
request and one envelope response. `SO_PEERCRED` read at accept, mapped
to the actor string written into every audit row for the connection.
Socket file mode 0600 in a 0700 directory, sized for the single-operator
appliance. Systemd unit mirroring the postgres one (`RequiresMountsFor`
not needed, `RestrictAddressFamilies=AF_UNIX` very much needed). The
CLI's `daemon` hole fills here.

### Phase 6: The ingestion operation verbs

`ingest`, `retire`, `prune`, `drift`, `validate`, `embed update` as
verb contracts. `ingest` wraps the H3 orchestrator when it exists (the
verb lands first and returns unimplemented until then if H3 has not
merged). `validate` is the reduced runtime check the store PRD promised
to quantify, now that constraints do most of its old job. `retire` and
`prune` name what they swept in their audit args, which is the sentence
T3's truth rests on.

### Phase 7: CLI repointing and census retirement

`yeomna-cli` commands stop being holes and become thin clients of the
daemon. The census test inverts: instead of asserting every command
reports its hole, it asserts every command reaches its verb. Table
output stays a CLI-side rendering of the same envelope.

## Technical Architecture

### Crate layout

Two new crates, one boundary each:

- **`yeomna-verbs`**: the contract types (Phase 1) and the verb
  implementations (Phases 2 through 4, 6). Depends on `yeomna-store` for
  connection plumbing and on the schema by knowledge. This crate is the
  only SQL author above the sink, and the workspace lint that enforces
  "no SQL outside yeomna-store and yeomna-verbs" is part of Phase 1.
- **`yeomna-daemon`**: transport only. Framing, peercred, connection
  lifecycle, dispatch into `yeomna-verbs`. No SQL, no business logic, in
  deliberate contrast to the reference's 7.9k-line dispatch file.

Layout confirmed by Todd, 2026-08-13.

`yeomna-cli` gains a client module speaking the frame protocol and loses
nothing else.

### Resolved Design Decisions

**V1. Verbs are an enum, dispatch is a match.** A closed vocabulary is
the point. Serde-tagged enum over the wire, exhaustive match in the
daemon, compiler-enforced coverage.

**V2. Reads are audited too.** Charter section 6 says human and agent
are logged the same way, and it does not carve out reads. One row per
verb call. Volume is a non-problem at appliance scale, and if it ever is
one, retention is cron plus cascade per the inherit-do-not-author rule.

**V3. The actor is the kernel's answer.** `SO_PEERCRED` uid resolved to
a name at accept time. No client-supplied identity field exists in the
protocol, so there is nothing to spoof. In-process callers (tests, the
CLI in a future embedded mode) supply an actor through a constructor
that is not reachable over the wire.

**V4. Renames, the mapping.** Principles: brands and structure words
from the old store are mythology and change, engineering words stay.
Applied to every inherited name that assumed document-store structure:

| Old | New | Why |
|---|---|---|
| `DbCollections` | absorbed into `schema show` | the schema is shipped and fixed, there is no dynamic collection set to list |
| `DbCreateCollection` | removed | tables are born in `schema apply`, not at runtime |
| `DbCreateIndex` | removed from runtime, absorbed into `schema apply` | same reason, the appliance ships its indexes |
| CLI `db databases`, `db create-database` | kept as `database list`, `database create` (with a kind: kg pattern or plain), plus `database drop` | ruled under V-Q4, the yeomna pattern is stampable and lifecycle is the customer's |
| CLI `db truncate`, `db drop-collection` | absorbed into `graph drop` and `purge` | the destructive verbs that exist carry the audit story |
| `DbQuery` | `query` with the structured filter | the name survives, the raw surface does not |
| everything else | keeps its name minus the `Db` prefix | engineering, not mythology |

The final name-by-name table lands in the Phase 1 spec, which is where
R4 closes.

**V5. One connection per session, no pool.** The daemon opens one store
connection per client session, matching the sink's one-sequential-caller
rule and keeping peercred-to-actor binding one-to-one. Pooling is a
measured-need decision for later, per the sizing frame.

**V6. The envelope is the captured one.** `{success, command, data,
timestamp}` and `{success: false, command, error, timestamp}` exactly as
`yeomna-cli` pinned them. Automation compatibility is a feature, the
envelope is engineering, not mythology.

### The audit write, precisely

```
BEGIN
  INSERT INTO audit_log (actor, verb, args) VALUES (...)
  <the verb's own statements, if mutating>
  <node_log appends, if node-mutating>
COMMIT
```

Read verbs use the same shape with no mutation inside. **Ruled by Todd,
2026-08-13 (V-Q1): attempt logging.** A failed verb leaves its audit row
marked failed. Mechanically: the audit insert commits in its own small
transaction before the verb executes, and an `outcome` column (default
`ok`, set to `failed` with the error name) is updated after. A crash
between the two leaves an attempt row with no outcome, which reads as
exactly what it was. The mutation itself still runs in one transaction
with its `node_log` appends. The `outcome` column is a one-line schema
amendment landing with the Phase 1 spec.

## Testing Strategy

- **Contract tests per verb**, cluster-gated with the standing skip
  pattern: known corpus in, envelope out, golden shapes pinned.
- **Audit completeness**: a test wrapper runs every implemented verb
  once and asserts `audit_log` gained exactly one row per call, with
  actor and args populated. This is G1 as a test, and it runs in CI
  forever.
- **The T3 test**: run `retire` and `prune` against a scratch graph and
  assert the audit args name the swept keys. The charter's falsifiable
  thesis becomes a literal assertion.
- **Escape-attempt tests**: hostile filter fields, oversized frames,
  unknown verbs, and a client-supplied actor field all die at the
  boundary with typed errors.
- **Peercred integration**: connect as the test uid, assert the actor
  recorded matches, single test that requires the daemon and skips
  without it.
- **Census inversion** (Phase 7): every CLI command reaches a verb or
  names its planned-unimplemented error.

## Risk Assessment

- **R1. Scope.** The charter calls this the largest block. Mitigation:
  the phases are separately mergeable, each behind the standing
  issue-and-draft-PR workflow, and Phase 1 is pure types.
- **R2. Naming churn.** V4 renames could thrash if reopened per phase.
  Mitigation: R4 closes in the Phase 1 spec and the table is binding
  after that.
- **R3. The M-questions leak in.** Traversal depth (M2) and concurrent
  ingest-versus-serve (M3) surface inside verb implementations.
  Mitigation: verbs land with the caps and isolation statements the
  store PRD fixed, and the open questions stay open by name rather than
  getting quietly resolved in a verb body.
- **R4. Daemon security surface.** A socket server is new attack
  surface. Mitigation: kernel-enforced peercred, no identity in the
  protocol, 0600 socket, `RestrictAddressFamilies`, and the
  escape-attempt test suite.
- **R5. H3 timing.** The `ingest` verb wants the orchestrator.
  Mitigation: contracts land regardless, unimplemented is a typed error,
  and H3 can proceed in parallel after Phase 1 fixes the types it must
  emit.

## Open Questions

- **V-Q1. RULED 2026-08-13: attempt logging.** A failed verb leaves an
  audit row marked failed. Mechanics in the audit section above.
- **V-Q2. RULED 2026-08-13: inherit, do not author.** The daemon carries
  zero permission logic. Peercred identity maps to a Postgres role and
  grants decide what the session can do, the same mechanism that already
  makes the audit log append-only. Read and write both exist from day
  one. Who holds which role is user and database level policy, which is
  the deployment-era directory inheritance the charter names.
- **V-Q3. RULED 2026-08-13: orient is a per-graph survey.** Point it at
  a KG and it answers what that graph is about and where it stands:
  counts by node kind, edge counts by relation and basis, embedding
  coverage and model, last ingest activity, schema version. Structured
  facts, since this system is the KG an agent consults, not an agent.
  A welcome message is a deployment decision: a config-file field (H10)
  whose text rides in the response when the deployment sets one.
- **V-Q4. RULED 2026-08-13: immutable in structure, not in content.**
  Customers create and drop databases freely, their box and their data.
  The yeomna pattern is stampable: `create-database` takes a kind, and a
  KG-pattern database gets the schema applied at birth while a plain one
  comes up empty. On KG-pattern databases, structure (tables,
  constraints, grants) is owned by `schema apply` and unreachable from
  the runtime surface, while content is fully mutable through the KG
  verbs, up to and including dropping the database, since the graph is a
  rebuildable index (T4). The scoped `sql` verb serves plain databases
  with full utility and audited statement text, and refuses KG-pattern
  databases, not to fence the customer's data but because raw SQL on KG
  content would break the guarantees they are paying for: the diff log's
  head-versus-log rule and the audit log's completeness. Every operation
  raw SQL could perform on KG content, a verb performs with the record
  intact.

## Timeline

Phases 1 and 2 are the near-term work and unblock nothing else, so they
can interleave with H3. Phase 4 before Phase 6 (the audit transaction
pattern must exist before the destructive verbs use it). Phase 5 can
land any time after Phase 1. Phase 7 last, because the census inversion
is the definition of done: when it passes, the holes ledger's 28
store-facing CLI holes retire and T3 is argued true with tests rather
than intent.
