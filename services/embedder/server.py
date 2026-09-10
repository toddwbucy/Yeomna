"""The Yeomna embedding service, yeomna.embedding v1.

Three operations over HTTP/1.1 with JSON bodies on a Unix socket, per
`docs/embedding-contract.md`, which is normative. Where this file and
that document disagree, that document wins and this file is the defect.

The HTTP layer is the Python standard library and nothing else. A sealed
appliance counts every package as supply chain, and the GPU serializes
requests anyway, so a single-threaded server is a match for the workload
rather than a limitation of it.

Nothing in this file logs request content. The corpus does not leave the
box, and a log line is a way out of the box.
"""

from __future__ import annotations

import http.server
import json
import logging
import os
import signal
import socketserver
import sys
import threading
import time
from pathlib import Path
from typing import Any, Sequence

import numpy as np

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

DEFAULT_MODEL = "jinaai/jina-embeddings-v4"
# The full snapshot SHA is the identity of the weights and of the
# trust_remote_code modeling file together. /v1/info reports it because
# it is what makes two corpora comparable or not.
DEFAULT_REVISION = "853c867b65b749f3c3c72a06868140d842e04f06"
DEFAULT_DEVICE = "cuda:2"
DEFAULT_WEIGHTS = str(Path.home() / ".cache" / "huggingface")
DEFAULT_SOCKET = str(Path.home() / ".local" / "share" / "yeomna" / "run" / "embedder.sock")

# 16384, not the model's advertised 32768. Measured on GPU 2 by the
# spike: an 8,631-token pass peaked at 11.48 GiB of the card's 16 GiB,
# 7.5 to 8 GiB of which is weights. A 32k pass does not fit.
DEFAULT_MAX_TOKENS = 16384

# The snapshot's JinaEmbeddingsV4Processor clamps its own max_length to
# text_max_length, which is 32768. A ceiling above that would let the
# processor truncate silently, which is the one thing the contract
# forbids, so a ceiling above it is refused at startup instead.
PROCESSOR_TEXT_MAX_LENGTH = 32768

# The verification operation's cap. Enough to pool several chunks at the
# default 500-token window and prove the pooling identity by machine.
# Far too small to ingest with, which is the point.
TOKENS_CAP = 512

DIMENSION = 2048

DEFAULT_CHUNK_SIZE_TOKENS = 500
DEFAULT_CHUNK_OVERLAP_TOKENS = 200

# A 16k-token document is about 64 KB of UTF-8. An array of a hundred of
# them is about 6 MB. The cap is generous and finite.
MAX_BODY_BYTES = 64 * 1024 * 1024

# Contract task name -> (LoRA adapter name, processor prefix).
#
# `task_label` is the adapter the forward pass selects. The prefix is the
# string process_texts puts in front of the text as `f"{prefix}: {text}"`.
# The snapshot's own modeling file fixes the two prefixes that exist:
# PREFIX_DICT = {"query": "Query", "passage": "Passage"}.
#
# text-matching takes "Query" because the snapshot's
# _validate_encoding_params forces PREFIX_DICT["query"] for that task
# whatever prompt_name says. code takes "Query" because encode_text
# defaults prompt_name to "query" and the contract's `code` task carries
# no query-or-passage distinction to read one from. See the README's
# "The prefix for code" section: this is a gap in the contract rather
# than a choice this service is confident about.
TASKS: dict[str, tuple[str, str]] = {
    "retrieval.passage": ("retrieval", "Passage"),
    "retrieval.query": ("retrieval", "Query"),
    "text-matching": ("text-matching", "Query"),
    "code": ("code", "Query"),
}

LOG = logging.getLogger("yeomna.embedder")


class Settings:
    """What the environment says, read once at startup."""

    def __init__(self, env: dict[str, str] | None = None) -> None:
        e = os.environ if env is None else env
        self.model = e.get("YEOMNA_EMBEDDER_MODEL", DEFAULT_MODEL)
        self.revision = e.get("YEOMNA_EMBEDDER_REVISION", DEFAULT_REVISION)
        self.device = e.get("YEOMNA_EMBEDDER_DEVICE", DEFAULT_DEVICE)
        self.socket_path = e.get("YEOMNA_EMBEDDER_SOCKET", DEFAULT_SOCKET)
        self.weights = e.get("YEOMNA_EMBEDDER_WEIGHTS", DEFAULT_WEIGHTS)
        raw = e.get("YEOMNA_EMBEDDER_MAX_TOKENS", str(DEFAULT_MAX_TOKENS))
        try:
            self.max_tokens = int(raw)
        except ValueError:
            raise StartupRefusal(
                f"YEOMNA_EMBEDDER_MAX_TOKENS is {raw!r}, which is not an integer"
            ) from None
        if self.max_tokens < 1:
            raise StartupRefusal(
                f"YEOMNA_EMBEDDER_MAX_TOKENS is {self.max_tokens}, which is not a ceiling"
            )
        if self.max_tokens > PROCESSOR_TEXT_MAX_LENGTH - 64:
            raise StartupRefusal(
                f"YEOMNA_EMBEDDER_MAX_TOKENS is {self.max_tokens}, and the "
                f"processor clamps its own max_length to "
                f"{PROCESSOR_TEXT_MAX_LENGTH}. A ceiling that close to the "
                f"clamp would let the processor truncate an accepted input, "
                f"which the contract forbids"
            )


