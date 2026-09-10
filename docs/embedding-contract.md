# The Yeomna Embedding Contract, v1

Wire package: `yeomna.embedding`. Transport: HTTP/1.1 with JSON bodies
over a Unix domain socket. Path prefix: `/v1`.

Editorial rules: ASCII only, no em-dashes, no semicolons, never the
words genuinely, honestly, or actually. These govern prose. JSON keeps
its syntax.

Parent PRD: `docs/PRD-embedder.md`. This document is normative for the
wire. Where it and an implementation disagree, this document wins and
the implementation is the defect.

## What this is, and what it is not

This is the contract between the appliance and whatever holds the
embedding model. It is a fork of the reference's PE-API v1, not a
revision of it, and the fork is in the response shape.

**Kept from PE-API:** a response is always chunked. There is no
single-vector mode and no flag that produces one. Late chunking is the
retrieval-quality feature the product depends on, and a contract that
lets a caller ask for the unchunked shape is a contract that will be
asked for it.

**Changed from PE-API:** the transport is a Unix socket and cannot be
anything else, the OpenAI request kinship is dropped along with the
`model` request field and the `encoding_format` field, errors are a
Yeomna envelope rather than an OpenAI one, and a third operation exists
whose only consumer is a test.

**Why the transport is not negotiable.** Charter section 5.1: "Embedding
is local. The embedder is GPU-resident on the same machine, so document
text is never transmitted to an embedding API. This is a requirement,
not a configuration default." A requirement that a config key can turn
off is a preference, so the client type carries no network variant and
this contract names no host.

## Who pools, and why the service does

