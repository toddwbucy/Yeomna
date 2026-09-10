# PRD: The Native Embedder

Editorial rules: ASCII only, no em-dashes, no semicolons, never the words
genuinely, honestly, or actually. These govern prose. Rust, Python, SQL,
and JSON keep their syntax.

## Revision History

| Version | Date | Change |
|---|---|---|
| 0.2 | 2026-09-10 | Built. Four defects in the spike's approach found and fixed on the way (below), the ceiling re-measured at the ceiling with a condition the first draft did not name, and **R28 raised and left open** on whether the `code` task needs a query and passage split. |
| 0.1 | 2026-09-10 | First draft. Owns H4 under D9. Rules the Phase 3 fork the pipeline-libraries PRD deferred, and rules it against the arm the spike proved, for a reason the spike could not have seen. Retires the client's TCP default by deleting the variant rather than moving the default. Proposes R25 (weights are not fetched), R26 (the task belongs in the embeddings row), and R27 (the backend is Python behind the contract). Records the silent-drop defect in the codebase ingest path that would have made the first embedding run produce zero rows and report success. |

## Executive Summary

Charter section 5.1 says embedding is local, and says it is a requirement
rather than a configuration default. Nothing in this appliance embeds
anything today. `embed.text` refuses by name, `query --hybrid` refuses by
name, ingest passes `None` where an embedder would go, and the
`embeddings` table has been correct and empty since spec 008. H4 is the
hole under all of that, and D9 ruled on 2026-09-09 that it is filled
natively: an in-box embedder service that belongs to Yeomna, not a
component borrowed from WeaverTools.

This PRD is that service. Four things in it are worth reading before the
phases.

**The wire carries pooled chunk vectors, and the service does the
pooling.** The pipeline-libraries PRD left this as a fork to be ruled
"with the code in hand," and the code is now in hand. The spike
(`spikes/jina-late-loop`, 2026-08-10) is the existence proof for the
other arm, where the service returns token-level hidden states and the
client pools them through `late_chunk_embeddings`. That arm works. It
works over `.npy` files on a local disk, which is what the spike
measured. It does not survive contact with the transport: one 8,631-token
document is 8631 by 2048 floats, which is 70 MB raw and about 212 MB once
JSON has written each float as text, and the embedder transport is HTTP
with a JSON body by a standing ruling in `yeomna-embed`. The pooled
response for the same document is 29 vectors, about 240 KB. The spike
measured the mathematics and was right about it. It could not measure a
transport that did not exist yet.

**The pooling identity becomes a test rather than a code path, and that
is a promotion.** `late_chunk_embeddings` is not deleted and does not
become dead code. The service exposes a second, size-capped operation
that returns token-level states, and a gated test asks the service for
both views of the same short text, pools the token view in Rust, and
requires the two to agree. The Rust mathematics stops being the producer
and starts being the auditor. Todd's framing for this is the Bastion
Method, and this is its shape exactly: the motive is retrieval quality
and it will keep changing, while the mechanic is that pooled output must
equal the pooling identity, which does not change and is checked by
machine every run.

**The seal extends to the service, structurally.** The spike recorded
that Jina's own modeling file imports `requests`, a network library, into
the embedding path. That is not a thing to document and hope about. The
unit runs with `PrivateNetwork=yes` and `RestrictAddressFamilies=AF_UNIX`,
so the import is inert because there is no network in the namespace, and
the offline environment variables the spike used become the second and
third layer rather than the only one. This mirrors the three-layer seal
on the Postgres cluster, where the systemd layer is the one that matters
because an edit to a config file cannot reopen it.

**The TCP endpoint is removed, not defaulted away.** H4 asked for
"retirement of the client's TCP default." Moving the default to Unix
leaves a field an operator can point anywhere, and charter 5.1 says local
embedding is a requirement. A requirement that a config key can turn off
is a preference. `EmbeddingEndpoint` loses its `Tcp` variant, so a socket
path is the only thing the type can hold, and corpus text cannot reach an
embedding API because there is no expressible way to name one.

One defect is recorded here rather than found later. `insert_embedding`
reads the model name out of the parent node's payload and returns
`Ok(false)` when it is absent, which is counted as "not created" rather
than as an error. The document path writes `embedding_model` into node
payloads. The codebase path does not. So flipping `CodebaseConfig::embed`
to true today would run the GPU, produce correct vectors, drop every one
of them, and report success with `embeddings_written: 0`. This is the
same shape as the drift and retire near-miss from spec 019: a defect that
exists only because the consumer that would have exercised it had not
arrived. Phase 2 fixes it and tests it.

