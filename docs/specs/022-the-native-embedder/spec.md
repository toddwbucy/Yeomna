# Specification: 022 The Native Embedder, and H4

Parent PRD: `docs/PRD-embedder.md` v0.2, **Phases 1 and 2**. The two were
drafted apart and built together, because the client rewrite forces the
ingest path to change: `EmbeddingClient::embed` changed shape, so leaving
ingest for a later spec would have meant writing a per-chunk embedding call
against the new client and deleting it one PR later. Phase 3, hybrid query,
stays separate and is the tenth item of R21's order.
Owner: H4. Ruled by D9 (H4 is Yeomna-native, its own PRD) and by that
PRD's D1 through D11. The ninth PR in R21's order.
Status: built, 2026-09-10. **R25, R26, and R27 proposed** by the parent PRD
and all three implemented as proposed. **R28 raised and left open** by the
build: whether the `code` task needs a query and passage split the way
`retrieval` does.

Editorial rules: ASCII only, no em-dashes, no semicolons, never the
words genuinely, honestly, or actually. These govern prose. Rust,
Python, SQL, and JSON keep their syntax.

---

## Overview

Nothing in this appliance embedded anything before this spec. `embed.text`
was a valid request the dispatch declined, ingest passed `None` where an
embedder would go, and the `embeddings` table had been correct and empty
since spec 008. This spec builds the service that fills it and stops the
refusal.

The parent PRD ruled the fork the pipeline-libraries PRD deferred: **the
service pools and the wire carries chunk vectors**, which is the opposite
of the arm the spike proved. The reason is the transport, which the spike
could not have measured because it wrote `.npy` files to a directory. One
8,631-token document is 8631 by 2048 floats, about 212 MB once JSON has
written each float as text, against 240 KB for the same document's 29
pooled vectors. See PRD D1 and D2.

`late_chunk_embeddings` is not discarded by that ruling. It becomes the
oracle: the service exposes a capped token-level operation whose only
purpose is a test that pools in Rust and requires the service's own
pooling to match. The mathematics the spike validated at cosine 0.999999
stops producing and starts verifying, which is the shape Todd named the
Bastion Method.

Both ingest paths embed here, which is Phase 2 folded in. That is where the
defect the PRD recorded gets fixed: `insert_embedding` read the model out of
the parent node's payload, the document path wrote it and the codebase path
never did, and the live ingest path discarded the sink's `errors` count for
chunks and embeddings while keeping it for edges. So the first codebase
ingest with embedding on would have run the GPU, produced correct vectors,
rejected every one, and reported `embeddings_written: 0` with success. The
cohort rides the embedding document now, so the ordering dependency is gone
rather than documented, and a rejected chunk or embedding row stops the run.

## Task Scope

- **The contract**, `docs/embedding-contract.md`, `yeomna.embedding` v1.
  Three operations over HTTP/1.1 with JSON bodies on a Unix socket:
  `GET /v1/info`, `POST /v1/embed`, `POST /v1/tokens`. Always chunked,
  no single-vector mode.
- **The service**, `services/embedder/`. Python, uv-managed with a
  committed lockfile, `transformers >=4.35,<5.0`, loading
  `jinaai/jina-embeddings-v4` at revision
  `853c867b65b749f3c3c72a06868140d842e04f06` from a local path on
  `cuda:2` in fp16, max 16,384 tokens. Binds one socket at 0600,
  replacing a stale one, refusing a missing directory, which is the
  daemon's `bind()` behavior.
- **The unit**, `deploy/yeomna-embedder.service`. Three seal layers:
  `PrivateNetwork=yes`, `RestrictAddressFamilies=AF_UNIX`, and the
  offline environment variables.
- **The client**, rewritten in `crates/yeomna-embed/src/embedding.rs`:
  `EmbeddingEndpoint::Tcp` deleted, the chunked response parsed, the
  three TCP-default artifacts retired together.
- **The trait**, `Embedder` in `crates/yeomna-pipeline/src/embed.rs`
  beside `Extractor`, with `EmbeddingClient` as the real implementor and
  `HashEmbedder` as a deterministic double.
- **The config key**, `embedder_socket`, in both readers.
- **The verb.** `embed.text` calls instead of refusing, defaulting to
  `retrieval.query`.
- **The oracle test**, service-gated, cosine 0.9999.
- **Both ingest paths embed** (Phase 2). `ingest_documents` gains the
  embedder parameter it never had, both take chunk boundaries from the
  encoding pass when embedding is on, and `IngestRequest` gains `embed` and
  `embed_task`.
- **The cohort on the row** (R26): `embeddings` gains `model_revision` and
  `task`, both `NOT NULL`, `SCHEMA_VERSION` goes to 1.3.0, and the
  provenance moves onto the embedding document.
- **Over the ceiling**, counted and named rather than truncated, with the
  file still chunked for keyword search.

## Out of Scope

- **Hybrid query.** Phase 3, its own spec, and the tenth item of R21's
  order.
- **Multimodal.** The contract refuses images rather than ignoring them,
  which is the whole of the work here.
- **The 128-dimension multivector.** A different feature. Pooling it
  would be wrong (spike README).
