# Review notes: 022 The Native Embedder

Written during and after the build, before the PR opened. Editorial rules
as the spec's.

---

## What the local review pass was for

Todd's standing instruction: run the commits through a review before
opening the PR, to catch the class of thing CodeRabbit found last time
rather than discovering it in review. This file records what that pass
found, plus what dogfooding found, plus the two places where a decision
made during the build differs from the spec as drafted.

## Found by dogfooding, not by a test

**An empty source file failed the whole ingest.** The first embedding run
over a tree containing an empty `.rs` file returned:

```
internal: ingest failed: embedding failed: embedder refused (invalid-request):
input at index 0 is the empty string, which has no embedding
```

The service is right to refuse an empty input, because a zero vector has no
cosine and would answer every query equally badly from inside an HNSW
index. The ingest path was wrong to treat that refusal as fatal. Real
repositories have empty files: placeholder modules, generated stubs,
`__init__.py`. The local chunker already returns no chunks for blank text,
so one path treated an empty file as nothing to do and the other treated it
as a failed run.

Fixed in both paths: the embedder is not asked about text with no
non-whitespace bytes. The first fix also treated any `invalid-request` from
this call site as "nothing embeddable", and the local review pass showed
why that was wrong and it was removed (see below). Regression test:
`an_empty_source_file_does_not_fail_an_embedding_run`, which runs against
the double and therefore without a GPU.

This is the third instance in this project of a defect that existed only
because the consumer that would exercise it had not arrived yet. The other
two were the boxed future with no `Send` bound and the drift report calling
unassessable files missing. Each was caught by the next thing to be built,
which is the argument for building the next thing.

## Found by asking what the new columns are for

R26 added `model_revision` and `task` to `embeddings`. Two readers were
already asking a weaker version of the question those columns answer, and
leaving them alone would have made the columns decoration.

**`codebase.validate` checked for one model where the invariant is one
cohort.** It set `ok = ... && models.len() <= 1`. Two vectors from the same
model at another revision or under another LoRA adapter share a name and
have incomparable geometry, so a graph holding both would have reported
`ok` while holding vectors that cannot be ranked against a single query
vector. It now checks distinct `(model, model_revision, task)` triples and
names them in its report. Test:
`validate_refuses_a_graph_holding_two_cohorts`, which plants two cohorts
differing once by revision and once by task.

**`orient` reported a bare list of model names.** It now reports the cohort
triple with a count per cohort, which is what a reader deciding whether a
graph can be searched with a given query vector needs.

## Where the spec as drafted and the build differ

**Phases 1 and 2 became one spec.** The spec was drafted with the ingest
wiring out of scope. That did not survive: `EmbeddingClient::embed` changed
shape, so ingest had to change in the same commit or be written against the
new client in the old per-chunk style and deleted one PR later. The spec
header and its Out of Scope list were corrected rather than left to
contradict the diff.

**The claim about which verb shows the coverage gap was wrong.** The PRD
said `read::health` reports `chunks_without_embeddings`, so the gap left by
an over-ceiling document is visible. It does, and it counts across the whole
database, so on a multi-graph appliance it cannot say which graph is short.
`orient` is the graph-scoped one and it reports chunks against embeddings
per graph. Corrected in the PRD, and checked against the real corpus:
`wt_embed` shows 1,480 chunks and 1,275 vectors, which is the 5
over-ceiling documents' 205 chunks.

## Two conditions the first draft of the contract did not name

Both came out of building the service and both are now in
`docs/embedding-contract.md`, because a contract that omits them describes
a service nobody can implement twice the same way.

**The ceiling has a prerequisite.** 16,384 tokens holds only with
`PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True`. Under the default
caching allocator a fresh process embedded 13,993 tokens and then refused
14,500 with `out-of-memory`, so the real ceiling was about 14k and the
service would have been advertising a `max_tokens` it could not reach. The
service sets the variable itself before importing torch rather than
trusting its unit file, because an unpredictable refusal is worse than a
low ceiling.

