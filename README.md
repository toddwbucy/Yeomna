# Yeomna: Charter PRD

Status: draft v0.5, 2026-09-11. Founding artifact. This document sets the
context a project is built inside: who it is for, what it is made of, what
it refuses, and why. It supersedes v0.4 (2026-08-08), which superseded v0.2
of the same day and folded in the v0.3 ratification batch from that day's
socket-boundary review session. Requirements from v0.1 (2026-06-14) that
survive are carried by reference and noted in section 11.

v0.5 adds one decision to section 12, that this appliance is developed
independently of every consumer, and one open item to section 14 that
follows from it. Nothing else in v0.4 changed. Appendix A records what
changed across versions and which changes are still awaiting ratification.

Editorial rules for this document and its descendants: ASCII only, no em-dashes,
no semicolons, never the words genuinely, honestly, or actually.

Yeomna is a codename, not the final product name. See section 13.

The reference implementation is HADES-Burn. Yeomna mines it for proven
solutions, pointed to by file throughout. Yeomna copies solutions, not
structure. Where this document and the reference disagree about what the product
should be, this document is the shape. Where they disagree about what something
costs, the reference wins.

---

## 1. The product, in one paragraph

Yeomna is a sealed appliance that turns a firm's documents and code into
queryable context and serves that context to a language model running on the
same machine. It carries its own Postgres instance, bound to a Unix socket with
no TCP surface. Everything the product does, ingestion, storage, retrieval, and
serving, happens inside that one box. A single verb layer sits over the store
and is the only surface anyone touches, human or agent alike. Nobody writes SQL.

**The store itself is commodity. The sealing is the invention, and the sorting
that happens at the sealed door is what a regulated buyer is paying for.** Every
choice in this document follows from that sentence. Nothing below is an attempt
to build a better database.

## 2. Who it is for

The buyer is a small regulated firm. A law office, a medical practice, an
accounting firm. Organizations with a document corpus that matters, a compliance
obligation attached to it, and no database administrator.

This buyer determines nearly every choice below. They cannot carry enterprise
licensing. They cannot operate a database cluster. They cannot send their client
files to a third-party cloud, and for many of them that is not a preference but
a rule they answer to. What they can do is buy a box that installs, runs, and
gets backed up by whatever IT support they already retain.

### 2.1 The credential

When a compliance officer asks what stores the data, the answer has to close the
question rather than open one. "Postgres" closes it. Decades of audited track
record, a known security posture, a documented CVE process, and a hiring pool
that outlives any single vendor. Every alternative considered in section 12
required explaining an unfamiliar name to someone whose job is to be
unconvinced.

This is a product requirement, not an engineering preference. The store is part
of the sale.

### 2.2 What is inherited and what is invented

A standing test that keeps scope from drifting. If a capability is standard
database work, Postgres has it and the appliance inherits it rather than
authoring it. Access segmentation is a directory problem, and Postgres already
integrates with the customer's Active Directory or LDAP, so authentication and
policy stay the customer's and row-level enforcement is ordinary. Clustering is
node-to-node replication that does not exist until there is a second node, and a
single node comes first, so there is nothing here to author and nothing to scope.
Retention and destruction ride foreign-key cascade and cron.

What is invented is what the appliance does with Postgres: the sealing, the
single verb surface, and the provenance model. Everything else is a part someone
else maintains.

## 3. Founding theses

Each of these is falsifiable. If one turns out to be wrong, the thing built on
it comes down with it, and that is the point of listing them.

**T1. The model is not the agent.** The model is swappable inference. The
persistent part of an agent, its memory and accumulated context, lives outside
the weights. On every turn the model reconstitutes its working context from a
store. Yeomna is that store and the machinery that fills and serves it.

**T2. Co-residence is architectural, not a deployment preference.** The latency
between a processor and its own memory is an inner-loop cost. The store belongs
on the same machine, reached over a socket. Federation happens above the
appliance, after the local work is done. The inner loop touches the wire
nowhere. What reaches the wire is only what is required to, which is a remote
model if the customer chose one, or federation above the box. A byte on the wire
that was not required to leave is a design defect, not a tuning target.

**T3. The agent is a user, and it is governed like a user.** The command surface
does not know or care which is on the other end. A person and an agent call the
same verbs, are logged the same way, and are bounded the same way. There is one
audited entry point, not two. The falsifiable part is not the intent but the
sufficiency: that one surface can serve both without a capability gap wide enough
to force a bypass.

