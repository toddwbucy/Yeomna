# Review notes: 020 retire, prune, and T3

Status: build complete 2026-09-09, local review run before the PR
opened. **R23 is proposed and needs Todd**, and the build assumes it,
so declining it means reverting one field and reshaping one verb.

Editorial rules: ASCII only, no em-dashes, no semicolons, never the
words genuinely, honestly, or actually.

## What landed

`codebase.retire` and `codebase.prune`, both force-gated, both
measuring and deleting in one statement so the counts an operator reads
are the counts that went, both audited. The dispatch has no phase
refusal left.

`retire` does not decide for itself what is gone. It asks `drift`,
which walks the tree the way an ingest does, and takes only what drift
calls `missing`. That matters more than it looks: spec 019's review
found that drift was reporting present-but-unassessable files as
missing, and `missing` is exactly the list this verb consumes. Had the
two specs landed in the other order, the first `retire` over a
repository with one oversized file would have deleted the graph's
record of source that was sitting right there. There is now a test for
precisely that, and it is the one worth keeping.

## R23, proposed

`RetireRequest` gains a required `path`. The captured shape was
`{graph, prefix, force}`, which predates D3's ruling that retire
removes nodes whose sources are gone: without a tree the verb would
have to take a prefix on faith, and a typo would delete live knowledge.
With a tree it can only sweep what the source truly lacks, and a
tree that cannot be walked is a refusal rather than a licence.

The alternative, kept on the record, is prefix-only retire as the
capture wrote it, which is a coherent operator action ("decommission
this subtree") and consistent with `graph.drop` taking a whole graph
under force. It is also strictly less safe, and this is the verb the
charter names when it explains why T3 was false, so the safer shape
won the recommendation.

Declining R23 is cheap: the field comes out, the drift call goes with
it, and retire sweeps the prefix outright.

## What T3's flip rests on, and what it does not

Recorded in the spec and now in the charter, because a falsifiable
thesis being tested is worth writing down carefully.

**True:** every path in this product that writes to or deletes from the
store goes through a verb, and every call leaves one audit row with the
actor the kernel named. The charter's sentence, that a human running
`codebase retire` produces no verb-layer record of what was swept, no
longer describes this codebase. The test
`every_destructive_verb_leaves_a_record_of_its_call` asserts the row,
its actor, its terminal outcome, and that the args name the graph and
the force.

**Not a bypass, and so not a falsifier:** `embed.text` waits on H4, the
`schema.*` verbs on H7, the `graph-embed.*` verbs on H9. Each refuses
by name. They are capabilities the appliance lacks, not side doors, and
T3's clause is about the second. If one ever ships as a side door the
thesis goes false again, which is why the read-verbs guard now names
holes rather than phases.

**Still open, as R22:** the audit row carries who, when, which verb,
the request's args, and a terminal outcome. It does not carry the
substance, so "retire swept 47 nodes under `src/old`" is durable only
as "retire was called with that prefix and succeeded". The enumerated
sweep rides the response envelope, which nothing persists. For a
regulated buyer that is a gap. The shape that fits is a `result` jsonb
column written by destructive verbs inside the same transaction as the
mutation, so the record of a sweep is as durable as the sweep. It is a
schema change and a contract question, so it is proposed rather than
built. This is the third time the audit log's read and write surfaces
have come up, and the other two are in specs 017 and 018.

## Build findings

**Symbols are not a cascade, and that is the whole shape of these two
verbs.** A symbol node names its file through `payload->>'file_key'`,
not a foreign key, because the key is derived rather than referential.
Deleting a file node therefore leaves its symbols standing. `retire`
sweeps the family together for that reason, and `prune` exists as the
mop for orphans arriving by any other route, including a retire that
failed halfway.

**The shrinking-refusal guard emptied of phases.** It has fired on every
phase since 012 and it fired once more here. It now names H4, H7, and
H9, which is the milestone: what a verb waits for is a hole, never a
phase.

## Test inventory

`ingestion_verbs.rs` grew four (13 total): retire's force gate, empty
prefix, unwalkable tree, no-op, and family sweep in one test; retire
never touching a present-but-unassessable file; prune's force gate,
clean graph, planted orphan class, and second run finding nothing; and
the destructive-record test that T3 rests on. `audit.rs` covers both
new verbs. `read_verbs.rs`'s guard names holes.

## Riding items

- R22, above.
- M4 is untouched. ZFS snapshots beneath Postgres still retain what a
  sweep removed, and no verb changes that.
- Document retire does not exist, matching `codebase.drift`'s scope.