# ---------------------------------------------------------------------------
# Errors
# ---------------------------------------------------------------------------


class StartupRefusal(Exception):
    """The service declines to start. Never a fallback, always a stop."""


class ServiceError(Exception):
    """One of the contract's closed set of error codes.

    `message` never carries request content. Counts and field names are
    not content: the contract's own example message is
    "18204 tokens exceeds max_tokens 16384".
    """

    def __init__(self, code: str, status: int, message: str) -> None:
        super().__init__(message)
        self.code = code
        self.status = status
        self.message = message

    def envelope(self) -> dict[str, Any]:
        return {"error": {"code": self.code, "message": self.message}}


def invalid_request(message: str) -> ServiceError:
    return ServiceError("invalid-request", 400, message)


def unknown_task(named: str) -> ServiceError:
    # The name is echoed because it is the client's own vocabulary and
    # not corpus text, and a refusal a caller cannot map to its own bug
    # costs an exchange.
    return ServiceError(
        "unknown-task",
        400,
        f"task {named!r} is not served. This service serves "
        f"{sorted(TASKS)} and never routes to a different adapter",
    )


def images_unsupported() -> ServiceError:
    return ServiceError(
        "images-unsupported",
        400,
        "images were provided. This service embeds text only, and a "
        "text-only embedding of a request that asked for multimodal is "
        "quietly wrong, so the request is refused rather than narrowed",
    )


def mixed_task_batch() -> ServiceError:
    return ServiceError(
        "mixed-task-batch",
        400,
        "one request named more than one task. v1 does not reload an "
        "adapter mid-batch, so a batch carries one task or none",
    )


def input_too_large(count: int, ceiling: int) -> ServiceError:
    return ServiceError(
        "input-too-large", 400, f"{count} tokens exceeds max_tokens {ceiling}"
    )


def token_cap_exceeded(count: int) -> ServiceError:
    return ServiceError(
        "token-cap-exceeded",
        400,
        f"{count} tokens exceeds the /v1/tokens cap of {TOKENS_CAP}. The cap "
        "exists so this verification operation cannot become the production "
        "path by convenience. Use POST /v1/embed",
    )


def model_not_loaded(state: str) -> ServiceError:
    return ServiceError(
        "model-not-loaded",
        503,
        f"the weights are {state}. Retry, and read GET /v1/info for the "
        "loaded flag",
    )


def out_of_memory() -> ServiceError:
    return ServiceError(
        "out-of-memory",
        503,
        "the GPU ran out of memory on this batch. Retryable: halve the "
        "inputs and retry",
    )


def internal(message: str) -> ServiceError:
    return ServiceError("internal", 500, message)


def is_out_of_memory(exc: BaseException) -> bool:
    """Recognize a CUDA allocation failure across torch versions.

    `torch.OutOfMemoryError` and `torch.cuda.OutOfMemoryError` are both
    RuntimeError subclasses and which name exists depends on the version,
    so the type name and the message are what get checked.
    """
    if type(exc).__name__ == "OutOfMemoryError":
        return True
    text = str(exc).lower()
    return "out of memory" in text or "cuda error: out of memory" in text


# ---------------------------------------------------------------------------
# The windowing rule, and the pooling. No torch here on purpose.
# ---------------------------------------------------------------------------


def window_boundaries(
    num_tokens: int, chunk_size_tokens: int, overlap_tokens: int
) -> list[tuple[int, int]]:
    """Reproduce `late_chunk_embeddings`'s boundaries exactly.

    `crates/yeomna-chunking/src/late.rs` lines 51 through 88:

        step  = max(size - overlap, 1)
        start = 0, step, 2*step, ...  while start < n
        end   = min(start + size, n)

    A window is emitted per start and iteration stops at the first window
    whose end reaches n, so the windows tile 0..n with no gap and the
    last one clamps. The floor of 1 on the step is what makes an overlap
    at or above the chunk size terminate rather than loop forever.
    """
    if num_tokens <= 0:
        return []
    step = max(chunk_size_tokens - overlap_tokens, 1)
    out: list[tuple[int, int]] = []
    start = 0
    while start < num_tokens:
        end = min(start + chunk_size_tokens, num_tokens)
        out.append((start, end))
        if end >= num_tokens:
            break
        start += step
    return out