## Background and Context

### What exists, and what it expects

| Piece | State | Where |
|---|---|---|
| `embeddings` table | correct and empty. `halfvec(2048)`, HNSW `halfvec_cosine_ops`, `model` and `model_hash` both `NOT NULL` | `crates/yeomna-store/schema.sql:67` |
| `EmbeddingClient` | speaks HTTP JSON in the OpenAI single-vector shape, parses `data[].embedding` and nothing else, defaults to `http://localhost:8087/v1` | `crates/yeomna-embed/src/embedding.rs` |
| `late_chunk_embeddings` | complete and unit-tested, takes one vector per token, returns pooled chunk vectors and token-space boundaries. Its only caller in history is the spike harness | `crates/yeomna-chunking/src/late.rs:51` |
| `embed.text` | a valid request the dispatch declines: "embed.text arrives with the embedder, H4" | `crates/yeomna-verbs/src/execute.rs:259` |
| `query --hybrid` | refused before any validation: "hybrid ranking needs the embedder, H4" | `crates/yeomna-verbs/src/read.rs:449` |
| codebase ingest | takes `Option<&EmbeddingClient>`, and the verb layer always passes `None` with `embed: false` | `crates/yeomna-pipeline/src/codebase.rs:157`, `crates/yeomna-verbs/src/ingest.rs:99` |
| document ingest | has no embedder parameter of any kind | `crates/yeomna-pipeline/src/documents.rs:161` |
| the spike | evidence, not shippable code, and it says so in its first paragraph | `spikes/jina-late-loop/` |

Two shapes meet nowhere. `EmbeddingClient::embed` returns one pooled
vector per input text. `late_chunk_embeddings` wants one vector per token
from a single whole-document pass. The types are both
`Vec<Vec<f32>>`-shaped and mean different things, which is the sort of
collision that compiles.

### What the spike settled, and what it could not

Run 2026-08-10 on GPU 2 against `jinaai/jina-embeddings-v4` at revision
`853c867b65b749f3c3c72a06868140d842e04f06`, fully offline, over this
repository's own charter as the document.

| Finding | Value | What it settles |
|---|---|---|
| Whole-document pool through `late.rs` against Jina's own `single_vec_emb` | cosine 0.999999 | The lifted pooling is Jina's pooling, measured on real model output rather than argued from the source |
| Chunks at 500 tokens with 200 overlap over 8,631 tokens | 29, boundaries tile 0 to 8631 with no gap | The windowing is correct at the seams, which is where windowing is wrong |
| Chunk vector norms | all unit | Ready for a cosine index without a normalization step at the edge |
| Chunk to document cosine | 0.8870 to 0.9691 | Chunk vectors land in the document's space, which is what makes them comparable to a query vector |
| Token window to byte span | clean, chunk 1 decoded back to charter text | The token-to-byte mapping is exact, including the 9-byte `"Passage: "` prefix rebase |
| Peak VRAM at 8,631 tokens | 11.48 GiB of 16 | The context ceiling below, and it is the finding with the most consequence |

Two details from the spike are worth carrying forward as obligations
rather than trivia. HF fast tokenizers report **character** offsets while
every consumer here slices **bytes**, and on pure-ASCII input the two
coincide, which is how the mismatch stayed invisible until review. And
the model's 128-dimension multivector output is a late-interaction
projection, not a smaller embedding, so pooling it would be wrong.

What the spike could not settle is anything about the transport, because
it wrote `.npy` files to a directory. It is cited below as evidence for
the mathematics and never as evidence for the wire.

### Four defects in the spike's approach, found while building on it

The spike was evidence and said so, and building the real thing is what
tested it. Each of these would have been a defect in the service if its
code had been moved rather than read.

1. **Pooling with no attention mask.** `jina_dump.py` takes a plain mean
   over the sequence, which is correct only for one unpadded input.
   `process_texts` pads a batch to its longest member and the model's own
   pooling is a mean over the mask, so a batch would have pooled pad
   positions into every vector but the longest. The service runs one
   forward pass per input, which also keeps the peak at the cost of the
   largest single document, and the largest single document is what the
   ceiling is about.