**Tested 2026-09-09, and the clause below is answered.** Every path in this
product that writes to or deletes from the store goes through a verb, and every
call leaves one audit row carrying the actor the kernel named. The last two
destructive paths, `codebase retire` and `codebase prune`, became verbs in spec
020, which is what the sentence at the end of section 6 was waiting for. What
stays open is capability rather than surface: `embed.text` waits on H4, the
`schema.*` verbs on H7, and the `graph-embed.*` verbs on H9, and each refuses by
name rather than routing around the line. If one of them ever ships as a side
door instead of a verb, this thesis goes false again. Whether the record is rich
enough for a regulator is a separate question, raised as R22: the audit row
carries who, when, what was asked, and that it succeeded, while the enumerated
sweep rides the response envelope and is not durable.

Section 6 records the gap as it stood, which is what
makes this testable rather than aspirational.

**T4. The graph is a rebuildable index, not precious data.** It is derived from
source by ingestion. Losing it costs a re-ingest. This is what makes substrate
changes survivable and it is why Yeomna ships no migration tooling.

## 4. The store: one engine, five capabilities

There is no second store anywhere in the unit. Nothing to keep in sync, nothing
that can drift, one thing to back up, one thing to answer for.

| Capability | Mechanism |
|---|---|
| Documents | JSONB, GIN-indexed, queryable to nested paths, no per-shape migration |
| Embeddings | pgvector |
| Keyword retrieval | native FTS, tsvector plus GIN, ranked with ts_rank_cd |
| Graph | ordinary tables, reached by recursive CTEs |
| Transactions | Postgres, across all of the above |

JSONB is a real document model that happens to share an engine with everything
else. Calling it a downgrade to rows and columns misreads what it is. The
practical test is that a document whose shape nobody anticipated stores and
queries without a schema change, and JSONB passes it.

Two of these are upgrades over the reference rather than ports of it, which
matters when setting expectations:

- **Keyword retrieval.** The reference has no engine full-text search at all. Its
  hybrid path computes term coverage in Rust, with no term frequency, no IDF, and
  no length normalization. tsvector with ts_rank_cd is strictly more.
- **Vector indexing.** The reference's ANN module is 444 lines of dead code with
  zero callers. Its live search path brute-forces in Rust and refuses past a
  100,000-embedding ceiling. pgvector delivers a capability the product has never
  had.

## 5. The sealed box

The appliance ships with its own Postgres instance. This is a reversal of an
earlier ruling and the distinction that justifies it is narrow and load-bearing:

**Shipping stock Postgres is upstream's binary and upstream's CVE feed. Shipping
a custom-compiled Postgres is a build pipeline owned forever.** Only the second
is the trap. Everything in the unit is assembled from stock parts that other
people maintain.

Consequences, all deliberate:

- Bound to a Unix socket. No TCP surface. There is no network port to secure
  because there is no network port.
- Backup, restore, and replication ride on standard Postgres tooling that a
  regulated buyer's IT staff already recognizes. Nothing bespoke to learn and
  nothing bespoke to trust.