def mean_pool_and_normalize(rows: Any) -> np.ndarray:
    """Mean over the rows, then L2 normalize. `late.rs` lines 91 to 129.

    A zero vector stays zero rather than becoming NaN, and a vector whose
    squared sum overflows to infinity is left untouched rather than
    collapsed to zeros. Both rules are `l2_normalize_f32`'s.
    """
    a = np.asarray(rows, dtype=np.float32)
    if a.ndim != 2 or a.shape[0] == 0:
        return np.zeros((0,), dtype=np.float32)
    # An overflow to infinity is one of the two cases the rule names, so
    # it is a result rather than a warning.
    with np.errstate(over="ignore", invalid="ignore"):
        pooled = a.sum(axis=0, dtype=np.float32) / np.float32(a.shape[0])
        norm = np.sqrt(np.sum(pooled.astype(np.float32) ** 2, dtype=np.float32))
        if norm > np.float32(0.0) and np.isfinite(norm):
            pooled = pooled / norm
    return pooled.astype(np.float32, copy=False)


# ---------------------------------------------------------------------------
# Offsets: character to byte, and the prefix rebase
# ---------------------------------------------------------------------------


def char_to_byte_table(text: str) -> list[int]:
    """Cumulative UTF-8 byte position of every character boundary.

    `table[i]` is the byte offset of character `i`, and `table[len(text)]`
    is the byte length of the whole string. HF fast tokenizers report
    character offsets and every consumer here slices bytes. On pure-ASCII
    input the two coincide, which is how the mismatch stayed invisible
    until the spike's review caught it.
    """
    table = [0] * (len(text) + 1)
    pos = 0
    for i, ch in enumerate(text):
        pos += len(ch.encode("utf-8"))
        table[i + 1] = pos
    return table


def count_prefix_tokens(char_offsets: Sequence[Sequence[int]], prefix_chars: int) -> int:
    """How many leading tokens lie wholly inside the task prefix.

    A token whose character span ends at or before the prefix's last
    character is prefix and nothing else. A token that straddles the
    boundary is kept, because it carries the first characters of the
    caller's own text: with a BPE tokenizer the prefix's trailing space
    attaches to the next token, so `"Passage: hello"` tokenizes the
    fourth token as `" hello"` spanning characters 8 to 14 across a
    prefix of 9 characters. Dropping it would drop content.
    """
    n = 0
    for start, end in char_offsets:
        if end <= start:
            # A special token reports an empty span. Qwen2's tokenizer
            # adds none, and a tokenizer that did would report (0, 0)
            # here, which is inside any prefix.
            n += 1
            continue
        if end <= prefix_chars:
            n += 1
            continue
        break
    return n


def chunk_byte_span(
    char_offsets: Sequence[Sequence[int]],
    start_token: int,
    end_token: int,
    prefix_chars: int,
    prefix_bytes: int,
    byte_at: Sequence[int],
    doc_bytes: int,
) -> tuple[int, int]:
    """UTF-8 byte span of a token window, in the caller's own text.

    Two conversions, both owned here so the client never repeats them.
    Character offsets become byte offsets through `byte_at`, and the task
    prefix is subtracted so the span indexes the text the caller sent
    rather than the text the model saw. A token straddling the prefix
    boundary clamps to 0.
    """
    lo: int | None = None
    hi: int | None = None
    for i in range(start_token, end_token):
        a, b = char_offsets[i][0], char_offsets[i][1]
        if b <= a:
            continue
        a = min(max(a, prefix_chars), len(byte_at) - 1)
        b = min(max(b, prefix_chars), len(byte_at) - 1)
        ab, bb = byte_at[a], byte_at[b]
        lo = ab if lo is None else min(lo, ab)
        hi = bb if hi is None else max(hi, bb)
    if lo is None or hi is None:
        return (0, 0)
    start_byte = min(max(lo - prefix_bytes, 0), doc_bytes)
    end_byte = min(max(hi - prefix_bytes, 0), doc_bytes)
    if end_byte < start_byte:
        end_byte = start_byte
    return (start_byte, end_byte)


# ---------------------------------------------------------------------------
# Request validation
# ---------------------------------------------------------------------------


def _require_int(body: dict[str, Any], key: str, default: int, floor: int) -> int:
    if key not in body or body[key] is None:
        return default
    value = body[key]
    if isinstance(value, bool) or not isinstance(value, int):
        raise invalid_request(f"{key} must be an integer")
    if value < floor:
        raise invalid_request(f"{key} must be at least {floor}")
    return value


def _validate_task(body: dict[str, Any]) -> str:
    if "task" not in body or body["task"] is None:
        raise invalid_request("task is required and must be one of " + str(sorted(TASKS)))
    named = body["task"]
    if isinstance(named, list):
        # A list is the only way one request can name two tasks, since
        # `task` is a scalar field. EC-11: refuse rather than reload an
        # adapter mid-batch.
        distinct = {t for t in named if isinstance(t, str)}
        if len(named) != len(distinct) or len(distinct) != 1:
            raise mixed_task_batch()
        named = next(iter(distinct))
    if not isinstance(named, str):
        raise invalid_request("task must be a string")
    if named not in TASKS:
        raise unknown_task(named)
    return named


