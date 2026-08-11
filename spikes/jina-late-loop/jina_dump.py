"""Stage A step 2: encode the charter, dump token-level fixtures for late.rs."""
import os, json, hashlib
os.environ["HF_HOME"] = "/opt/weaver/huggingface"
os.environ["HF_HUB_OFFLINE"] = "1"
os.environ["TRANSFORMERS_OFFLINE"] = "1"

import numpy as np
import torch
from transformers import AutoModel

OUT = os.path.dirname(os.path.abspath(__file__)) + "/fixtures"
os.makedirs(OUT, exist_ok=True)

doc = open("/home/todd/git/Yeomna/README.md").read()
M = "jinaai/jina-embeddings-v4"
model = AutoModel.from_pretrained(M, trust_remote_code=True, torch_dtype=torch.float16).to("cuda:2")
model.requires_grad_(False)

PREFIX = "Passage"
inputs = model.processor.process_texts([doc], max_length=32768, prefix=PREFIX)
inputs = {k: v.to("cuda:2") for k, v in inputs.items()}
seq = inputs["input_ids"].shape[1]
print("tokens:", seq)

with torch.inference_mode():
    out = model(task_label="retrieval", **inputs, output_vlm_last_hidden_states=True)

hs = out.vlm_last_hidden_states[0].float().cpu().numpy()      # (seq, 2048)
sv = out.single_vec_emb[0].float().cpu().numpy()              # (2048,)
mv = out.multi_vec_emb[0].float().cpu().numpy()               # (seq, 128)
print("hidden:", hs.shape, "single:", sv.shape, "multi:", mv.shape)
print("peak VRAM:", round(torch.cuda.max_memory_allocated(2)/2**30, 2), "GiB")

# Sanity: our pooling math (mean over mask + L2) must reproduce single_vec.
pool = hs.mean(axis=0)
pool = pool / np.linalg.norm(pool)
print("pool-vs-single cosine:", float(np.dot(pool, sv)))

# Offsets: retokenize the prefixed text, confirm ids match the forward's.
prefixed = f"{PREFIX}: {doc}"
tok = model.processor.tokenizer
enc = tok(prefixed, return_offsets_mapping=True, return_tensors="pt", max_length=32768, truncation=True)
ids_match = bool((enc["input_ids"][0] == inputs["input_ids"][0].cpu()).all())
print("retokenized ids match forward ids:", ids_match)
prefix_chars = len(f"{PREFIX}: ")

np.save(f"{OUT}/hidden_states.npy", hs)
np.save(f"{OUT}/single_vec.npy", sv)
np.save(f"{OUT}/multi_vec.npy", mv)
# Raw f32 little-endian for the Rust reader, plus metadata.
hs.astype("<f4").tofile(f"{OUT}/hidden_states.f32")
sv.astype("<f4").tofile(f"{OUT}/single_vec.f32")
meta = {
    "doc": "README.md",
    "doc_sha256": hashlib.sha256(doc.encode()).hexdigest(),
    "model": M,
    "task": "retrieval",
    "prompt_name": "passage",
    "prefix_chars": prefix_chars,
    "seq_len": int(seq),
    "hidden_dim": int(hs.shape[1]),
    "multivec_dim": int(mv.shape[1]),
    "ids_match": ids_match,
    "offsets_in_prefixed_text": [[int(a), int(b)] for a, b in enc["offset_mapping"][0].tolist()],
}
json.dump(meta, open(f"{OUT}/meta.json", "w"))
print("fixtures written to", OUT)