2. **The revision pin did not cover the LoRA adapters.** The model's
   `from_pretrained` calls `snapshot_download` for `adapters/*` **with no
   revision** when handed a repository id, so the adapters resolved through
   `refs/main` while the weights resolved through the pin. The spike passed
   a repository id. On this machine `refs/main` happens to be the pinned
   SHA, so **the spike's numbers stand by coincidence rather than by its
   pin.** The service resolves the snapshot directory itself and hands
   `from_pretrained` a path, which reads the adapters from inside the
   pinned snapshot and removes the download call, which also satisfies R25
   without a library call that could reach the network.
3. **Prefix tokens were pooled into the vector.** Harmless for the spike's
   document-level identity check against the model's own single vector,
   which includes them too. Wrong for a chunk whose byte span has to start
   at byte 0 of the caller's text. The service drops the prefix tokens from
   the hidden states, the offsets, and `token_count` together, which is
   also what makes `/v1/tokens` a true oracle for `/v1/embed` rather than
   an approximate one.
4. **The prefix length was hard-coded at nine bytes.** `"Passage: "` is
   nine and `"Query: "` is seven, and three of the four contract tasks use
   the shorter one. It is computed per task now, and the prefix token count
   is derived from the offsets rather than assumed, so a token straddling
   the prefix boundary clamps to byte 0 instead of going negative.

### The context ceiling, which is a hardware fact and not a preference

11.48 GiB at 8,631 tokens, on a card with 16 GiB. Weights are 7.5 to 8
GiB of that, so about 8 GiB is left for context and 8.6k tokens consumed
3.5 to 4 GiB of it. A 32k-token pass wants roughly four times that. **The
32k context the model advertises does not fit on GPU 2.** The spike said
"testing runs at 16k, which is plenty" and that is the number this PRD
takes.

The consequence has to be chosen rather than discovered. A document above
the ceiling can be truncated, refused, or split into overlapping macro
windows. Truncation is out: it drops the tail of a document out of vector
search while leaving it in full-text search, and nothing in the response
would say so. See D5.

**Re-measured during the build, and the ceiling has a condition the first
draft did not name.** 16,384 tokens holds only with
`PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True`. Under the default
caching allocator the real ceiling measured about 14k: a fresh process
embedded 13,993 tokens and then refused 14,500 with `out-of-memory`,
because fixed segments fragment under the growing activations a single
long pass asks for. With expandable segments the whole ladder to 16,384
passes. A service advertising a `max_tokens` it cannot reach has
unpredictable refusals, so the service sets the variable itself before
importing torch rather than trusting its unit to do it. Measured peaks, on
GPU 2 with 370 MiB already in use by another process:

| content tokens | torch peak | nvidia-smi |
|---|---|---|
| idle, loaded | 7.82 GiB | 8,330 MiB |
| 7,998 | 11.07 GiB | 11,926 MiB |
| 13,993 | 13.61 GiB | 14,566 MiB |
| 16,380 | 14.62 GiB | 15,666 MiB |

The refusal boundary was confirmed to the token: 16,384 accepted, 16,385
refused as `input-too-large` with the count in the message.

## User Stories

- **As a Claude Code session building these projects**, I ask
  `query --hybrid` for "where does the session hold its client" and get
  chunks that match on meaning as well as on words, because the phrase I
  remember is not the phrase in the file.
- **As the operator standing up a graph**, I run `codebase.ingest` with
  embedding on, and if the embedder is not reachable the run fails before
  it writes a chunk, rather than leaving text in the store with no vectors
  beside it.
- **As the operator of a sealed appliance**, I can state that document
  text never reached an embedding API, and point at the reason it could
  not have: the client type cannot express a network endpoint and the unit
  has no network namespace.
- **As the person who has to answer a regulator**, I can say which model
  and which task produced every vector in the store, from the row itself.
- **As a reviewer**, I can check that the service's pooling is the pooling
  the spike validated, by running one test, without reading Python.
- **As a test author on a machine with no GPU**, I can exercise the whole
  ingest-with-embeddings path against a deterministic double and get the
  same code path the real embedder takes.

## Goals

1. **H4 closes.** `embed.text` answers. The refusal at `execute.rs:259`
   is deleted, and the shrinking-refusal test names one fewer hole.
2. **Late chunking is wired**, for the first time in the history of this
   code or the reference's. A document is encoded in one pass and its
   chunk vectors are conditioned on the whole document.
3. **The seal holds structurally**, at three layers, with the type system
   as one of them.
4. **The model and the task are recorded per vector**, so two vectors can
   be checked for comparability before they are compared.