- **Fetching weights.** R25.
- **A Rust model loader.** R27. The contract is what a loader would have
  to satisfy.
- **Idle unload, throughput measurement, macro windows, M1.** PRD Phase
  4.

## Files to Modify

- `docs/embedding-contract.md` (new): the wire contract.
- `services/embedder/` (new): `server.py`, `pyproject.toml`, `uv.lock`,
  `README.md`.
- `deploy/yeomna-embedder.service` (new).
- `crates/yeomna-embed/src/embedding.rs`: endpoint, parser, result type.
- `crates/yeomna-embed/src/lib.rs`: the transport note, which keeps its
  gRPC ruling and gains the socket-only one.
- `crates/yeomna-embed/tests/config_and_types.rs`: the pinning test
  inverts.
- `crates/yeomna-pipeline/src/embed.rs` (new): the trait and the double.
- `crates/yeomna-pipeline/src/lib.rs`: export them.
- `crates/yeomna-verbs/src/execute.rs`: the arm at 259 becomes a call,
  and `Session` gains an embedder socket the way it has a store endpoint.
- `crates/yeomna-verbs/src/embed.rs` (new): the verb.
- `crates/yeomna-cli/src/config.rs` and the daemon's reader:
  `embedder_socket`.
- `docs/yeomna.toml.example`: the key, documented.
- `docs/holes.md`, `CLAUDE.md`, `README.md` section 5.1 if the seal
  wording needs it.

## Files to Reference

- `spikes/jina-late-loop/README.md` and `jina_dump.py`: the forward call
  (`process_texts(..., prefix="Passage")`,
  `output_vlm_last_hidden_states=True`), the character-to-byte offset
  conversion, the `prefix_bytes` rebase, the retokenization abort, and
  every measured number.
- `crates/yeomna-chunking/src/late.rs`: the windowing rule
  (`step = size - overlap`, last window clamps) the service must
  reproduce exactly, and `mean_pool_and_normalize`.
- `crates/yeomna-daemon/src/server.rs`: `bind()`, for the socket
  behavior the service copies.
- `crates/yeomna-pipeline/src/extract.rs`: the `Extractor` trait, for the
  shape `Embedder` matches.
- `deploy/yeomna-daemon.service`: the hardening list, and the two lines
  the embedder cannot keep (see EC-9).

## Patterns to Follow

- **Refusals name what they wait for and why.** The existing
  `unimplemented_in` messages are the model, and `InstallError::NoSource`
  from spec 021 is the model for a refusal that names a ruling.
- **Skip messages name the cause.** Cluster-gated tests print which
  socket they looked for. Service-gated tests do the same, separately, so
  a run with Postgres up and the embedder down says which one is missing.
- **No process-wide environment mutation in a test binary.** The
  `install`/`install_in` and `resolve_and_probe`/`resolve_and_probe_in`
  split is the pattern. This session has caught that mistake three times.
- **Both coordinate systems on every chunk**, because the mapping between
  them is where the spike found a bug.

## Functional Requirements

- **FR1** `GET /v1/info` reports `model`, `model_revision`, `dimension`
  (2048), `max_tokens` (16384), `tasks`, `device`, and `loaded`. A client
  can tell from it whether a corpus and a query will be comparable.
- **FR2** `POST /v1/embed` takes `input` (a string or an array of
  strings), `task`, `chunk_size_tokens`, `chunk_overlap_tokens`, and
  returns per input a `token_count` and a `chunks` array. Every chunk
  carries `chunk_index`, `total_chunks`, `vector`, `start_token`,
  `end_token`, `start_byte`, `end_byte`.
- **FR3** The response is always chunked. A one-sentence input returns
  one chunk with `total_chunks` 1. There is no single-vector mode and no
  flag that produces one.
- **FR4** The service's windowing is `late.rs`'s windowing: step is size
  minus overlap with a floor of 1, the last window clamps to the token
  count, and the boundaries tile the document with no gap. Pooling is
  mean over the window followed by L2 normalization, so every returned
  vector is unit.
- **FR5** Byte spans are UTF-8 byte offsets into the caller's own text,
  with the task prefix rebased out. Slicing the caller's text by a
  chunk's span yields text that round-trips.
- **FR6** `POST /v1/tokens` returns `hidden_states`, `offsets` as byte
  pairs, `prefix_bytes`, and `token_count`, and refuses above 512 tokens
  naming the cap and its purpose.
- **FR7** The service refuses `images` in a request rather than ignoring
  them, refuses a `task` it does not serve rather than routing to another
  adapter, and refuses an input above `max_tokens` rather than
  truncating.
- **FR8** The service binds one Unix socket at mode 0600, replaces a
  stale socket file, and refuses to start when the socket's directory is
  missing or the weights path is absent. It never fetches weights (R25).
- **FR9** `EmbeddingEndpoint` has one variant. `parse_endpoint` accepts
  `unix://path` and a bare absolute path, and rejects `http://`,
  `https://`, and a bare host, with an error naming charter 5.1.
