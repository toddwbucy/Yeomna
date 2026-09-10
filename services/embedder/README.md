# The Yeomna embedding service

`yeomna.embedding` v1, over HTTP/1.1 with JSON bodies on a Unix socket.
Three operations: `GET /v1/info`, `POST /v1/embed`, `POST /v1/tokens`.

`docs/embedding-contract.md` is normative for the wire. Where it and this
service disagree, that document wins and this service is the defect. This
file says how to run it, what was decided, and the two places where the
contract left something for the implementation to answer.

Built to `docs/specs/022-the-native-embedder/spec.md`, Phase 1 of
`docs/PRD-embedder.md`.

## Running it

By hand, for development:

```bash
cd services/embedder
uv sync --frozen
uv run python server.py
```

Under systemd, `deploy/yeomna-embedder.service` runs
`.venv/bin/python server.py` rather than `uv run`, and the difference is
not cosmetic. The unit carries `ProtectSystem=strict`,
`ProtectHome=read-only`, and `PrivateNetwork=yes`, so `uv` has nowhere to
write a virtual environment or a cache and no network to resolve through.
Installing the appliance runs `uv sync --frozen`, and the unit runs the
interpreter that resulted. A service that builds its own dependencies at
start is a service that can fail to start for a reason that has nothing to
do with the appliance.

Every knob is an environment variable and every default is the
contract's.

| Variable | Default | What it is |
|---|---|---|
| `YEOMNA_EMBEDDER_MODEL` | `jinaai/jina-embeddings-v4` | A hub repo id, or an absolute path to a snapshot directory |
| `YEOMNA_EMBEDDER_REVISION` | `853c867b65b749f3c3c72a06868140d842e04f06` | The full snapshot SHA. Reported by `/v1/info` |
| `YEOMNA_EMBEDDER_DEVICE` | `cuda:2` | GPU 2 is the Yeomna card |
| `YEOMNA_EMBEDDER_SOCKET` | `$HOME/.local/share/yeomna/run/embedder.sock` | Beside the store's socket, in a directory the appliance keeps at 0700 |
| `YEOMNA_EMBEDDER_MAX_TOKENS` | `16384` | The ceiling. See "The ceiling is measured" |
| `YEOMNA_EMBEDDER_WEIGHTS` | `$HOME/.cache/huggingface` | Becomes `HF_HOME`. The snapshot is looked for under `hub/models--<org>--<name>/snapshots/<revision>` |
| `YEOMNA_EMBEDDER_LOG` | `INFO` | Python log level |

A quick look, once it is up:

```bash
S=$HOME/.local/share/yeomna/run/embedder.sock
curl -s --unix-socket $S http://localhost/v1/info
curl -s --unix-socket $S -X POST -H 'Content-Type: application/json' \
  -d '{"input":"a sealed appliance","task":"retrieval.passage"}' \
  http://localhost/v1/embed
```

Tests:

```bash
uv run pytest                              # no GPU, no weights needed
YEOMNA_EMBEDDER_GPU_TESTS=1 uv run pytest  # loads the weights onto the card
```

54 of the 59 tests need neither a card nor the weights. They cover the
windowing rule, the pooling, the character-to-byte conversion, the prefix
rebase, request validation, the whole error table, the socket mode, the
stale-socket replacement, the missing-directory refusal, and the
not-loaded refusals over a real socket. The 5 GPU-gated ones skip with a
named reason so a run on a machine with no card says which thing was
missing. Do not run them while the service is up: two copies of the
weights do not fit on a 16 GiB card.

## The HTTP server is the standard library

`socketserver.UnixStreamServer` plus `http.server.BaseHTTPRequestHandler`
and nothing else. No fastapi, no uvicorn, no starlette, no pydantic.

Two reasons. A sealed appliance counts every package as supply chain, and
a web framework is a large dependency tree in front of three endpoints
whose bodies are four fields. And the GPU serializes requests anyway: one
forward pass owns the card, so a single-threaded server that handles one
request at a time is a match for the workload rather than a limitation of
it. An async framework would buy concurrency the hardware cannot spend.

The server keeps no connection alive. It answers with `Connection: close`
and closes, because a single-threaded server that honored keep-alive
would let one idle client hold the only worker, and a Unix socket setup
costs nothing next to a forward pass.