- Controlling the instance means controlling extension versions. This retires a
  constraint that would otherwise bind: at the customer's-Postgres floor,
  Ubuntu 24.04 ships pgvector 0.6.0, which cannot HNSW-index a 2048-dimension
  embedding (verified, "column cannot have more than 2000 dimensions for hnsw
  index"). Shipping pgvector 0.7 or later with halfvec raises the ceiling to
  4000 and stores fp16, and fp16 is already the reference's configured default.
- No CREATE EXTENSION prerequisite for the customer. pgvector is not a trusted
  extension and would otherwise require a superuser step from someone who does
  not have a DBA.

### 5.1 What never leaves the box

For a regulated buyer this is the second question after "what stores the data,"
and it deserves an answer in the same words every time it is asked.

**The corpus never leaves.** No cloud store, no external search cluster, no
managed database, no third-party vector service. There is nowhere for it to go,
because the design has no component outside the unit. This is section 9's "no
second store" restated as a privacy property rather than an operational one, and
it is the same property viewed from the buyer's side.

**Embedding is local.** The embedder is GPU-resident on the same machine, so
document text is never transmitted to an embedding API. This is a requirement,
not a configuration default.

**There is no network listener.** Unix socket only. Nothing can connect to the
store from off the machine because nothing can connect to it at all except a
process on that machine holding filesystem permission to the socket.

**No telemetry and no phone-home.** The appliance reports nothing to the vendor.
Usage data, error traces, corpus statistics, and query logs stay local. If
support ever needs diagnostics, the customer exports them deliberately.

Three boundaries are real and must be stated rather than implied, because each
is a place where data can leave and the appliance is not the thing deciding:

1. **Model inference.** T2 assumes a co-resident model, and with one the loop is
   closed. If a customer points the appliance at a hosted model API instead, the
   retrieved context leaves the building on every turn. The appliance cannot
   prevent this and should not pretend to. It should make the distinction
   visible, refuse to treat a remote model as the default, and be explicit in
   the compliance conversation that a remote model relocates the boundary.
2. **Backups.** A Postgres dump contains the corpus. Riding on standard tooling
   means the destination is chosen by the customer's IT staff, which is correct,
   and it means the appliance must not default to any remote destination.
3. **Updates.** Patching the appliance is a network path by definition. It
   carries software in and must carry nothing out. Update transport is a design
   item, not a settled property, and it is named here so it is not discovered
   during a security review.

### 5.2 Default-deny, and the first named opening

The backend is default-deny on the network, permanently. It terminates at a Unix
socket, and anything that needs the wire lives on the far side of that socket and
owns that concern itself. Network access is per-feature, opt-in, and front-end,
and each opening is named here and secured at the router and at the machine
before traffic is routed to the socket. The system starts sealed and opens one
path for one purpose when a feature earns it, which is the reverse of shipping
open and securing after. Physical security sits underneath the network layer as
the boundary software cannot promise and the operator secures by hand.

GitHub is the first named opening and it sets the pattern. It is an ingestion
source, not a backend integration. The operator clones a repository, which is a
deliberate front-end fetch, after which the code is local and its issue and pull
request data is graphed as any other local corpus. The appliance makes no live
backend call to GitHub. The front end may render a stored URL as a link that
resolves in the user's own browser under the user's own credentials. The box
stays sealed.

A remote model reaches the store the same way. A co-resident model speaks to the
socket directly. A model on another machine reaches the store through a front-end
service on the far side of the socket, which is where an MCP interface would
live. MCP is a front-end concern and is parked, not current work.

## 6. The verb layer

A single verb layer sits over the store and is the only surface anyone touches.
The agent calls those verbs. A human calls the same verbs. Neither writes SQL,
neither writes a JSONB path expression, neither hand-builds a traversal or a
search. The verbs emit the recursive CTEs, the JSONB lookups, the vector
queries, and the FTS calls underneath.

Three things follow, and the third is the one that pays for the other two.

**One audited entry point.** When an auditor asks who did what, there is one log
and one command set to reason about. This is a compliance property, not a
convenience. The product's central claim rests on that log, so its integrity is a
shipped default rather than a deployment option: the audit log is append-only,
update and delete are revoked, and its owning role is separated so that the
appliance operator is not the log's owner. Every one of those mechanisms is stock
Postgres, and none of them is on by default, which is why the charter commits to
turning them on rather than leaving them to the spec.

**Everything below the line is swappable.** Recursive CTEs today and a graph
extension later change nothing above the verb line. Native FTS today and
pg_search later change nothing above it either. No caller sees the substrate
move. The verb layer is a socket boundary. A caller depends on the contract at
the socket and never on what sits behind it, which is the same boundary the
backend holds against the network, applied one level in. Everything below the
line is swappable for that reason and not as a separate convenience.

**The line did not exist and had to be built.** In the reference, six CLI
commands reach the store directly and never construct a verb: codebase ingest,
retire, prune, drift, validate, and graph-embed update. Roughly 25 of 57 query
sites sit outside the verb boundary, and they are the destructive ones. A human
running `codebase retire` there produces no verb-layer record of what was swept.

That last point was a requirement, not a note. T3 was false until it was fixed,
and it was the largest single block of work in the build. It is also the reason
the work is worth funding: it is not a portability concern, it is what the
regulated sale rests on.

**Built, 2026-09-09.** Five of the six are verbs (specs 019 and 020: ingest,
retire, prune, drift, validate), the sixth waits on H9 as a capability rather
than a bypass, and nothing in this product reaches the store without
constructing a verb. `codebase retire` leaves an audit row naming its actor, its
graph, its prefix, and that force was given.

## 7. The graph, without Cypher

The graph lives in ordinary Postgres tables. Edges are rows. Traversal is a
recursive CTE with an explicit visited-set and a row cap.

This is sufficient because of the shape of the workload, not because of
optimism about it. Measured on the reference's live graphs:

- Largest graph: 18,506 documents and 19,024 edges.
- Real traversal depth: 1 to 3. The verb caps at 20.
- Edge types: at most 24 in use in any one graph.
- Embeddings: a few thousand per graph, 2048 dimensions.

Recursive CTEs were designed for typed, bounded traversal over known edge types.
Cypher and its relatives earn their keep on unbounded exploratory pattern
matching over unknown shapes, which is not this workload.

What is given up, stated plainly so it is a decision and not a discovery:
authoring ergonomics, engine-native traversal operators, and graph algorithms.
The last of these costs nothing here, because structural embeddings run in a
separate process on tensors and the store only has to scan edges.

## 8. Provenance

Deterministic edges and inferred edges must never be mistaken for each other.
This is a trust boundary and it does not need to be a storage boundary. Two
mechanisms carry it. Current state is partitioned by basis. History is an
append-only diff log.

### 8.1 Current state: basis is the partition key, status is a column

Edges live in one logical table partitioned by LIST on basis: declared,
structural, asserted. Status (ratified, pending) is an ordinary mutable column.

Verified by test at the reference's graph shape:

- Partition pruning survives the recursive term. A traversal restricted to
  declared and structural never touches the asserted partition, in the base term
  or the recursive term. The deterministic subgraph is not filtered, it is not
  visited.
- Changing an edge's basis is a plain UPDATE with automatic row movement between
  partitions. Measured at 2.25 ms for the realistic case. The design does not
  have to claim an immutability it cannot keep.
- Status changes are in place, with no row movement.
- Absence queries stay honest. A missing declared edge is a missing row in a
  pruned partition, which is a stronger guarantee than a row failing a predicate.

### 8.2 History: an append-only diff log per node

The diff log is the single source of truth for history. It records what a node
was on entering the store, every change since, and by extension what it is now.
The head row is not a competing source of truth. It is a convenience, current
state materialized so that a read does not replay a chain. If head and log
disagree, the log wins, because the head is derivable from the log and the log is
not derivable from the head. This is the relation git holds between a working
tree and a commit history, and the same rule about which of the two is authority.

This resolves the tension the partition model left open. Basis mutability by
plain UPDATE was under-specified rather than wrong. A basis change is a row move
for the head, which is the 2.25 ms measurement above, plus a log entry for
history, so the prior basis stays recoverable. The design does not have to claim
an immutability it cannot keep and does not have to lose the prior value either.

Git is borrowed vocabulary, not a borrowed engine. The backend grows exactly one
thing, history as diffs instead of overwrites, served on a get. Replay, rollback,
blame, and walkthrough are front-end renderings of that chain. The backend never
grows a git engine. A developer already thinks in commits and rollback, which is
why the vocabulary is worth borrowing at the surface even though nothing beneath
it is git.

### 8.3 Two replay axes, and where each kind of version lives

The diff log makes a capability available that the store was not built for.
Select a node, step its log back one entry, and let that node's content revert
while every other node in the graph stays frozen. That node's edges recompute
from the prior content against a still background. Call it differential
provenance: one document's relationships moving through time while the rest of
the corpus holds. The question it answers is whether a past version of a document
sat closer to the rest of the corpus than today's version does, which is not
answerable by reading either version.

What computes the relationships is an inductive graph model, GraphSAGE by current
selection, which derives a node's neighborhood from its features rather than from
a learned fixed identity. Compute is paid once. The first walk computes each
step, the computed states cache alongside the diff log, and every replay after
that is a read.

The cache is keyed by model version, and that is a feature rather than a
staleness problem. A new model version does not invalidate a cached relationship
state, it labels it. Keeping every cached state, keyed by the pair of content
version and model version, yields a second replay axis: hold the content fixed
and step the model version instead. That answers a different question. Why does
the system link these two documents, did it always, and under which model was
that judgment made. It is version control of the interpretation rather than of
the text. Both axes are replay of saved states, so the backend grows only an
append-only log of states keyed by that pair.

The two kinds of version do not share a mechanism, and conflating them is a trap
worth naming here. A document diff is semantic and worth computing, so it is
application-level. A model checkpoint has no meaningful semantic diff, because
training moves weights everywhere and a naive delta approaches the size of the
whole checkpoint. Checkpoint history is therefore block-level and belongs to the
filesystem: a copy-on-write snapshot per model version on ZFS or equivalent,
which shares unchanged blocks and stores only changed ones without needing to
know what a byte means.

That draws a boundary in both directions. Snapshotting is a deployment concern
underneath the appliance. The application names which model version it wants and
does not orchestrate snapshots. This is the same move the design makes three
times: drop to the layer where the capability already exists as a primitive and
inherit it. Clustering drops to Postgres. Directory and access policy drops to
Postgres. Checkpoint versioning drops to the filesystem.

### 8.4 Basis is derived at ingest, never inferred from edge type

**Basis must be derived at ingest, from the analyzer, and written explicitly.**
It is not a function of edge type. In the reference's live graph,
codebase_calls_edges holds 6,129 edges attributed to rust-analyzer or libclang
and 118 with no analyzer and no resolution recorded, in the same collection. A
second graph, NestedLearning, carries no provenance attribution on any code edge
at all. Any claim that the deterministic subgraph is identifiable by edge type
is false in the data that exists today.

The 118 are residue from ingesting before a better extractor covered that path.
Provenance therefore changes on extractor upgrade as normal operation, which is
why basis is mutable by design and why the change is logged rather than lost.

## 9. What is deliberately excluded

Two of these exclusions are contested and each carries a pre-registered
graduation trigger, which is what makes those two decisions rather than
aversions. The other three follow directly from a founding thesis and graduate
only if that thesis is overturned.

**Apache AGE.** Excluded because it fragments the one property the design is
built on. AGE stores graphs in ag_catalog with properties as agtype, so a
pgvector distance operator cannot participate in a Cypher block. Hybrid
retrieval becomes two phases. AGE is a sixth store wearing the same process.
Its one structural advantage, free provenance partitioning by label, was
falsified in section 8 and then supplied by declarative partitioning instead.
Secondary concerns: the packaged build is a release candidate in a community
repository, and upstream support lags Postgres majors.

  *Graduation trigger:* the day AGE can participate in a single statement with
  pgvector. Not the day it ships a package, which it already has.

**Elasticsearch.** Excluded because it solves a scale-and-relevance problem not
yet measured into, and it drags an external service and a sync surface into a
design whose entire point is one sealed store.

  *Graduation trigger:* a measured query on a real document set where native FTS
  relevance falls short. The graduation is pg_search from an active upstream,
  inside the same engine. It is not a second store.

**A second store of any kind.** No separate vector database, no separate search
cluster, no cache tier. Every one of them reintroduces sync and drift.

**Migration tooling.** Per T4. Existing graphs stay where they are on the
reference implementation. New graphs are built by re-ingesting from source.
There is no migrate command and no data-fidelity requirement.

**An inbound network surface, and any internal network listener.** The internal
fabric is socket-only and binds no network listener. The appliance has no port
anything can connect to. Its own outbound calls, named in 5.1, are a separate and
permitted thing. Bound to a socket, Postgres does not listen on TCP, so the store
is not reachable across the network even from the same machine. The discriminator
is the internal fabric versus the appliance as a unit, not inward versus outward
traffic.

## 10. Open measurements

Held in keeping with the adversarial rule. Each is a benchmark against real
query shapes, not a whiteboard call, and each has a named graduation path that
does not reintroduce a second store.

**M1. Native FTS relevance.** Is tsvector with ts_rank_cd good enough on a real
firm's document set? The bar is "better than term coverage in Rust," which is
what the reference does, not "as good as Elasticsearch." Graduation: pg_search.

**M2. Recursive CTE behavior at depth.** Do hand-rolled CTEs stay clean on deep
variable-length traversal? Partition pruning through the recursive term is
already confirmed. What is untested is cycle behavior and row growth at depth on
a real code graph, where calls edges do cycle. The benchmark should try to blow
up the depth-20 ceiling rather than confirm that depth 3 works. Measured
2026-09-09: it does not blow up, on the real graph or at depth 100, because a
real call graph saturates by depth 5. See
`docs/measurements/M2-recursive-cte-at-depth.md` for the tables and the reopen
condition.

**M3. Ingest isolation.** codebase retire becomes a multi-statement transaction
rather than a single atomic statement, so its isolation level is now a choice
somebody makes rather than something the engine picks. Write-heavy ingest runs
against read-heavy serving in one instance. This is a correctness surface and a
defect here is silent.

**M4. Erasure against an append-only history.** A regulated erasure obligation
wants data gone. The diff log of section 8.2 wants nothing lost. The same
mechanism that makes the audit trail tamper-evident reopens the erasure problem,
and erasure must also reach the asserted partition or a deleted document leaves
inferred edges pointing at a ghost. The erasure path is the single named
exception: it tombstones without destroying the chain's integrity. Unsolved,
named here so it is not discovered during a compliance review.

Retired, not open: the pgvector dimension ceiling (section 5), and whether plain
Postgres is fast enough against the remembered ArangoDB baseline. The second is
retired because the baseline was never a graph-engine result. The reference
brute-forces cosine over a few thousand vectors in Rust, so the comparison was
against three thousand dot products.

## 11. Harvested versus built fresh

**Harvested from the reference, solutions not structure:** the batch ingestion
pipeline (resumable, fault-isolated), document support (PDF, LaTeX, markdown,
plain text), code analysis (rust-analyzer, syn, rustpython, gopls, libclang,
tree-sitter), chunking strategies including late chunking, the Jina V4 embedder
service contract, deterministic key derivation, the Unix-socket daemon and its
framing, the bounded operation vocabulary, and access tiers. The document
workflow is tight in the reference and comes over close to as-is, with AQL to SQL
a near one-to-one conversion.

**Built fresh:** the store layer entire, the verb layer as a single audited
surface covering ingestion as well as retrieval, and the provenance model.

**Left behind:** the ArangoDB client, the dead ANN module, the graph
methodology layer, task management, and the reference's read/write socket split,
which is inert in the reference and is replaced by Postgres roles.

The reference is not going away. It stays as reference code and as the fallback
for a customer already running ArangoDB. ArangoDB is the problem, and everything
else around the reference is solid and a hard sell as it stands.

## 12. Decision record

This section exists so the following are not re-opened without new evidence.

**ArangoDB, rejected on licensing.** Community caps at 100 GiB aggregate and
bars commercial production use, so the intended buyer lands on enterprise
pricing. It is also an unfamiliar name in a sale where familiarity is the point.
No claim in the design rides on it. The data on the reference's ArangoDB is test
data, rebuildable in a weekend, so no switching cost holds the project to the old
store. The migration objection is dead before it is raised.

**Neo4j, rejected on where its paid line falls.** Its price ladder is friendlier
than ArangoDB's, which was the reasonable attraction. But the vector index is
Enterprise and Aura only, so the free self-hosted tier lacks the core operation,
and the tier that has it is managed cloud, which contradicts T2 and sends a
firm's documents off-premises. Self-hosted with vectors means an Enterprise
license per customer or an OEM agreement, either of which makes every sale a
Neo4j sale and puts pricing power in someone else's hands. GPLv3 was not the
objection and was withdrawn.

**Kuzu, SurrealDB, Memgraph, FalkorDB, rejected as a class.** Every
purpose-built graph-and-vector store that is free is either young and
single-vendor sponsored or source-available under BSL or SSPL. Kuzu was MIT and
was archived in October 2025 after its sponsor was acquired, which demonstrates
that a permissive license protects the code and not the user. The durable
property is distributed maintainership, meaning no single party whose exit ends
the project. Postgres and SQLite are the stores that have it.

**SQLite, viable and not chosen.** It passes the durability test and has no
operational cost at all. It loses on section 2.1: it is not the name that closes
a compliance question, and multi-user serving is not its shape. sqlite-vec is
also pre-v1 alpha with an unstable storage format. Worth revisiting only if the
product ever targets a single-developer deployment.

**Developed independently of every consumer, including the one that drove
its resumption.** Ruled 2026-09-11. Yeomna is a standalone product, not a
component or a deliverable of any project that uses it. Its roadmap, its
release cadence, and its version are its own.

The reason is stronger than clean boundaries, and it comes from what a
consumer needs rather than from what this project prefers. An instrument
that establishes baseline behaviour for an agent has to hold its substrate
fixed, because the deliverable is attribution: when behaviour shifts, the
instrument must be able to say whether the agent moved or the ground did.
A substrate co-developed with the experiment that measures against it is a
confound by construction, so it cannot be co-developed and still be a
substrate. This is the same rule as the package pin of section 5, which
refuses upstream's cadence during development, raised one level to refuse
a consumer's cadence.

Two consequences, both binding. **A consumer is a caller and never a
driver**, so what gets built next is decided by this product's own order
and not by what a consumer needs this week. And **the substrate owes its
consumers a pinnable identity**, which is the open item added to section 14
below: retrieval behaviour is what a consumer measures through, so a change
to ranking is a change to their instrument, and it has to be declarable
rather than discovered.

Nothing here loosens the boundary already drawn in the other direction. A
consumer is never a component supplier to this product either.

**Premises falsified during design, recorded so they are not re-imported:**

1. There were no fusion-splitter verbs, because nothing was fused. The
   reference's search verb is four round trips plus a Rust loop. Postgres fuses
   what ArangoDB split.
2. The ANN layer was dead code. The hardest-looking module cost nothing because
   it should be deleted.
3. The verb surface was not the migration boundary. Roughly 25 of 57 query
   sites sit outside it, and they are the ingestion path.
4. Provenance is not a function of edge type, per section 8.
5. A store interface does not supply the substrate-choice control. An interface
   creates the ability to choose a backend. The control is a prohibition on
   changing one, and making migration mechanically easy makes the prohibition
   harder to hold. If that control is wanted, it needs a provisioning-time stamp
   that refuses to change and fails loud.

## 13. Naming

Yeomna is adopted as the **codename** for the project, not as the final product
name. The final name is deferred to public release. The repository stays private
until then, so the cost of carrying a codename that may not survive is close to
zero, and the availability check is rerun at release rather than treated as
settled now.

Spelling is Yeomna, the standard romanization of the Korean. That is the string
that was checked, and it is the string that belongs in the repository, in crate
metadata, and in any recheck.

### 13.1 How it was reached

The name was derived from the product rather than chosen abstractly. Working the
IS and IS-NOT fields surfaced an asymmetry. The load-bearing IS-NOTs are refusals
of an opening: no listening port, no second store, no direct query surface, no
cloud, no customer-administered database. The remainder, migration tooling above
all, are consequences of a thesis rather than refusals of an opening, and section
9 keeps that distinction. The store is commodity. The sealing is the invention.

That put the naming target on the act where holding and judging are one motion.
Nothing is stored in the ordinary sense. Things are received into the record and
sorted at the instant of reception by how far they may be trusted. What is fixed
is not the verdict but the requirement: a fact can never sit outside a basis.
The partition is compulsory even where it is revisable, which is why provenance
is not a field beside the fact but the ground the fact stands on, and why a
deterministic edge and an inferred edge cannot occupy the same ground.

Yeomna is the Korean judge of the dead, and the alignment is with the mechanism
rather than the office. Judgment there rests on records kept by underworld
scribes and on a mirror that separates truth from deception at the moment of
reckoning. The verdict is only as good as the register behind it. That is the
partition key in mythic dress.

### 13.2 Rejected, and closed

Reopening one of these requires a reason not listed here.

**Postgres-derived names**, including PGKB and variants. Rejected on ethical
grounds ahead of legal risk. The PostgreSQL Community Association holds
registered marks on PostgreSQL and Postgres and asks that no mark containing a
variant be registered. Beyond the policy, leaning the brand on Postgres borrows
credibility the project has not earned.

**Themis**, the Titaness of settled order. Closest Greek fit and reads clean,
but the crate is taken by a wrapper for a cryptographic services library
covering secure data exchange and storage protection. Adjacent enough to confuse
a buyer who searches the name.

**Forseti**, the Norse arbiter whose function is reconciliation rather than
punishment. The crate is taken by a multi-language linter with pluggable engines
and rulesets, which is a judgment tool applying standing rules to input. Same
collision, closer still.

**adit.** Differs from *audit* by one character, and audit is the most repeated
concept in a compliance product's own documentation, which is what the
visual-collision rule exists to prevent. It also now names the store primitive
extracted from weaver-memory, which is passive and decides nothing, while this
product ingests, retires, prunes, ratifies, and serves. Carrying a primitive's
name onto an application is the drift the vocabulary rules are written to catch.

**Threshold-family candidates**, including narthex, vestibule, transom,
bailment, and cellarer. They name the door without naming the sorting, and the
sorting is the half a buyer pays for.

### 13.3 Known risks, accepted for codename use

Recorded so they are not presented as discoveries later. Each is a reason to
retest at release, not a reason to reopen now.

**Yeomna trips the same two rules that closed three candidates above.** The
transposition against *yeoman* is tighter than adit against audit, sharing four
leading characters. And Yeoman is a live JavaScript scaffolding tool with over
5,000 generators and more than 13,000 dependent npm packages, which is a larger
and more searched adjacent name than either the Themis or Forseti collision.
crates.io returns zero for `yeomna`, but crates.io matches exactly and search
engines do not.

**Pronunciation is not recoverable from the spelling.** The `yeo` digraph is a
romanization of a single Korean vowel sounding roughly like "yuh," so an English
reader has no path from the written form to the spoken one. The name has to be
taught once to every buyer.

**The mythology is the predecessor's category.** HADES was a Greek god of the
underworld and Yeomna is the Korean judge of the dead. The stated reason for
moving off HADES was to drop the mythology. The mechanism-not-office defense in
13.1 holds on its merits, and no buyer reads the defense.

**The medical half of the buyer set.** The soul-judgment origin is darker than a
medical practice will want in a sales conversation. Whether the story can be
told as the incorruptible record rather than as judgment of the dead is
unresolved. It does not block a codename and it would block a product name.

**Availability was checked once.** `yeomna` returned clear on crates.io in
August 2026. No claim is made about npm, GitHub, or trademark in any
jurisdiction. Recheck at release.

## 14. Status and next steps

This charter is a draft. Appendix A marks which of its v0.4 changes are ratified
and which are folded pending a ruling.

Open, and not for this document to close:

- The final product name, per section 13.
- Whether the reranker and the categorizer ship as first-class stages or as
  contract-defined empty slots, where a slot is defined by a wire contract and
  never by an implementation.
- Whether a client appliance calls up to a shared model for an inductive
  embedding or carries a frozen local copy.
- Whether the substrate-choice control of section 12 item 5 is wanted, and who
  owns it.
- **What a version of this appliance means, and what counts as a
  retrieval-affecting change.** Raised 2026-09-11 by the independence
  ruling in section 12. A consumer that measures through this appliance
  needs to record which one it measured through, and today there is no
  number that says: the crates are all at an initial version and the only
  meaningful marker is the schema version, which tracks tables rather than
  ranking. The shape of the answer is the one R26 already used a level
  down, where a vector is comparable to a corpus only when the model, the
  model revision, and the task all match. The same triple is wanted here,
  at the appliance rather than the row.

Next: a technical spec beneath this charter covering the schema, the verb
inventory, and the ingestion operation set. Then the first slice, which the
prior PRD recommended be chunking plus code analysis as pure libraries with no
store dependency, and that recommendation still holds.

## Appendix A. Change log

### v0.4 to v0.5, 2026-09-11

One decision and one open item, both from the same session.

Section 12 gains **developed independently of every consumer**. The
reasoning is not about clean boundaries: an instrument that establishes
baseline behaviour for an agent has to hold its substrate fixed, because
its deliverable is attribution, and a substrate co-developed with the
experiment measuring against it is a confound by construction. It is the
package pin of section 5 raised a level, refusing a consumer's cadence
rather than upstream's. Two binding consequences: a consumer is a caller
and never a driver, and the substrate owes its consumers a pinnable
identity.

Section 14 gains the open item that second consequence creates: **what a
version of this appliance means, and what counts as a retrieval-affecting
change.** Unratified and needing a ruling. The shape of the answer is
R26's, one level up.

Nothing else moved. The charter never named a consumer, so this is an
addition rather than a correction, and the entanglement it answers lived
in the working notes rather than here.

### v0.2 to v0.4

Ratified in session and applied here: T2 rewritten from a size claim to a
necessity claim (section 3). The verb-layer swappability boundary named as a
socket boundary (section 6). Section 9's exclusion opener scoped to the two
contested exclusions. Section 9's network line rewritten to inbound surface and
internal listener. Near-zero ArangoDB switching cost recorded (section 12).
Default-deny and the GitHub opening seated as 5.2. Provenance as an append-only
diff log with the authority model settled, the two replay axes, and the
application-level versus filesystem-level version ruling (sections 8.2 and 8.3).
Inherited-versus-invented test seated as 2.2, which closes the access
segmentation and clustering questions without giving either a charter section.

Folded pending a ruling, strike or keep on the next pass:

1. Audit-log integrity committed as a shipped default in section 6, rather than
   left to the spec. The mechanisms are stock Postgres and none is on by
   default, and the product's core claim rests on the log, which is the argument
   for committing in the charter.
2. T3 given an explicit falsifiable clause (sufficiency of one surface), which
   answers the review finding that T3 read as a design commitment rather than a
   thesis.
3. Section 13.1's IS-NOT claim softened to the load-bearing IS-NOTs, since
   section 9 shows that not every exclusion refuses an opening.

Not carried forward, on purpose: clustering earns no exclusion entry and no
mention, because it is a Postgres mechanic that does not exist until a second
node and it reopens nothing. Access segmentation likewise, per 2.2.