def _validate_input(body: dict[str, Any]) -> list[str]:
    if "input" not in body or body["input"] is None:
        raise invalid_request("input is required: a string, or an array of strings")
    value = body["input"]
    if isinstance(value, str):
        items = [value]
    elif isinstance(value, list):
        if not value:
            raise invalid_request("input is an empty array")
        for item in value:
            if not isinstance(item, str):
                raise invalid_request("every element of input must be a string")
        items = list(value)
    else:
        raise invalid_request("input must be a string or an array of strings")
    for i, item in enumerate(items):
        if item == "":
            # EC-1. A zero vector has no cosine and would sit in an HNSW
            # index answering every query equally badly.
            raise invalid_request(
                f"input at index {i} is the empty string, which has no embedding"
            )
    return items


class EmbedRequest:
    def __init__(self, inputs: list[str], task: str, size: int, overlap: int) -> None:
        self.inputs = inputs
        self.task = task
        self.chunk_size_tokens = size
        self.chunk_overlap_tokens = overlap


def validate_embed_request(body: Any) -> EmbedRequest:
    if not isinstance(body, dict):
        raise invalid_request("the request body must be a JSON object")
    if "images" in body:
        raise images_unsupported()
    task = _validate_task(body)
    inputs = _validate_input(body)
    size = _require_int(body, "chunk_size_tokens", DEFAULT_CHUNK_SIZE_TOKENS, 1)
    overlap = _require_int(body, "chunk_overlap_tokens", DEFAULT_CHUNK_OVERLAP_TOKENS, 0)
    return EmbedRequest(inputs, task, size, overlap)


def validate_tokens_request(body: Any) -> tuple[str, str]:
    if not isinstance(body, dict):
        raise invalid_request("the request body must be a JSON object")
    if "images" in body:
        raise images_unsupported()
    task = _validate_task(body)
    inputs = _validate_input(body)
    if len(inputs) != 1:
        raise invalid_request(
            "/v1/tokens takes one input. It is a verification operation and "
            "batching it has no consumer"
        )
    return inputs[0], task


# ---------------------------------------------------------------------------
# The weights, and the refusal to fetch them
# ---------------------------------------------------------------------------


def resolve_weights(model: str, revision: str, hf_home: str) -> Path:
    """Name the local snapshot directory, or refuse to start.

    R25: weights are present or the service does not start. There is no
    download, no fetch on first use, and no cache warm. This resolves the
    path by the hub cache's own layout rather than by asking a library,
    so the refusal happens before any code that could reach a network is
    imported.

    The directory is what gets handed to from_pretrained, which matters
    beyond the refusal. The snapshot's own from_pretrained reads the LoRA
    adapters from `<name_or_path>/adapters` when name_or_path is a
    directory and calls snapshot_download with no revision when it is a
    repo id. Passing the directory is what puts the adapters under the
    same pin as the weights.
    """
    if os.path.isabs(model):
        path = Path(model)
    else:
        repo_dir = "models--" + model.replace("/", "--")
        path = Path(hf_home) / "hub" / repo_dir / "snapshots" / revision
    if not path.is_dir():
        raise StartupRefusal(
            f"the weights for {model} at revision {revision} are not present "
            f"at {path}. This service never fetches weights (R25), so a "
            f"missing snapshot is a refusal to start rather than a download"
        )
    for needed in ("config.json", "adapters/adapter_config.json"):
        if not (path / needed).exists():
            raise StartupRefusal(
                f"the snapshot at {path} is missing {needed}, so it is not a "
                f"complete copy of {model}"
            )
    if not os.path.isabs(model) and path.name != revision:
        raise StartupRefusal(
            f"the resolved snapshot {path.name} is not the pinned revision {revision}"
        )
    return path


# ---------------------------------------------------------------------------
# The model, loaded lazily in a background thread
# ---------------------------------------------------------------------------


