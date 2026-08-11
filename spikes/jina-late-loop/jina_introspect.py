"""Stage A step 1: load Jina v4 offline on cuda:2, introspect its surface."""
import os, inspect
os.environ["HF_HOME"] = "/opt/weaver/huggingface"
os.environ["HF_HUB_OFFLINE"] = "1"
os.environ["TRANSFORMERS_OFFLINE"] = "1"

import torch
from transformers import AutoModel, AutoTokenizer

M = "jinaai/jina-embeddings-v4"
# Two snapshots exist in the local cache. Pin the one this spike ran on, so
# both weights and the trust_remote_code modeling file are the same bytes on
# every rerun.
REVISION = "853c867b65b749f3c3c72a06868140d842e04f06"
tok = AutoTokenizer.from_pretrained(M, trust_remote_code=True, revision=REVISION)
model = AutoModel.from_pretrained(
    M, trust_remote_code=True, revision=REVISION, torch_dtype=torch.float16
)
model = model.to("cuda:2")
model.requires_grad_(False)

print("class:", type(model).__name__)
print("config dims:", getattr(model.config, "hidden_size", "?"))
for name in ("encode_text", "encode_image", "forward"):
    fn = getattr(model, name, None)
    if fn:
        try:
            print(f"\n{name}{inspect.signature(fn)}")
        except (ValueError, TypeError):
            print(f"\n{name}: (signature unavailable)")

# Tiny smoke encode, both output modes
with torch.inference_mode():
    sv = model.encode_text(texts=["a sealed appliance"], task="retrieval", prompt_name="passage")
    mv = model.encode_text(texts=["a sealed appliance"], task="retrieval", prompt_name="passage", return_multivector=True)
s = sv[0] if isinstance(sv, list) else sv
m = mv[0] if isinstance(mv, list) else mv
print("\nsingle-vector:", tuple(s.shape), s.dtype)
print("multivector:  ", tuple(m.shape), m.dtype)
print("\nVRAM used:", round(torch.cuda.memory_allocated(2)/2**30, 2), "GiB")