## The seal

The service reaches nothing.

- `HF_HUB_OFFLINE=1` and `TRANSFORMERS_OFFLINE=1` are set by the process
  itself, before `transformers` is imported. The unit sets them too. The
  point of setting them is that a code path reaching for the network
  fails on the variable and says so, rather than failing on the absent
  network namespace and not saying so. The possibility is real: the
  snapshot's `modeling_jina_embeddings_v4.py` imports `requests` into the
  embedding path.
- Weights are present or the service does not start. `resolve_weights`
  names the snapshot directory from the hub cache's own layout, checks
  for `config.json` and `adapters/adapter_config.json`, and refuses with
  the path it looked at. There is no download, no fetch on first use, and
  no cache warm. This is R25.
- The socket is mode 0600 in a directory the service refuses to create.
  A stale socket file from a killed service is replaced on bind, which is
  `yeomna-daemon`'s `bind()` behavior, line for line.
- Nothing logs request content. Counts and field names are not content,
  which the contract settles by writing "18204 tokens exceeds max_tokens
  16384" as its own example message. A body that will not parse produces
  "the request body is not valid UTF-8 JSON" and not the body.

## The pins, and why each one is there

`transformers>=4.35,<5.0` is the one that is mandatory. transformers 5.x
breaks the snapshot's cached custom code through `ROPE_INIT_FUNCTIONS`
drift, measured by the spike, and the reference's own bound is what let
the spike load at all. The resolution in `uv.lock` is 4.57.6.

The other six are there because the model's own custom code imports them.
`peft` carries the base class of `MultiAdapterLinear`, which is how one
set of weights serves three LoRA adapters. `accelerate` is what
`from_pretrained`'s device placement wants. `pillow` and `torchvision`
come in through the vision tower the text path never uses. `requests` is
the network library named above. `torch` is the point.

`requires-python` is `>=3.12,<3.13`, which is what the resolved torch and
transformers wheels publish. A newer interpreter is a resolution failure
rather than a quiet fallback, which is the posture the appliance takes to
a config file it cannot parse.

`uv.lock` is committed. The resolution is the pin, and it happens on a
developer machine rather than on the appliance.

## The four tasks, and the prefix each one gets

The contract names four tasks. The model has three LoRA adapters and two
prefixes. The mapping:

| Contract `task` | `task_label` adapter | Processor prefix |
|---|---|---|
| `retrieval.passage` | `retrieval` | `Passage` |
| `retrieval.query` | `retrieval` | `Query` |
| `text-matching` | `text-matching` | `Query` |
| `code` | `code` | `Query` |

The prefix strings are read out of the snapshot rather than guessed.
`modeling_jina_embeddings_v4.py` line 33 is
`PREFIX_DICT = {"query": "Query", "passage": "Passage"}`, and
`process_texts` builds `f"{prefix}: {text}"`, so `"Passage: "` is nine
bytes and `"Query: "` is seven. Those two are the whole set the model was
trained with, and `process_texts` would accept any string at all, which
is why the value comes from the model's code and not from this file's
judgment.

**The prefix for `code` is a gap in the contract, not a decision this
service is confident about.** The snapshot's `_validate_encoding_params`
forces `PREFIX_DICT["query"]` for `text-matching` whatever `prompt_name`
says, so `text-matching` is settled by the model itself. For `code` it is
not. `encode_text` defaults `prompt_name` to `"query"`, so the model's
own default path prefixes code with `"Query: "`, and that is what this
service does. But the contract's `code` task carries no query-or-passage
distinction, while `retrieval` does, and a corpus of code embedded with
the query prefix and then searched with the query prefix is at least
self-consistent. If the intent was that ingested code is a passage, the
contract needs a `code.passage` and a `code.query` the way retrieval has
them, and that is a ruling rather than an implementation choice. Recorded
here rather than decided quietly.

## The pooling, and how it is kept identical to Rust

`window_boundaries` and `mean_pool_and_normalize` are pure functions with
no torch in them, so the no-GPU tests exercise the arithmetic that
matters. They reproduce `crates/yeomna-chunking/src/late.rs`:

```
step  = max(size - overlap, 1)
start = 0, step, 2*step, ...  while start < n
end   = min(start + size, n)
```