class Model:
    """The weights, and the two operations that need them.

    Loaded on a background thread so the bind does not wait on it.
    `/v1/info` answers immediately with `loaded: false` and `/v1/embed`
    refuses with `model-not-loaded` until the load finishes (EC-6).
    """

    def __init__(self, settings: Settings, weights: Path) -> None:
        self.settings = settings
        self.weights = weights
        self._state = "loading"
        self._detail = "loading"
        self._lock = threading.Lock()
        # The GPU serializes anyway. This makes that explicit rather than
        # incidental, and keeps peak VRAM the cost of one forward pass.
        self._gpu = threading.Lock()
        self._model: Any = None
        self._tokenizer: Any = None
        self._torch: Any = None
        # Peak allocation of the most recent forward pass, in GiB. Torch's
        # own measure, so it is comparable to the spike's 11.48 GiB at
        # 8,631 tokens and not to what nvidia-smi reports for the process.
        self._peak_gib = 0.0

    # -- state ------------------------------------------------------------

    @property
    def loaded(self) -> bool:
        with self._lock:
            return self._state == "ready"

    @property
    def state(self) -> str:
        with self._lock:
            return self._detail

    def start(self) -> threading.Thread:
        thread = threading.Thread(target=self._load, name="weights", daemon=True)
        thread.start()
        return thread

    def _load(self) -> None:
        began = time.monotonic()
        try:
            import torch
            from transformers import AutoModel, AutoTokenizer

            LOG.info("loading %s from %s", self.settings.model, self.weights)
            tokenizer = AutoTokenizer.from_pretrained(
                str(self.weights), trust_remote_code=True
            )
            if not tokenizer.is_fast:
                raise RuntimeError(
                    "the tokenizer is not a fast tokenizer, so it reports no "
                    "offsets and no byte span could be derived"
                )
            model = AutoModel.from_pretrained(
                str(self.weights),
                trust_remote_code=True,
                torch_dtype=torch.float16,
            )
            model = model.to(self.settings.device)
            model.requires_grad_(False)
            model.eval()
            with self._lock:
                self._torch = torch
                self._model = model
                self._tokenizer = tokenizer
                self._state = "ready"
                self._detail = "ready"
            LOG.info(
                "weights ready in %.1fs on %s",
                time.monotonic() - began,
                self.settings.device,
            )
        except BaseException as exc:  # noqa: BLE001 - the thread must report
            with self._lock:
                self._state = "failed"
                self._detail = f"not loaded: {type(exc).__name__}: {exc}"
            LOG.error("the weights failed to load: %s: %s", type(exc).__name__, exc)

    def _require(self) -> tuple[Any, Any, Any]:
        with self._lock:
            if self._state != "ready":
                raise model_not_loaded(self._detail)
            return self._torch, self._model, self._tokenizer

    # -- the forward pass -------------------------------------------------

    def _encode(self, text: str, task: str, ceiling: int, over: Any) -> dict[str, Any]:
        """One text, one forward pass. Returns the token-level view.

        One input per pass on purpose. process_texts pads a batch to its
        longest member, the model's own text pooling is a mean over the
        attention mask, and pooling padded positions would be wrong. A
        batch of one has no padding to mask. It also keeps peak VRAM the
        cost of the largest single input rather than of the batch, which
        matters on a 16 GiB card whose ceiling is one document.
        """
        torch, model, tokenizer = self._require()
        adapter, prefix = TASKS[task]
        joiner = f"{prefix}: "
        prefixed = joiner + text
        prefix_chars = len(joiner)
        prefix_bytes = len(joiner.encode("utf-8"))

        # Count before embedding, so the ceiling refuses rather than
        # truncates. No truncation here: this is the honest length.
        enc = tokenizer(
            prefixed, return_offsets_mapping=True, add_special_tokens=True
        )
        ids = list(enc["input_ids"])
        char_offsets = [tuple(pair) for pair in enc["offset_mapping"]]
        n_prefix = count_prefix_tokens(char_offsets, prefix_chars)
        token_count = len(ids) - n_prefix
        if token_count > ceiling:
            raise over(token_count)
        if token_count < 1:
            raise invalid_request(
                "the input produced no tokens of its own, so it has no embedding"
            )

        with self._gpu:
            on_cuda = self.settings.device.startswith("cuda") and torch.cuda.is_available()
            if on_cuda:
                torch.cuda.reset_peak_memory_stats(self.settings.device)
            try:
                inputs = model.processor.process_texts(
                    [text], max_length=len(ids), prefix=prefix
                )
                inputs = {
                    k: v.to(self.settings.device) if hasattr(v, "to") else v
                    for k, v in inputs.items()
                }
                forward_ids = inputs["input_ids"][0].tolist()
                # The spike's own guard. If retokenizing does not
                # reproduce the forward's ids then the offsets do not
                # describe the embedded tokens, and wrong offsets are
                # worse than no answer.
                if forward_ids != ids:
                    raise internal(
                        "retokenization did not reproduce the forward pass ids "
                        f"({len(forward_ids)} against {len(ids)}), so the byte "
                        "offsets would not describe the embedded tokens"
                    )
                with torch.inference_mode():
                    out = model(
                        task_label=adapter,
                        **inputs,
                        output_vlm_last_hidden_states=True,
                    )
                hidden = out.vlm_last_hidden_states[0].float().cpu().numpy()
                del out, inputs
                if on_cuda:
                    # The ceiling is a hardware fact, so the number that
                    # decides it is worth having in the log. A byte count
                    # is not request content.
                    self._peak_gib = torch.cuda.max_memory_allocated(
                        self.settings.device
                    ) / 2**30
            except ServiceError:
                raise
            except RuntimeError as exc:
                if is_out_of_memory(exc):
                    # EC-8. Retryable, and the client halves and retries.
                    if torch.cuda.is_available():
                        torch.cuda.empty_cache()
                    raise out_of_memory() from None
                raise

        if hidden.shape[0] != len(ids):
            raise internal(
                f"the forward pass returned {hidden.shape[0]} hidden states for "
                f"{len(ids)} tokens"
            )
        # The width, checked here and not only assumed from the contract.
        # YEOMNA_EMBEDDER_MODEL accepts a snapshot path, so a model of
        # another width can be loaded, and every response would still
        # report DIMENSION while carrying vectors of that other width. The
        # Rust client refuses a mismatch before anything is stored, so this
        # does not reach halfvec(2048), but a service that reports a
        # dimension it is not serving is lying to a caller that has no way
        # to check. Refuse rather than report.
        if hidden.shape[1] != DIMENSION:
            raise internal(
                f"the model produced {hidden.shape[1]}-wide hidden states and this "
                f"service serves {DIMENSION}. Load a model of the right width"
            )
        # Drop the prefix. token_count excludes the prefix tokens, so the
        # windows, the offsets, and the hidden states must all be the
        # same token_count things or /v1/tokens would not be an oracle
        # for /v1/embed.
        hidden = hidden[n_prefix:]
        return {
            "hidden": hidden,
            "char_offsets": char_offsets,
            "n_prefix": n_prefix,
            "token_count": token_count,
            "prefix_chars": prefix_chars,
            "prefix_bytes": prefix_bytes,
            "prefixed": prefixed,
        }

    def embed(self, request: EmbedRequest) -> dict[str, Any]:
        results = []
        for index, text in enumerate(request.inputs):
            enc = self._encode(
                text,
                request.task,
                self.settings.max_tokens,
                lambda n: input_too_large(n, self.settings.max_tokens),
            )
            byte_at = char_to_byte_table(enc["prefixed"])
            doc_bytes = len(text.encode("utf-8"))
            offsets = enc["char_offsets"]
            hidden = enc["hidden"]
            boundaries = window_boundaries(
                enc["token_count"],
                request.chunk_size_tokens,
                request.chunk_overlap_tokens,
            )
            chunks = []
            for chunk_index, (start, end) in enumerate(boundaries):
                vector = mean_pool_and_normalize(hidden[start:end])
                start_byte, end_byte = chunk_byte_span(
                    offsets,
                    start + enc["n_prefix"],
                    end + enc["n_prefix"],
                    enc["prefix_chars"],
                    enc["prefix_bytes"],
                    byte_at,
                    doc_bytes,
                )
                chunks.append(
                    {
                        "chunk_index": chunk_index,
                        "total_chunks": len(boundaries),
                        "vector": vector.tolist(),
                        "start_token": start,
                        "end_token": end,
                        "start_byte": start_byte,
                        "end_byte": end_byte,
                    }
                )
            results.append(
                {
                    "index": index,
                    "token_count": enc["token_count"],
                    "chunks": chunks,
                }
            )
            LOG.info(
                "embedded input %d of %d: %d tokens, %d chunks, peak %.2f GiB",
                index + 1,
                len(request.inputs),
                enc["token_count"],
                len(chunks),
                self._peak_gib,
            )
        return {
            "model": self.settings.model,
            "model_revision": self.settings.revision,
            "task": request.task,
            "dimension": DIMENSION,
            "results": results,
        }

    def tokens(self, text: str, task: str) -> dict[str, Any]:
        enc = self._encode(text, task, TOKENS_CAP, token_cap_exceeded)
        byte_at = char_to_byte_table(enc["prefixed"])
        doc_bytes = len(text.encode("utf-8"))
        offsets = enc["char_offsets"][enc["n_prefix"] :]
        rebased = []
        for a, b in offsets:
            a = min(max(a, enc["prefix_chars"]), len(byte_at) - 1)
            b = min(max(b, enc["prefix_chars"]), len(byte_at) - 1)
            lo = min(max(byte_at[a] - enc["prefix_bytes"], 0), doc_bytes)
            hi = min(max(byte_at[b] - enc["prefix_bytes"], 0), doc_bytes)
            rebased.append([lo, max(hi, lo)])
        return {
            "model": self.settings.model,
            "model_revision": self.settings.revision,
            "task": task,
            "dimension": DIMENSION,
            "token_count": enc["token_count"],
            "prefix_bytes": enc["prefix_bytes"],
            "offsets": rebased,
            "hidden_states": enc["hidden"].astype(np.float32).tolist(),
        }

    def info(self) -> dict[str, Any]:
        return {
            "model": self.settings.model,
            "model_revision": self.settings.revision,
            "dimension": DIMENSION,
            "max_tokens": self.settings.max_tokens,
            "tasks": list(TASKS),
            "device": self.settings.device,
            "loaded": self.loaded,
        }