The service returns pooled chunk vectors. The alternative, where the
service returns token-level hidden states and the client pools them, is
the arm the spike proved (`spikes/jina-late-loop`, cosine 0.999999
against the model's own pooling). It was ruled against on transport
grounds, which the spike could not have weighed because it wrote fixture
files to a local directory.

One 8,631-token document is 8631 by 2048 floats. That is 70 MB raw, and
about 212 MB once JSON has written each float as decimal text. The same
document's pooled response is 29 vectors, about 240 KB. Ingesting a
100-document corpus would move roughly 20 GB of JSON text to compute
about 12 MB of vectors.

Chunking policy does not move to the service. `chunk_size_tokens` and
`chunk_overlap_tokens` are request fields, so the service executes a
policy and never owns one. What moves is the arithmetic, into the process
that already holds the weights the arithmetic came from.

## `GET /v1/info`

No request body.

```jsonc
{
  "model": "jinaai/jina-embeddings-v4",
  "model_revision": "853c867b65b749f3c3c72a06868140d842e04f06",
  "dimension": 2048,
  "max_tokens": 16384,
  "tasks": ["retrieval.passage", "retrieval.query", "text-matching", "code"],
  "device": "cuda:2",
  "loaded": true
}
```

Every field is required. `model_revision` is the full model snapshot SHA
and is what makes two corpora comparable or not, so it is reported rather
than assumed. `max_tokens` is the service's real ceiling and not the
model's advertised context: see the ceiling section below.

`loaded` is false while the weights are still loading. A client that gets
`loaded: false` should expect `POST /v1/embed` to refuse with
`model-not-loaded` rather than block.

## `POST /v1/embed`

The production operation.

```jsonc
{
  "input": "long document text",          // string, or array of strings
  "task": "retrieval.passage",            // required, must be in /v1/info tasks
  "chunk_size_tokens": 500,               // optional, default 500
  "chunk_overlap_tokens": 200             // optional, default 200
}
```

There is no `model` field. One service serves one model, `/v1/info` says
which, and a request that named a model would be a request that could
name the wrong one.

Response:

```jsonc
{
  "model": "jinaai/jina-embeddings-v4",
  "model_revision": "853c867b65b749f3c3c72a06868140d842e04f06",
  "task": "retrieval.passage",
  "dimension": 2048,
  "results": [
    {
      "index": 0,                         // position in the request's input array
      "token_count": 8631,                // tokens the forward pass saw, prefix excluded
      "chunks": [
        {
          "chunk_index": 0,
          "total_chunks": 29,
          "vector": [0.0123, -0.0456],    // `dimension` floats, unit norm
          "start_token": 0,
          "end_token": 500,
          "start_byte": 0,
          "end_byte": 1974
        }
      ]
    }
  ]
}
```

`results` is in request order and `index` is redundant with position on
purpose, so a mis-ordered response is detectable rather than silently
mismatched against the wrong input.

**Every chunk carries both coordinate systems.** The token range is what
the pooling used. The byte span is what the store holds, because
`chunks.start_char` and `chunks.end_char` are byte offsets. The mapping
between them is where the spike found a defect, so both are shipped and
the client can check one against the other.

### The windowing rule, stated so two implementations cannot drift

Given `n` tokens, size `s`, and overlap `v`:

```
step  = max(s - v, 1)
start = 0, step, 2*step, ...  while start < n
end   = min(start + s, n)
```

A window is emitted for each `start`, and iteration stops at the first
window whose `end` reaches `n`. The windows therefore tile `0..n` with no
gap and the last one clamps. This is `late_chunk_embeddings` in
`crates/yeomna-chunking/src/late.rs`, and the floor of 1 on the step is
what makes an overlap at or above the chunk size terminate rather than
loop forever.

Pooling is the mean of the window's token vectors followed by L2
normalization, which is the model's own text pooling. Every returned
vector is unit norm.

### Byte offsets, and the two obligations

Byte spans are UTF-8 byte offsets into **the caller's own text**, which
means two conversions the service owns and the client must not repeat.

1. **Character to byte.** HF fast tokenizers report character offsets.
   Every consumer here slices bytes. On pure-ASCII input the two
   coincide, which is how the mismatch stayed invisible until the spike's
   review caught it.
2. **Prefix rebase.** The model is fed a task prefix (`"Passage: "`, nine
   bytes). Offsets from the tokenizer are into the prefixed text, and the
   prefix length is subtracted before the span crosses the wire.

`token_count` likewise excludes the prefix tokens, so a caller comparing
it against the ceiling is comparing the thing the ceiling is about.

### The context ceiling

`max_tokens` is 16,384, and it is a hardware fact rather than a
configuration choice. The model advertises 32k. Measured on GPU 2, an
8,631-token pass peaked at 11.48 GiB of the card's 16 GiB, with 7.5 to 8
GiB of that being weights. A 32k pass does not fit.

Re-measured during the build, at the ceiling itself:

| content tokens | torch peak | nvidia-smi |
|---|---|---|
| idle, loaded | 7.82 GiB | 8,330 MiB |
| 7,998 | 11.07 GiB | 11,926 MiB |
| 13,993 | 13.61 GiB | 14,566 MiB |
| 16,380 | 14.62 GiB | 15,666 MiB |

**The ceiling holds only with `PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True`,
and that condition is part of the contract rather than a deployment note.**
Under the default caching allocator the real ceiling measured about 14k: a
fresh process embedded 13,993 tokens and then refused 14,500 with
`out-of-memory`, because the allocator's fixed segments fragment under the
growing activation sizes a single long pass asks for. With expandable
segments the whole ladder up to 16,384 passes. A service that advertises
`max_tokens` it cannot reach is a service whose refusals are unpredictable,
so an implementation sets the variable itself before importing torch rather
than relying on its unit file to do it.

Batching interacts with this. One forward pass per input, never a padded
batch: the model's own pooling is a mean over the attention mask, so
pooling a padded batch would pool pad positions, and one pass per input
also keeps the peak at the cost of the largest single document, which is
what the ceiling is about.

An input above the ceiling is **refused**, never truncated. Truncation
would drop a document's tail out of vector search while leaving it in
full-text search, with nothing in any response saying so. A refusal is
loud, and the caller counts it.

## `POST /v1/tokens`

The verification operation. Its only consumer is a test.

```jsonc
{ "input": "a short text", "task": "retrieval.passage" }
```

```jsonc
{
  "model": "jinaai/jina-embeddings-v4",
  "model_revision": "853c867b...",
  "task": "retrieval.passage",
  "dimension": 2048,
  "token_count": 7,
  "prefix_bytes": 9,
  "offsets": [[0, 1], [1, 6]],            // byte pairs, prefix already rebased
  "hidden_states": [[0.01, -0.02]]        // token_count arrays of `dimension` floats
}
```

Capped at **512 tokens**. Enough to pool several chunks at the default
500-token window and prove the seam. Far too small to ingest with, which
is the point: this operation exists so that the pooling identity is
checked by machine rather than asserted in prose, and it must not become
the production path by convenience.

The test that consumes it asks for both views of one short text, pools
the token view through `late_chunk_embeddings`, and requires cosine
0.9999 or better against every chunk the service pooled.

## Errors

One envelope, and the `code` values are a closed set.

```jsonc
{ "error": { "code": "input-too-large", "message": "18204 tokens exceeds max_tokens 16384" } }
```

| Code | HTTP | Meaning |
|---|---|---|
| `invalid-request` | 400 | Malformed body, missing `input`, missing `task`, or an empty input string |
| `unknown-task` | 400 | `task` is not in `/v1/info`'s list. The service never routes to a different adapter |
| `images-unsupported` | 400 | `images` was provided. Refused rather than ignored, because a text-only embedding of a request that asked for multimodal is quietly wrong |
| `mixed-task-batch` | 400 | One request named more than one task. v1 does not reload an adapter mid-batch |
| `input-too-large` | 400 | Above `max_tokens`, with the count in the message |
| `token-cap-exceeded` | 400 | `/v1/tokens` above 512, naming the cap and why it exists |
| `model-not-loaded` | 503 | Weights are still loading. Retryable |
| `out-of-memory` | 503 | GPU OOM on this batch. Retryable, and the client halves and retries |
| `internal` | 500 | Anything else, with no request content in the message |

An empty input string is `invalid-request` rather than a zero vector,
because a zero vector has no cosine and would sit in an HNSW index
answering every query equally badly.

**Two cases the code set does not name.** An unknown path answers HTTP 404
and a known path with the wrong method answers HTTP 405, both carrying the
normal envelope with `code: invalid-request`. The nine codes all describe
conditions of a request body, and none of them means "not an operation", so
a client mapping code to status will see `invalid-request` arrive at a
status the table does not list. Stated here rather than left to be
discovered.

`internal` messages carry no request content. The corpus does not leave
the box, and that includes leaving it through a log line.

## The socket

One socket, mode 0600, in a directory the appliance keeps at 0700.
Filesystem permission is the gate, which is the same model the store and
the daemon use. A stale socket file from a killed service is replaced on
bind. A missing directory is a refusal to start rather than a directory
the service creates, because a service that creates its own socket
directory can create it in the wrong place.

The service reaches nothing. The unit carries `PrivateNetwork=yes`, so
the network library the model's own code imports is inert for want of a
namespace, `RestrictAddressFamilies=AF_UNIX` so the kernel refuses
anything else, and `HF_HUB_OFFLINE=1` with `TRANSFORMERS_OFFLINE=1` so a
code path that reaches for the network fails on the variable and says so
before it fails on the namespace and does not.

Weights are present or the service does not start. There is no download,
no fetch on first use, and no cache warm. This is charter 5.1's third
boundary, Updates, and it is proposed as **R25** in the parent PRD on the
same grounds R24 gave for `tools install`: a byte arriving from the
network into a sealed appliance decides what the appliance may talk to
and how a release is verified, which is a charter question and not a
convenience.

## Two things the revision pin does not cover

Both were found while building the first implementation, and both are
recorded because `model_revision` on the response is what a reader will
otherwise assume covers them.

**The LoRA adapters.** The model's own `from_pretrained` calls
`snapshot_download` for `adapters/*` **with no revision** when it is handed
a repository id, so the adapters resolve through `refs/main` while the
weights resolve through the pin. The spike passed a repository id, which
means its adapters came from wherever `refs/main` pointed. On this machine
`refs/main` happens to be the pinned SHA, so the spike's numbers stand by
coincidence rather than by the pin. An implementation must resolve the
snapshot directory itself and hand `from_pretrained` a path, which reads
the adapters from inside the pinned snapshot and removes the download call.
That also satisfies R25 without a library call that could reach the
network.

**The tokenizer, under a different library version.** transformers 4.57
warns that this snapshot's tokenizer regex is incorrect and offers to fix
it. The flag is not set, because the committed `tokenizer.json` is part of
the identity the revision pins and changing how it splits text would change
every offset and every vector in a corpus keyed by `model_revision`. The
retokenization guard does not catch this, since both sides of it use the
same tokenizer. It is the one drift a revision cannot detect, and the pin
that does cover it is the committed dependency lock.

## Versioning

`v1` is this document. A change that a v1 client would misread is a v2
and gets a new path prefix, because two shapes on one path is how the
reference ended up with a contract that promised late chunking and a
service that returned single vectors.