5. **Nothing in the embedding path requires a GPU to test**, except the
   tests whose subject is the GPU.
6. **The pooling identity is checked by machine**, not asserted in prose.

### Non-Goals

- **No Rust model loader.** See D8. The backend is Python behind the
  contract, and the contract is what a future Rust loader would have to
  satisfy.
- **No multimodal embedding.** The model supports images. Nothing in this
  appliance extracts one yet, and a capability with no caller is
  speculation. The contract refuses images rather than ignoring them.
- **No multivector or late-interaction retrieval.** The 128-dimension
  output is a different feature with a different index and its own PRD if
  it ever earns one.
- **No graph embeddings.** That is H9 and it has its own future PRD.
  `structural` keeps refusing and keeps naming H9.
- **No embedding of a query the caller supplies as a vector.** See D7.
- **No fetching weights.** See D6 and R25.
- **No second store, no external vector service, no remote model.** The
  charter settled these and this PRD does not reopen them.
- **No migration.** A model change or a schema change costs a re-ingest,
  per T4. There is no re-embed-in-place tooling and none is planned.
- **No M1.** Whether relevance is good enough on a firm's document set is
  a measurement against a corpus this repository does not have. Hybrid
  retrieval informs M1 and cannot settle it.

## Feature Specifications

### Phase 1: The contract, the service, and `embed.text`

The deliverable is a service that answers and a verb that stops refusing.
Nothing ingests yet.

**The contract**, at `docs/embedding-contract.md`, named
`yeomna.embedding` v1 to match the `yeomna.extraction` package the
extraction lift renamed. It is not PE-API v1 and does not claim to be:
the response shape is a fork, which the pipeline-libraries PRD predicted
would be "incompatible wire contracts either way." What carries over from
PE-API is its central judgement, that a response is always chunked and
there is no single-vector mode, because that is the shape late chunking
has and papering it over with a flag hides the thing the product depends
on.

Three operations over HTTP/1.1 with JSON bodies on a Unix socket:

| Operation | Purpose |
|---|---|
| `GET /v1/info` | The model, its revision, its dimension, its context ceiling, the tasks it serves, and whether the weights are loaded |
| `POST /v1/embed` | Text in, pooled chunk vectors out, always chunked |
| `POST /v1/tokens` | Token-level hidden states plus byte offsets, size-capped, for verification only |

`POST /v1/embed` takes the chunking policy as parameters, so policy stays
where the rest of the chunking policy lives:

```jsonc
{
  "input": ["long document text", "another"],
  "task": "retrieval.passage",
  "chunk_size_tokens": 500,
  "chunk_overlap_tokens": 200
}
```

and returns, per input, the chunks with both coordinate systems on every
chunk:

```jsonc
{
  "model": "jinaai/jina-embeddings-v4",
  "model_revision": "853c867b...",
  "task": "retrieval.passage",
  "dimension": 2048,
  "results": [
    {
      "index": 0,
      "token_count": 8631,
      "chunks": [
        {
          "chunk_index": 0,
          "total_chunks": 29,
          "vector": [0.0123, -0.0456, "..."],
          "start_token": 0, "end_token": 500,
          "start_byte": 0,  "end_byte": 1974
        }
      ]
    }
  ]
}
```

Both coordinate systems ride every chunk because the token range is what
the pooling used and the byte span is what the store holds, and the
mapping between them is the thing the spike found a bug in. Shipping both
means the mapping is checkable against the text rather than trusted.

`POST /v1/tokens` returns `hidden_states` as an array of arrays plus
`offsets` as byte pairs and `prefix_bytes`, and refuses above a token cap
(D4) so it cannot be used as a back door to the shape D1 rejected.

**The service**, at `services/embedder/`. Python, uv-managed with a
committed lockfile, `transformers >=4.35,<5.0` because the spike found
that 5.x breaks the model's cached custom code through `ROPE_INIT_FUNCTIONS`
drift. It loads the pinned revision from a configured local path, on
`cuda:2`, and refuses to start if the weights are absent. It binds one
Unix socket at mode 0600 and replaces a stale one, the way the daemon's
`bind()` does.

**The unit**, at `deploy/yeomna-embedder.service`, mirroring
`yeomna-postgres.service` including the lesson that a restart loop cannot
conjure hardware. Three seal layers: `PrivateNetwork=yes`,
`RestrictAddressFamilies=AF_UNIX`, and `HF_HUB_OFFLINE=1` with
`TRANSFORMERS_OFFLINE=1`. `RequiresMountsFor` on the weights path, so a
boot that has not mounted them refuses to start rather than trying to
download.