**The revision pin does not cover the LoRA adapters.** The model's
`from_pretrained` downloads `adapters/*` with no revision when handed a
repository id, resolving through `refs/main` while the weights resolve
through the pin. The spike passed a repository id, and on this machine
`refs/main` happens to be the pinned SHA, so **the spike's numbers stand by
coincidence rather than by its pin.** The service resolves the snapshot
directory itself and hands `from_pretrained` a path, which reads the
adapters from inside the pinned snapshot and removes the download call, and
so satisfies R25 without a library call that could reach the network.

## Four defects in the spike, found by building on it

Recorded in full in the PRD's background section. Summarised: pooling with
no attention mask (correct only for one unpadded input, wrong for a batch),
the adapter pin above, prefix tokens pooled into the vector (harmless for a
document-level identity check, wrong for a chunk whose span must start at
byte 0), and a hard-coded nine-byte prefix when `"Query: "` is seven and
three of the four tasks use it.

None of this makes the spike wrong about what it measured. It is what
happens when evidence gets built on, and it is the reason the spike said
"nothing here ships" in its first paragraph.

## Raised and left open

**R28: does the `code` task need a query and passage split.** The model's
snapshot fixes prefixes for `Query` and `Passage` and forces `Query` for
`text-matching`, and fixes nothing for `code`, so the service uses `Query`
and says so. Whether that is right is a retrieval-quality question about
whether ingested code is a passage. It is cheap to hold open because R26 put
the task on the row, so whichever way it is ruled a reader can tell which
corpora were embedded under the old answer. Left for Todd rather than
decided quietly.

**The tokenizer hazard.** transformers 4.57 warns that this snapshot's
tokenizer regex is incorrect and offers a flag to fix it. The flag is not
set, because the committed `tokenizer.json` is part of the identity the
revision pins and changing how it splits text would change every offset and
vector in a corpus keyed by `model_revision`. It is the one drift a revision
cannot detect, and the thing that does cover it is the committed dependency
lock. Worth a ruling if the lock is ever loosened.

## Found by the local review pass

Ten findings, one high, and the high one was a regression this spec
introduced.

**The `errors > 0` check aborted a re-ingest without `overwrite`.** The
PRD was right that the live ingest path discarded the sink's rejection
count for chunks and embeddings, and the fix that replaced the discard was
wrong. `InsertOutcome::errors` counts every row the sink did not create,
and without `overwrite` the insert is `ON CONFLICT DO NOTHING`, so a row
that was already there is indistinguishable from one that was refused.
`IngestRequest::overwrite` defaults to false, so the second ingest of a
changed file aborted, and aborted partway through with earlier files
already committed. The check is gated on `overwrite` now, where the insert
upserts and a non-creation can only mean a rejection. Test:
`a_second_ingest_without_overwrite_does_not_abort`, verified to fail with
the exact message when the guard is removed and pass with it back.

**The `invalid-request` fallback re-created the shape it was meant to
kill.** Written on the premise that the only input either path sends is
the file's own text, so `invalid-request` could only mean "nothing
embeddable". The reviewer probed the live service and found that
whitespace-only input is not refused at all, it returns a real vector, so
after the `trim().is_empty()` guard the arm had no legitimate trigger left.
Worse, `chunk_size_tokens` below 1 is also `invalid-request`, and
`CodebaseConfig::chunking` is a public field, so a caller passing a zero
window would have had every file fall through to keyword-only with
`embeddings_written: 0` and `success: true`. That is the silent drop the
PRD's summary says Phase 2 exists to fix, one layer up. The arm is gone
and the guard stays.

**`query --hybrid` still refused by naming H4**, which this spec fills, so
a caller was pointed at a closed hole. It names PRD Phase 3 now, and the
shrinking-refusal guard's case list follows. Two sibling assertions were
stale the same way and were corrected rather than deleted.