# ---------------------------------------------------------------------------
# The HTTP layer
# ---------------------------------------------------------------------------


class Handler(http.server.BaseHTTPRequestHandler):
    """One request at a time, and one request per connection.

    HTTP/1.1 with an explicit `Connection: close`. A single-threaded
    server that kept connections alive would let one idle client hold the
    only worker, and a socket setup costs nothing next to a forward pass.

    `Connection: close` alone does not close the hole it was written for.
    A client that connects and then sends nothing has not reached the point
    where the header applies: `StreamRequestHandler` is still blocked
    reading the request line, with the default timeout of None, and it
    holds the only worker until that client disconnects. The unit carries
    `Restart=no`, so an operator would have to notice and restart. The
    timeout below is what actually closes it, and
    `BaseHTTPRequestHandler` turns the resulting error into a closed
    connection.

    Ten seconds, which is generous for a local client writing a header and
    far short of the request timeout: a slow forward pass is slow after the
    body has arrived, and this deadline is only on the read.
    """

    protocol_version = "HTTP/1.1"
    server_version = "yeomna-embedder/1"
    timeout = 10
    sys_version = ""

    def address_string(self) -> str:
        # BaseHTTPRequestHandler reads client_address[0], which an
        # AF_UNIX peer does not have.
        return "unix"

    def log_message(self, fmt: str, *args: Any) -> None:
        LOG.info("%s", fmt % args)

    def log_error(self, fmt: str, *args: Any) -> None:
        LOG.warning("%s", fmt % args)

    # -- plumbing ---------------------------------------------------------

    def _send(self, status: int, payload: dict[str, Any]) -> None:
        body = json.dumps(payload, allow_nan=False).encode("utf-8")
        self.close_connection = True
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Connection", "close")
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(body)

    def _fail(self, error: ServiceError) -> None:
        LOG.info("refused with %s (%d)", error.code, error.status)
        self._send(error.status, error.envelope())

    def _read_body(self) -> Any:
        if self.headers.get("Transfer-Encoding", "").lower() == "chunked":
            raise invalid_request(
                "chunked transfer encoding is not accepted. Send Content-Length"
            )
        raw = self.headers.get("Content-Length")
        if raw is None:
            raise invalid_request("Content-Length is required")
        try:
            length = int(raw)
        except ValueError:
            raise invalid_request("Content-Length is not an integer") from None
        if length < 0:
            raise invalid_request("Content-Length is negative")
        if length > MAX_BODY_BYTES:
            raise invalid_request(
                f"the request body is {length} bytes, above the "
                f"{MAX_BODY_BYTES} byte limit"
            )
        data = self.rfile.read(length) if length else b""
        if len(data) != length:
            raise invalid_request(
                f"the request body ended after {len(data)} of {length} bytes"
            )
        try:
            return json.loads(data.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError):
            # The body is request content. Its shape is not, and its
            # bytes do not go in the message.
            raise invalid_request("the request body is not valid UTF-8 JSON") from None

    def _route(self, handler: Any) -> None:
        try:
            handler()
        except ServiceError as error:
            self._fail(error)
        except Exception as exc:  # noqa: BLE001 - the envelope is the surface
            # An unexpected failure names its type and nothing from the
            # request. The traceback goes to the log, which this process
            # keeps, and the string of the exception does not, because an
            # exception raised while parsing can carry the input in it.
            LOG.exception("unhandled failure serving %s", self.path)
            self._fail(
                internal(
                    f"the request failed inside the service ({type(exc).__name__}). "
                    "See the service log"
                )
            )

    # -- the operations ---------------------------------------------------

    def do_GET(self) -> None:  # noqa: N802 - BaseHTTPRequestHandler's spelling
        def run() -> None:
            path = self.path.split("?", 1)[0]
            if path != "/v1/info":
                raise self._wrong_route(path, "GET")
            self._send(200, self.server.model.info())  # type: ignore[attr-defined]

        self._route(run)

    def do_HEAD(self) -> None:  # noqa: N802
        # Answered rather than left to the base class, which would reply
        # with its own HTML error page and no envelope. `_send` already
        # omits the body for HEAD.
        self.do_GET()

    def do_PUT(self) -> None:  # noqa: N802
        self._refuse_method()

    def do_DELETE(self) -> None:  # noqa: N802
        self._refuse_method()

    def do_PATCH(self) -> None:  # noqa: N802
        self._refuse_method()

    def do_OPTIONS(self) -> None:  # noqa: N802
        self._refuse_method()

    def _refuse_method(self) -> None:
        # Without these the base class answers 501 with an HTML page and no
        # envelope, which is the one way out of this service that is not the
        # contract's shape.
        path = self.path.split("?", 1)[0]
        self._route(lambda: (_ for _ in ()).throw(self._wrong_route(path, self.command)))

    def do_POST(self) -> None:  # noqa: N802
        def run() -> None:
            path = self.path.split("?", 1)[0]
            model: Model = self.server.model  # type: ignore[attr-defined]
            if path == "/v1/embed":
                request = validate_embed_request(self._read_body())
                self._send(200, model.embed(request))
            elif path == "/v1/tokens":
                text, task = validate_tokens_request(self._read_body())
                self._send(200, model.tokens(text, task))
            else:
                raise self._wrong_route(path, "POST")

        self._route(run)

    # The operations, so a wrong method can be told from an unknown path.
    ROUTES = {
        "/v1/info": "GET",
        "/v1/embed": "POST",
        "/v1/tokens": "POST",
    }

    def _wrong_route(self, path: str, method: str) -> ServiceError:
        # The contract's closed code set names no code for a routing
        # mistake, so the envelope is reused with the status that names the
        # real condition: 405 when the path is an operation reached the
        # wrong way, 404 when it is not an operation at all. Every method
        # answers through here, so there is no request shape that leaves
        # this service without the envelope.
        wanted = self.ROUTES.get(path)
        if wanted is not None:
            return ServiceError(
                "invalid-request",
                405,
                f"{path} is a {wanted}, not a {method}",
            )
        return ServiceError(
            "invalid-request",
            404,
            f"{path!r} is not an operation. This service serves GET /v1/info, "
            "POST /v1/embed, and POST /v1/tokens",
        )