A window per start, stopping at the first window whose end reaches `n`,
so the windows tile `0..n` with no gap and the last one clamps. The floor
of 1 on the step is what makes an overlap at or above the chunk size
terminate rather than loop forever. Pooling is the mean of the window
followed by L2 normalization, with `late.rs`'s two rules kept: a zero
vector stays zero, and a vector whose squared sum overflows to infinity
is left untouched rather than collapsed to zeros.

The tests pin this against `late.rs`'s own unit tests, value for value,
and against the spike's measured 29 chunks over 8,631 tokens at the
default 500 by 200.

Reading both files is not the proof. `/v1/tokens` is. It returns the
token-level hidden states for one short text, and the test pools them and
requires the service's own chunk vectors to match at cosine 0.9999 or
better. Measured here at 0.99999994 over ten windows of a 419-token text.
The Rust oracle does the same through `late_chunk_embeddings`.

## Byte offsets: the two conversions the service owns

Byte spans on the wire are UTF-8 byte offsets into the caller's own text.
Two conversions get it there, and the client must not repeat either.

**Character to byte.** HF fast tokenizers report character offsets. Every
consumer here slices bytes. `char_to_byte_table` builds the cumulative
map. On pure-ASCII input the two coincide, which is how the mismatch
stayed invisible until the spike's review caught it. A test embeds CJK and
an emoji and requires each chunk's byte span to slice text that
round-trips.

**Prefix rebase.** Offsets come back as offsets into the prefixed text.
The prefix is subtracted, and `token_count` excludes the prefix tokens
too, so a caller comparing `token_count` against `max_tokens` is
comparing the thing the ceiling is about.

The prefix token count is derived rather than assumed. A token whose
character span ends at or before the prefix's last character is prefix and
nothing else. A token that straddles the boundary is content, because
with this BPE tokenizer the prefix's trailing space attaches to the next
token: `"Passage: hello"` tokenizes as `Pass`, `age`, `:`, `" hello"`,
and that fourth token spans characters 8 to 14 across a nine-character
prefix. Dropping it would drop the caller's first word. It is kept and
its start clamps to byte 0.

The prefix tokens are then dropped from the hidden states, the offsets,
and the token count together, so all three describe the same
`token_count` things. That is what makes `/v1/tokens` an oracle for
`/v1/embed` rather than a second, differently indexed view.

**The retokenization guard.** Before anything is returned, the ids the
tokenizer produced are compared against the ids the forward pass saw. If
they differ the response is `internal`, because offsets that do not
describe the embedded tokens are worse than no answer. This is the spike's
own abort, and it is the reason its fixtures were trustworthy.

## One input per forward pass

`/v1/embed` accepts an array and loops, one forward pass per element,
rather than batching them into one pass.

Two reasons, and the first is correctness. `process_texts` pads a batch to
its longest member, and the model's own text pooling is a mean over the
attention mask. Pooling padded positions would be wrong, and the spike
could not have seen this because it embedded one document. A batch of one
has no padding to mask.

The second is the card. Peak memory is set by the longest input, so a
batch of two 16k-token documents needs twice the activations of the
largest thing that fits. One at a time keeps the ceiling a property of one
document.

## The ceiling is measured

`max_tokens` is 16384, and it is a hardware fact. Measured on GPU 2, an
RTX 2000 Ada with 16380 MiB, on 2026-09-09, with 370 MiB in use by
another process:

| Content tokens | Torch peak allocated | GPU 2 `memory.used` |
|---|---|---|
| idle, weights loaded | 7.82 GiB | 8330 MiB |
| 1,992 | 8.52 GiB | 9266 MiB |
| 7,998 | 11.07 GiB | 11926 MiB |
| 11,991 | 12.76 GiB | 13666 MiB |
| 13,993 | 13.61 GiB | 14566 MiB |
| 16,380 | 14.62 GiB | 15666 MiB |

16,384 tokens is accepted and 16,385 is refused with
`input-too-large: 16385 tokens exceeds max_tokens 16384`. The spike's
11.48 GiB at 8,631 tokens sits where this table says it should.