**The client**, rewritten in place in `crates/yeomna-embed`:

- `EmbeddingEndpoint` loses `Tcp`. `parse_endpoint` accepts `unix://` and
  a bare absolute path, and rejects everything else including
  `http://localhost`, naming charter 5.1.
- The response parser learns the chunked shape. `EmbedResult` gains
  chunks with both coordinate systems and loses the derived-dimension
  guess.
- `Embedder`, a trait in `yeomna-pipeline` beside `Extractor`, with
  `EmbeddingClient` as the real implementor and `HashEmbedder` as a
  deterministic double. The double is the docling-rag harvest pointer the
  ledger recorded for exactly this purpose, and it is what lets an
  embedding test run on a machine with no card.
- `embedder_socket` in the config file, an `Option<String>` deriving
  `{socket_dir}/embedder.sock` when absent, which is the shape
  `daemon_socket()` already has and puts the socket in a directory the
  appliance keeps at 0700. The config stays flat rather than growing a
  table, because the file has one shape and a second one would be a
  second surface. The holes ledger notes this key was listed before it
  existed, and this is when it starts existing.

**The verb.** `execute.rs:259` becomes a call. `embed.text` takes one
text and one task and returns one vector, which is the single-chunk case
of the same operation and is what a query needs. The contract does not
change, so R4 is not reopened.

**The equivalence test**, gated on the service being up. Ask `/v1/tokens`
and `/v1/embed` for the same short text, pool the token view through
`late_chunk_embeddings`, and require cosine 0.9999 or better against
every chunk the service pooled. This is the spike's finding turned into a
standing check, and it is the only reason `/v1/tokens` exists.

### Phase 2: Ingest wiring, and what a document too large gets

**The defect first.** `write_file_nodes` in the codebase path learns to
stamp `embedding_model` into the file node payload, which is what
`insert_embedding` reads and what the document path already writes. The
test plants a file, embeds with the double, and asserts a row lands. A
second test asserts that a payload without the key produces an error
rather than a silent `Ok(false)`, because a write that drops data and
reports success is worse than one that fails.

**Both paths embed.** `ingest_documents` gains the embedder parameter it
never had. Both call the service once per document with the whole text
and store what comes back, which is late chunking wired. The
chunk-count-against-vector-count check stays, because a mismatch means
the two sides disagree about the chunking policy and that is not
recoverable by guessing.

Chunk text and chunk spans now come from the service, not from the local
`ChunkingStrategy`, when embedding is on. This is the substantive change
in the ingest path: late chunking does not chunk text and then embed the
pieces, it encodes the document and then decides where the pieces were.
Byte spans from the service are what `chunks.start_char` and
`chunks.end_char` receive, and the CRLF-exact byte contract is preserved
because the service converts character offsets to byte offsets before
they cross the wire.

**The task column.** `embeddings` gains `task text NOT NULL`, and
`SCHEMA_VERSION` goes to 1.3.0. Two vectors from the same model under
different LoRA adapters have the same `model` and `model_hash` and
incomparable geometry, and `retrieval.query` against a `text-matching`
corpus is quiet garbage rather than an error. With the column, the check
is available to any reader. See R26. Per T4 this costs a drop, a
re-apply, a template re-stamp, and a re-ingest, which is what schema
changes cost here.

**Over the ceiling.** A document whose token count exceeds the service's
`max_tokens` is refused by the service with a named error, and ingest
records it the way `drift` records an unassessable file: counted,
enumerated in the summary, and never confused with a document that had no
chunks. `read::health` already reports `chunks_without_embeddings`, so
the coverage gap is visible from the verb layer without new instrument.
The alternative, silent truncation, is rejected by D5.

### Phase 3: Hybrid query

**Built, spec 023.** The tenth item of R21's order, and this is the PRD
that supplies its vector.

RRF over two sources in one statement, which is the store PRD's binding
constraint: "The reference's search verb is four round trips plus a Rust
loop. Postgres fuses what ArangoDB split." One CTE ranks by
`ts_rank_cd`, one ranks by `vec <=> query`, and the fusion is
`sum(1.0 / (k + rank))` with k as a named constant rather than a request
field, because a caller who can tune the fusion constant is a caller who
can make retrieval quality unreproducible.

The query vector is computed inside the verb through the session's
embedder handle, at the task that **pairs** with the corpus's, which the
verb reads off the rows rather than assuming. It is not accepted from the
caller. See D7.