class UnixHTTPServer(socketserver.UnixStreamServer):
    """Bind the way `yeomna-daemon`'s `bind()` binds.

    Mode 0600 in a directory the appliance keeps at 0700, a stale socket
    file replaced, and a missing directory refused rather than created,
    because a service that creates its own socket directory can create it
    in the wrong place.
    """

    allow_reuse_address = False
    request_queue_size = 16

    def __init__(self, path: str, handler: type, model: Model) -> None:
        self.model = model
        self._path = Path(path)
        super().__init__(path, handler)

    def server_bind(self) -> None:
        parent = self._path.parent
        if not parent.is_dir():
            raise StartupRefusal(f"the socket directory {parent} does not exist")
        try:
            os.unlink(self._path)
            LOG.warning("replaced a stale socket at %s", self._path)
        except FileNotFoundError:
            pass
        self.socket.bind(str(self._path))
        os.chmod(self._path, 0o600)
        self.server_address = str(self._path)

    def server_close(self) -> None:
        super().server_close()
        try:
            os.unlink(self._path)
        except FileNotFoundError:
            pass

    def handle_error(self, request: Any, client_address: Any) -> None:
        # The default prints a traceback to stdout with a banner. This
        # keeps it in the log and says nothing about the request.
        LOG.exception("a connection failed")