**`parse_endpoint` had no production caller.** The config value went
straight into a `PathBuf`, so FR9's refusal was unreachable by an operator
and `unix:///run/yeomna/embedder.sock` (the spelling the contract and this
crate's own documentation use) became a literal relative path.
`EmbeddingClientConfig::from_config` is the seam now, verified by config
file: a URL is refused with charter 5.1 in the message, and both spellings
reach the socket.

**`embedder_for` did not check `loaded`.** A client connects while the
weights are still loading, on purpose. An ingest started on one wrote the
first file's nodes and symbols, then died on `model-not-loaded`, leaving
the half-ingest EC-4 exists to prevent. Checked before the walk now, beside
the task check and for the same reason.

**The service's routing did not match the contract.** `GET /v1/embed`
answered 404 rather than 405, and `DELETE`, `PUT`, `PATCH`, and `OPTIONS`
fell through to `BaseHTTPRequestHandler`, which answers 501 with an HTML
error page and no envelope. That was the one way out of the service that
was not the contract's shape. Every method routes through one place now,
405 when the path is an operation reached the wrong way and 404 when it is
not an operation, both with the envelope, checked by two new tests.

**The batching machinery was unreachable and untested.** `embed`'s work
queue, index rebasing, reassembly, and out-of-memory halving existed for a
caller that does not exist: every path in the workspace hands it one input.
Untested machinery in the path a future batched caller will take is worse
than none, so it is driven now by
`a_multi_input_batch_reassembles_in_input_order`, with five inputs and a
batch size of two so the split, the rebasing, and the reassembly all run,
and every vector checked against what the same input gets alone. The
reviewer also noted that `model` and `revision` took whichever sub-batch
answered last, which would have mixed two cohorts into one result if the
service ever disagreed with itself. That is a refusal now.
`PipelineConfig::embed_batch_size`, which nothing read, is gone.

**This branch's docs claimed spec 021's work.** `holes.md` said H8 was
retired and H2 complete in all seven phases, and `CLAUDE.md` said the same,
but spec 021 is PR #51 and is not in this branch. If this had merged first,
`main` would have asserted a hole was filled with neither its spec nor its
code present. Scoped to what this branch carries, with the dependency
named.

Smaller: `yeomna-embed` moved to `[dev-dependencies]` in `yeomna-store`,
where its only use is; `refresh_info`, `health_check`, and
`EmbedResult::chunk_count` deleted for having no caller; the `task` field on
the `embed` span recorded rather than declared and left empty; EC-2's
accepting half tested where only its refusing half was; and the one
process-environment write in the Python tests explained in place, since
`seal_environment` exists to set variables before transformers is imported
and a test that loads the model without calling it is not testing what
production runs.

**Left as reported, with a reason.** The reviewer flagged that the systemd
unit runs `uv run --frozen` under `ProtectHome=read-only`, and that `uv`
wants to write a venv and a cache. That is a real risk and it is why
`ExecStart` now names the interpreter inside the built virtual environment
instead, so the unit runs a Python that exists rather than a tool that
would build one. Installing the appliance builds the environment, which is
what an appliance does.

## Verification

- Gate 420 green three times with the cluster up and the embedder running,
  clippy zero, fmt clean, the no-SQL lint passing unchanged.
- The service's own tests: 54 passed and 5 skipped without a GPU, 59 passed
  with `YEOMNA_EMBEDDER_GPU_TESTS=1`.
- The oracle test passes against the live model: the service's pooling and
  `late_chunk_embeddings` agree to cosine 0.9999 or better on every chunk,
  over a 10-window document.
- Service-gated tests skip with a named reason when the socket is absent,
  checked by stopping the service and running them.
- `embed.text` answers through `yeomna call` and through the daemon, and
  both leave an audit row with the actor the kernel named.
- Real ingests: one crate with 21 chunks and 21 vectors, and the whole
  WeaverTools corpus at 235 documents, 1,480 chunks, 1,275 vectors, 5
  documents over the ceiling, in 2 minutes 52 seconds with GPU 2 at 15.5
  GiB and full utilization.
- A nearest-neighbour query over the stored vectors returns sensible
  neighbours, and stored self dot products are 1.0 within halfvec's fp16
  precision.