Three things the build settled that this paragraph did not anticipate.
The cohort check has three outcomes rather than one, and they are
different problems: no embeddings is a `not-found` naming what would fix
it, more than one cohort is an `invalid-args` naming them, and one cohort
is the answer. Hybrid needs a graph, because a cohort is a property of one
and an unscoped database could hold as many cohorts as graphs. And each
source contributes ten times the requested limit before fusion, because
fusion reorders: observed on the WeaverTools corpus, a chunk at keyword
rank 22 and vector rank 10 landed sixth in a fused top six, and cutting
each source at the limit would have discarded it before the fusion could
find it.

### Phase 4: Deferred, and named so it is not discovered

- **Idle unload.** The extraction service pinned `cuda:2` with a
  15-minute idle VLM unload and the embedder wants the same card. Two
  models on one 16 GiB card is a scheduling problem, and it is not one
  until a PDF needs extracting.
- **Throughput.** Nothing here measures documents per minute. It is a
  measurement, not a threshold, per D8's precedent.
- **Macro-window late chunking** for documents above the ceiling, if
  refusing them turns out to matter on a real corpus.
- **M1.** Needs a firm's document set.

## Technical Architecture

```
  yeomna-verbs                      yeomna-pipeline
  +--------------------+            +------------------------+
  | embed.text         |            | trait Embedder         |
  | query --hybrid     |----------->|   EmbeddingClient      |
  +--------------------+            |   HashEmbedder (test)  |
                                    +-----------+------------+
                                                |
                                    HTTP/1.1 JSON over AF_UNIX
                                    /run/yeomna/embedder.sock 0600
                                                |
  +---------------------------------------------v------------------------+
  | services/embedder            PrivateNetwork=yes                      |
  |   /v1/info    model, revision, dimension, ceiling, tasks             |
  |   /v1/embed   one forward pass, window, pool, normalize              |
  |   /v1/tokens  hidden states plus byte offsets, capped, for the test  |
  |                                                                      |
  |   jina-embeddings-v4 @ 853c867b, cuda:2, fp16, max 16384 tokens      |
  +----------------------------------------------------------------------+

  crates/yeomna-chunking/src/late.rs
    late_chunk_embeddings: no longer the producer, now the oracle
```

### Resolved Design Decisions

**D1. The service pools. The wire carries chunk vectors.** This rules the
fork the pipeline-libraries PRD deferred, and it rules against the arm the
spike proved, so the reasoning has to carry its own weight.

The spike's arm ships token-level states to the client. For one 8,631-token
document that is 8631 by 2048 f32, which is 70 MB raw, about 35 MB at
fp16, and roughly 212 MB once JSON has written each float as decimal text.
The embedder transport is HTTP with a JSON body, ruled in
`crates/yeomna-embed/src/lib.rs` and quoted in D2. Ingesting the
WeaverTools corpus of 100 documents would move something on the order of
20 GB of JSON text across a socket and through `serde_json`, to compute
1,518 vectors totalling about 12 MB. The pooled response for the same
document is 29 vectors, about 240 KB.

The spike measured the mathematics and its measurement stands. It wrote
`.npy` files to a local directory and could not have measured a transport
that had not been chosen. Choosing its arm now would be citing evidence
for a claim it does not make.

Three things make the ruling cheap rather than a sacrifice. Chunking
policy stays client-side, because `chunk_size_tokens` and
`chunk_overlap_tokens` are request parameters, so the service executes a
policy and does not own one. The pooling is Jina's own pooling, which the
spike proved to cosine 0.999999, so running it in the process that holds
the model is running it where it came from. And `late_chunk_embeddings`
stays live as the auditor under D3, so the Rust mathematics is not
discarded, it is repointed at verification.

**D2. HTTP/1.1 with a JSON body, over a Unix socket, and no gRPC.**
`crates/yeomna-embed/src/lib.rs:6-9` carries a standing ruling: "Do not
reintroduce the reference's pre-PR-#70 gRPC Unix-socket pattern for the
embedder, that path was removed deliberately." That ruling is kept. The
client's hyper and hyperlocal stack already speaks HTTP over a Unix
socket end to end, `yeomna-proto` deliberately dropped the dead
`persephone.embedding` package, and a Python HTTP server is a smaller
dependency than a Python gRPC server. The extraction service stays gRPC
because it is already gRPC. Two transports in the box is a cost, and it
is smaller than reversing a ruling to buy symmetry.

