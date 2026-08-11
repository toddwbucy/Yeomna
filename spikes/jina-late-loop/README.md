# Spike: Jina v4 late-chunking loop

Branch deliverable for `spike/spu-embed-loop`, run 2026-08-10 on GPU 2, the
Yeomna-assigned card. Everything here is evidence for the Phase 3 embedder
contract ruling. Nothing here ships.

## What ran

1. `jina_introspect.py` loads `jinaai/jina-embeddings-v4` fully offline
   (`HF_HUB_OFFLINE=1`, weights and custom code already in
   `/opt/weaver/huggingface`) on `cuda:2` and prints the API surface.
2. `jina_dump.py` encodes the charter (`README.md`, 8,631 tokens) in one
   full-document pass and dumps token-level fixtures: last hidden states
   (8631 x 2048), Jina's own single vector (2048), the multivector
   (8631 x 128), and tokenizer byte offsets.
3. `loop/` is a Rust harness feeding those fixtures to
   `yeomna_chunking::late_chunk_embeddings`, its first real caller.

Regenerate fixtures with the venv of your choice, then:

```bash
python jina_dump.py            # writes ./fixtures (about 80MB, gitignored)
cd loop && cargo run --release -- ../fixtures
```

Reproducibility hard points, added after review: the model snapshot is pinned
by full SHA in both scripts and recorded in `meta.json`. The dump aborts
before writing anything if retokenization does not reproduce the forward's
ids. The source document is copied into the fixture directory and the harness
verifies its SHA-256 against the recorded hash before slicing, so the harness
reads nothing outside the fixture directory.

## Results

| Check | Result |
|---|---|
| Whole-doc pool through `late.rs` vs Jina's `single_vec_emb` | cosine 0.999999 |
| Chunks at default 500/200 over 8,631 tokens | 29, boundaries tile 0..8631 |
| Chunk embedding norms | all unit |
| Chunk-to-document cosine range | 0.8870 to 0.9691 |
| Token window to byte span via offsets | clean, chunk 1 decoded back to charter text |
| Peak VRAM at 8.6k tokens | 11.48 GiB of 16 |

## What this settles, as evidence

**The lifted pooling is Jina's pooling.** Jina's text single-vector is mean
pool over the attention mask then L2 normalize, which is line for line what
`mean_pool_and_normalize` in `yeomna-chunking` does. The 0.999999 cosine is
that identity measured, and it validates the CodeRabbit-tightened
normalization on real model output.

**Late chunking pools the 2048-d hidden states.** Chunk vectors land in the
same space as Jina's document vectors, compatible with a `halfvec(2048)`
column. The 128-d multivector output is a different feature, a late
interaction projection, and pooling it would be wrong. The model's own
`forward` exposes the hidden states first class via
`output_vlm_last_hidden_states`.

**Boundary metadata: token ranges on the wire, bytes at the edge.** Late
boundaries live in token space. Tokenizer offsets convert them to byte spans,
with two obligations. The task prefix (`"Passage: "`, 9 bytes) must be tracked
and rebased. And HF fast tokenizers report **character** offsets, so the dump
converts them to UTF-8 byte offsets before writing, since the consumer slices
bytes. On pure-ASCII input the two coincide, which is how the mismatch stayed
invisible until review. Retokenization must reproduce the forward's ids or the
dump aborts.

**What an embedder backend must expose for this design:** token-level last
hidden states plus tokenizer offsets. Pooling then lives client side in
`late.rs`, which this spike proves works. This is the existence proof for the
client-pools arm of the Phase 3 fork.

## Operational notes

- GPU 2 is the Yeomna card. 32k context has fit on it before, roughly 7.5 to
  8 GB of weights leaving about 8 GB for context. Testing runs at 16k, which
  is plenty. The 11.48 GiB peak here was an 8.6k-token document.
- The Python loader needed seven packages (`torch`, `transformers`, `peft`,
  `accelerate`, `pillow`, `requests`, `torchvision`) and a version pin to
  load at all. `transformers` 5.x breaks the cached custom code
  (`ROPE_INIT_FUNCTIONS` drift), and the reference's `>=4.35,<5.0` pin is
  what saved it. The modeling file itself imports `requests`, a network
  library, into the embedding path. All of this is the case for the SPU
  backend, recorded while it was fresh.
- The backbone confirmed from the traceback: `Qwen2_5_VLTextModel`.