**`PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True` is load-bearing and
the service sets it itself.** Without it the default caching allocator
fragments across requests of different sizes, and on this card that is
the difference between the contract's ceiling holding and not: a process
using the default allocator embedded 13,993 tokens and then refused
14,500 with `out-of-memory`, while the same ladder under expandable
segments reached 16,380 and left 700 MiB spare. It is set in
`seal_environment` with `setdefault`, so an operator debugging an
allocation can override it, and it is set in the process rather than only
in the unit because the ceiling `/v1/info` advertises is a promise the
service has to keep on its own.

An input above the ceiling is refused, never truncated. Truncation would
drop a document's tail out of vector search while leaving it in full-text
search with nothing in any response saying so. The startup path also
refuses a `YEOMNA_EMBEDDER_MAX_TOKENS` within 64 of the processor's own
32768 clamp, because a ceiling that close to the clamp would let
`process_texts` truncate an accepted input.

## Two places the contract does not reach

Stated rather than diverged from quietly.

**An unknown path.** The contract's `code` values are a closed set and
none of them means "that is not an operation". `GET /v2/info` gets HTTP
404 and `POST /v1/info` gets HTTP 405, both carrying the normal envelope
with `code` `invalid-request`. No row of the contract's table describes a
routing condition, so no row is contradicted, but a client mapping code
to status will see a code it expected at 400 arrive at 404.

**`images`.** The contract says `images-unsupported` is for when "`images`
was provided". This service refuses when the key is present at all,
including `"images": null`, on the grounds that writing the field is
asking for the field.

## What the spike got wrong, and what it got right

Right, and copied: the forward call, the `output_vlm_last_hidden_states`
route to the 2048-dimension hidden states, the retokenization abort, the
character-to-byte conversion, the prefix rebase, the transformers pin, and
every measured number.

Wrong, or at least incomplete, and fixed here:

1. **Whole-sequence mean with no attention mask.** `jina_dump.py` pools
   `hs.mean(axis=0)`, which is correct only for a single unpadded input.
   `process_texts` pads a batch to its longest member and the model's own
   pooling is a mean over the mask. Answered by one forward pass per
   input, above.

2. **The revision pin does not cover the LoRA adapters.** The snapshot's
   `from_pretrained` reads adapters from `<name_or_path>/adapters` when
   `name_or_path` is a directory, and otherwise calls
   `snapshot_download(repo_id, allow_patterns=["adapters/*"])` with no
   `revision`, which resolves through `refs/main`. The spike passed the
   repo id, so its adapters came from whatever `refs/main` pointed at.
   On this machine `refs/main` happens to be `853c867b...`, so the spike's
   numbers stand, but it is a coincidence and not the pin working. This
   service resolves the snapshot directory itself and hands
   `from_pretrained` the path, which puts the adapters under the same pin
   as the weights and removes the `snapshot_download` call entirely.

3. **The prefix tokens were pooled into the vector.** The spike's
   whole-document pool included the `"Passage: "` tokens, which is fine
   for a document-level identity check against `single_vec_emb` (the
   model includes them too) but wrong for a chunk whose byte span is
   supposed to start at byte 0 of the caller's text. Here the prefix
   tokens are dropped from the hidden states before windowing.

4. **The spike's fixed 9-byte prefix subtraction assumed the prefix.**
   `"Query: "` is seven bytes, and three of the four contract tasks use
   it. The length is computed from the prefix the task receives.

## One note that is not a defect and is worth knowing

transformers 4.57 prints a warning when it loads this tokenizer, saying
the regex pattern is incorrect and that `fix_mistral_regex=True` would
fix it. The flag is **not** set. The snapshot's committed
`tokenizer.json` is the identity the revision pins, and changing how it
splits text would change every offset and every vector in a corpus
already keyed by `model_revision`. The retokenization guard compares the
service's tokenization against the forward pass's, and both use the same
tokenizer, so the guard holds either way. If a future transformers is
ever pinned upward, this warning is the first thing to check, because it
is a signal that the same revision can tokenize differently under a
different library version, which is the one thing `model_revision` in the
response cannot detect.

## Files

- `server.py`: the service. Errors, the pure windowing and pooling
  functions, the offset arithmetic, request validation, the weights
  resolution, the lazily loaded model, and the HTTP and socket layers.
- `test_server.py`: 59 tests, 54 of which need no card and no weights.
- `pyproject.toml` and `uv.lock`: the pins.