The consequence is D1, and it is worth stating that the transport ruling
decided the fork rather than the other way round.

**D3. `late_chunk_embeddings` becomes the oracle, and `/v1/tokens` exists
only to feed it.** A pooling identity asserted in a document decays. One
checked by a test that runs against the live service does not. The
operation is capped at a small token count under D4 so it cannot become
the production path by accident, and its response shape is the spike's
fixture shape, so the spike's harness remains readable as the thing this
test descends from.

**D4. `/v1/tokens` caps at 512 tokens.** Enough to pool several chunks at
the default 500-token window and prove the seam, small enough that the
response is under a megabyte, and far too small to ingest with.

**D5. A document over the ceiling is refused, never truncated.** The
ceiling is 16,384 tokens because 8,631 tokens took 11.48 GiB of a 16 GiB
card and the model's advertised 32k does not fit on GPU 2. Truncation
would leave the tail of a document in full-text search and out of vector
search with nothing in any response saying so. Refusal is loud, counted,
and enumerated, and `read::health` already reports
`chunks_without_embeddings` so the gap is visible through a verb. This
follows the drift ruling from spec 019 for the same reason: a thing that
was present but could not be assessed must never be reported as a thing
that was absent.

**D6. Weights are present or the service does not start.** No download,
no fetch on first use, no cache warm. The path is configured, the
revision is pinned by full SHA, and `RequiresMountsFor` means an unmounted
volume is a refusal rather than a 7 GB download into a sealed appliance.
This is charter 5.1's third boundary, Updates, and it is the same question
R24 raised about `tools install`, so it is proposed as **R25** and worded
the same way: a byte arriving from the network into a sealed appliance
decides what the appliance may talk to and how a release is verified.

**D7. The query vector is computed inside the verb, never accepted from
the caller.** A `vector` field on `QueryRequest` would let a caller reach
vector search without calling `embed.text`, which is a side door around a
refusal and would falsify T3 the moment `embed.text` refused anything
again. Computing it in the verb also means the verb can check the corpus
task against the query task, which a caller-supplied vector makes
impossible because the caller's task is unknowable.

**D8. The backend is Python, behind the contract, and the contract is the
permanent artifact.** Jina v4 ships custom modeling code, a Qwen2.5-VL
backbone, and LoRA adapters selected per task. Reimplementing that in
Candle is new construction whose only deliverable is the same vectors,
and the charter's inherit-do-not-author rule points the other way. The
service is replaceable because the contract is narrow: three operations,
one of which exists only for a test. Proposed as **R27**, with the note
that Python in the box is a supply chain, seven packages and a lockfile,
and that `PrivateNetwork=yes` is what makes that supply chain unable to
act at run time.

**D9. The task rides the embeddings row.** Proposed as **R26**. `model`
and `model_hash` identify the weights and not the adapter, so a corpus
embedded at `text-matching` and queried at `retrieval.query` produces
plausible nonsense with nothing to check. A column makes the cohort
question answerable by any reader of the row. It is a schema change, so it
costs 1.3.0, a drop, a re-apply, a template re-stamp, and a re-ingest,
which under T4 is what schema changes cost and is why they are affordable.

Two readers were strengthened by the columns rather than merely permitted
by them. `orient` reports the cohort triple with a count per cohort instead
of a bare list of model names. And `codebase.validate` checks for **one
cohort** per graph where it used to check for one model, because a graph
holding two revisions or two adapters of the same model would have reported
`ok` while holding vectors that cannot be ranked against a single query
vector. That check is the reason to have the columns at all, and leaving it
weaker would have made them decoration.

**D10. `EmbeddingEndpoint::Tcp` is deleted.** Charter 5.1: "Embedding is
local. The embedder is GPU-resident on the same machine, so document text
is never transmitted to an embedding API. This is a requirement, not a
configuration default." A requirement enforced by a default is a
preference. With the variant gone, the type cannot hold a network
endpoint, `parse_endpoint` rejects `http://` by naming the charter, and
the test that pinned the TCP default inverts to pin its absence. Three
things retire together: the constant, the `Default` impl, and that test.

**D11. Ingest fails before it writes when embedding was asked for and the
embedder is unreachable.** This is spec 011's EC-4 and it is unchanged.
The ordering in `codebase.rs` embeds before writing chunks for this
reason, and Phase 2 preserves it in the document path too.

### R28, open: does the `code` task need a query and passage split