- **FR10** `Embedder` is a trait with one required operation, and both
  `EmbeddingClient` and `HashEmbedder` implement it. `HashEmbedder`
  produces a deterministic unit vector per chunk from the text and the
  same windowing, so its output is stable across runs and machines.
- **FR11** `embed.text` returns one vector for one text at the requested
  task, defaulting to `retrieval.query`, because a bare `embed.text` is
  overwhelmingly a query and a passage-embedded query is the silent
  failure PE-API warned about. The response carries the model, the
  revision, the task, and the dimension.
- **FR12** The dispatch has one fewer refusal. The shrinking-refusal test
  names H7 and H9 and no longer names H4.
- **FR13** `embedder_socket` is read by the CLI and the daemon, derives
  `{socket_dir}/embedder.sock` when absent, and a session with no
  reachable embedder makes `embed.text` fail with an error naming the
  socket it tried.

## Edge Cases

- **EC-1** An empty input string: refused as invalid rather than
  returning a zero vector, because a zero vector has no cosine and would
  poison an index quietly.
- **EC-2** An input of exactly `max_tokens`: accepted. One token more:
  refused, with the count in the message.
- **EC-3** An overlap greater than or equal to the chunk size: the step
  floor of 1 in `late.rs` makes this terminate rather than loop, and the
  service must reproduce that rather than diverge. Tested at both sides.
- **EC-4** Multi-byte text. A document of CJK or emoji must produce byte
  spans that slice correctly, because the spike found that HF reports
  character offsets and ASCII hides the difference.
- **EC-5** A document whose text ends mid-window: the last window clamps
  and its `end_token` equals `token_count`.
- **EC-6** The service is up but the model is not loaded yet: `/v1/info`
  reports `loaded: false` and `/v1/embed` refuses with a retryable error
  rather than blocking for the load.
- **EC-7** A stale socket file left by a killed service: replaced on
  bind, as the daemon does.
- **EC-8** GPU out of memory mid-request: reported as a distinct
  retryable error. The client's existing OOM batch-halving stays and now
  halves inputs rather than chunks.
- **EC-9** The unit cannot carry two lines the daemon's carries.
  `MemoryDenyWriteExecute=yes` breaks CUDA, which maps writable
  executable pages. `PrivateDevices=yes` would hide `/dev/nvidia*`. Both
  are omitted with a comment saying why, because an unexplained omission
  in a hardening list reads as an oversight.
- **EC-10** `PrivateNetwork=yes` with a service that must resolve
  nothing: no `DNS`, no `HF_ENDPOINT`, and the offline variables set so
  that a code path reaching for the network fails on the variable before
  it fails on the namespace, which produces a better error.
- **EC-11** Two vectors requested at different tasks in one process: the
  service reloads no adapter mid-batch and instead groups by task, or
  refuses a mixed batch. Refusing is acceptable for v1 and must be
  explicit.

## Implementation Notes

DO:

- DO reproduce `late.rs`'s windowing exactly, and prove it with the
  oracle test rather than by reading both.
- DO convert character offsets to UTF-8 byte offsets in the service,
  before they cross the wire, and rebase the task prefix out.
- DO abort a response if retokenization does not reproduce the forward's
  ids, which is the spike's own guard and the reason its fixtures are
  trustworthy.
- DO put the `Tcp` deletion, the constant, and the pinning test in one
  commit, so the retirement reads as one act.
- DO make `HashEmbedder` deterministic across machines, which means no
  hashing of pointers, no iteration order dependence, and no floating
  point accumulated in a nondeterministic order.
- DO run the gate three times before opening the PR, and run the local
  review before that.

DON'T:

- DON'T ship the spike. It says "nothing here ships" in its first
  paragraph and that stays true. Reference its numbers, copy its
  guards, move none of its code.
- DON'T pool the 128-dimension multivector.
- DON'T add a single-vector mode, a `late: false` flag, or any other way
  to get an unchunked response.
- DON'T let `/v1/tokens` grow past its cap or gain a caller in
  production code.
- DON'T accept a caller-supplied vector anywhere (D7).
- DON'T emit SQL from `yeomna-embed` or `yeomna-pipeline`. The no-SQL
  lint's allowlist does not change.
- DON'T mutate the process environment in a test.

## Success Criteria

1. `embed.text` answers against the live service and H4 is filled.
2. The oracle test passes, so the service's pooling is the pooling the
   spike validated.
3. `EmbeddingEndpoint` cannot express a network endpoint, and the test
   that once pinned the TCP default now pins its absence.
4. Every test but the service-gated ones runs on a machine with no GPU,
   through `HashEmbedder`.
5. A real ingest writes real vectors with their cohort, and a nearest
   neighbour query over them returns something sensible.
6. Gate three times green with the cluster up.

## QA Acceptance Criteria

- `cargo build`, `cargo test` (3x, cluster up), `cargo clippy
  --all-targets`, `cargo fmt --check`, all clean.
- Tests cover FR1 through FR13 and EC-1 through EC-11, with the
  service-gated ones skipping by name when the socket is absent.
- The no-SQL lint passes unchanged.
- The service's own tests run under `uv run pytest` and do not require a
  GPU except where the subject is the model.
- The editorial sweep is clean on every document this spec touches.