# ---------------------------------------------------------------------------
# Startup
# ---------------------------------------------------------------------------


def seal_environment(weights: str) -> None:
    """Make the offline variables true before transformers is imported.

    EC-10. The unit carries `PrivateNetwork=yes`, so a code path reaching
    for the network has no namespace to reach through and fails with
    something unhelpful. These variables make it fail on the variable
    first, which produces a better error. The modeling file imports
    `requests` into the embedding path, so the possibility is real.
    """
    os.environ["HF_HOME"] = weights
    os.environ["HF_HUB_OFFLINE"] = "1"
    os.environ["TRANSFORMERS_OFFLINE"] = "1"
    os.environ.setdefault("HF_HUB_DISABLE_TELEMETRY", "1")
    # Load-bearing, and measured. The default caching allocator fragments
    # across requests of different sizes, and on GPU 2 that is the
    # difference between a 16,384-token pass fitting and not: with the
    # default allocator a fresh process refused 14,500 tokens after a
    # ladder of smaller ones, and with this one it took 16,380 at a peak
    # of 14.62 GiB. Set here rather than only in the unit, because the
    # ceiling in /v1/info is a promise the service keeps by itself.
    # setdefault, so an operator debugging an allocation can override it.
    os.environ.setdefault("PYTORCH_CUDA_ALLOC_CONF", "expandable_segments:True")


def build(settings: Settings) -> UnixHTTPServer:
    weights = resolve_weights(settings.model, settings.revision, settings.weights)
    seal_environment(settings.weights)
    model = Model(settings, weights)
    server = UnixHTTPServer(settings.socket_path, Handler, model)
    model.start()
    return server


def main() -> int:
    logging.basicConfig(
        level=os.environ.get("YEOMNA_EMBEDDER_LOG", "INFO").upper(),
        format="%(asctime)s %(levelname)s %(name)s %(message)s",
        stream=sys.stderr,
    )
    try:
        settings = Settings()
        server = build(settings)
    except StartupRefusal as refusal:
        LOG.error("refusing to start: %s", refusal)
        return 2
    LOG.info(
        "listening on %s, max_tokens %d, device %s",
        settings.socket_path,
        settings.max_tokens,
        settings.device,
    )

    def stop(signum: int, _frame: Any) -> None:
        LOG.info("signal %d: shutting down", signum)
        threading.Thread(target=server.shutdown, daemon=True).start()

    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    try:
        server.serve_forever(poll_interval=0.2)
    finally:
        server.server_close()
        LOG.info("stopped")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