Raised by the build and left for a ruling rather than decided quietly.

The model's snapshot fixes two prompt prefixes, `Query` and `Passage`, and
forces `Query` for `text-matching` whatever it is asked for. For `code` it
fixes nothing: the model's own helper defaults to the query prompt, so the
service uses `Query` for `code` and says so.

That may be wrong for a corpus. `retrieval.passage` and `retrieval.query`
exist as a pair because a document and a search for it are asymmetric, and
ingested code is a document. If code should be embedded as a passage and
searched for as a query, the contract needs `code.passage` and
`code.query`, the corpus half becomes the default for ingest, and every
code corpus embedded under the current single `code` task is a re-ingest
away from the fix. If code is symmetric enough that one geometry serves
both, the current shape is right and this is closed.

It is cheap to hold open because R26 put the task on the row: whichever
way it is ruled, a reader can tell which corpora were embedded under the
old answer.

## Testing Strategy

The gate is `cargo build`, `cargo test`, `cargo clippy --all-targets`,
`cargo fmt --check`, three times green before any PR opens, per the
standing rule.

**Three gate levels, each with its own skip message.** Cluster-gated
tests skip without Postgres, the way every existing test does.
Service-gated tests skip without the embedder socket, and print which
socket they looked for. Everything else runs everywhere, which is most of
it, because `HashEmbedder` exists.

**The double is the point.** `HashEmbedder` derives a deterministic
2048-dimension unit vector from the text, chunks by the same windowing
rule, and reports a synthetic model name. Every ingest-with-embeddings
test runs against it, which means the wiring, the payload stamp, the
count check, the byte spans, and the store write are all covered on a
machine with no GPU. Only tests whose subject is the model itself need
the service.

**The oracle test is the one that cannot be faked.** Service-gated, both
operations, one short text, `late_chunk_embeddings` in the middle, cosine
0.9999. If the service ever pools differently than the identity the spike
validated, that test fails and names the chunk it failed on.

**Negative tests, because the refusals are the design.** `http://` as an
endpoint is rejected and the error names the charter. Images in a request
are rejected rather than ignored. A document over the ceiling is refused
and enumerated. A node payload with no `embedding_model` errors rather
than silently writing nothing. `/v1/tokens` above its cap is refused. A
corpus task that does not pair with the query task is refused by hybrid
query rather than fused.

**Golden values on the seam that has already had a bug.** The
character-to-byte offset conversion gets a test with multi-byte input,
because the spike found that ASCII hides the mistake. A chunk's byte span
must slice the original text back to the chunk's own text, which is the
same check the spike's harness made against the charter and the reason it
caught the prefix rebase.

## Risk Assessment

| Risk | Severity | Handling |
|---|---|---|
| The 16 GiB card cannot hold weights plus a long document | High, measured | D5's ceiling, refuse and enumerate. Macro windows are Phase 4 if a corpus demands them |
| `transformers` 5.x breaks the model's cached custom code | High, observed in the spike | Pinned `>=4.35,<5.0` in a committed lockfile. This is R15's pinning discipline applied to Python |
| The modeling file imports `requests` into the embedding path | High if unhandled | `PrivateNetwork=yes` makes it inert. Not a documentation problem |
| Python in a sealed appliance is a supply chain | Medium, accepted | R27 records it. The lockfile is committed, the namespace has no network, and the contract is narrow enough that a Rust backend can replace it |
| Two models want one card | Medium, deferred | Phase 4. Not a problem until extraction needs the GPU, and the PDF path is not built |
| The chunking policy diverges between client and service | Medium | The count check fails the run. Both coordinate systems ride every chunk so a divergence is visible rather than silent |
| Vectors from different tasks get compared | Medium | D9's column, and hybrid query checks before it fuses |
| The service is a new failure mode for ingest | Low | D11: fail before writing. A half-ingest is the thing being prevented |
| Late chunking changes what a chunk is, so old chunks are not new chunks | Low, by design | T4. The graph is a rebuildable index and re-ingest is the answer |

## Timeline

Phased, not dated, in the order the phases are numbered. Phase 1 is one
spec and one PR. Phase 2 is one spec and one PR, and it carries the
schema bump. Phase 3 is the tenth item of R21's order and has its own
spec. Phase 4 is named so it is not discovered later.

The three open rulings, R25, R26, and R27, are proposed by this document
and each is implemented as proposed, because each is reversible at the
cost of a re-ingest and none of them is reversible more cheaply by
waiting.
