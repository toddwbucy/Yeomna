"""Tests for the Yeomna embedding service.

Everything but the last section runs on a machine with no GPU and with no
weights. The windowing rule, the pooling, the character-to-byte
conversion, the prefix rebase, the error envelope, the socket behavior,
and the not-loaded refusals are all reachable without touching the card.

The GPU-gated section skips with a named reason so a run on a machine
with Postgres up and no card says which thing was missing.
"""

from __future__ import annotations

import http.client
import json
import math
import os
import socket
import stat
import threading
from pathlib import Path

import numpy as np
import pytest

import server
from server import (
    DEFAULT_REVISION,
    DIMENSION,
    TASKS,
    TOKENS_CAP,
    Model,
    ServiceError,
    Settings,
    StartupRefusal,
    UnixHTTPServer,
    char_to_byte_table,
    chunk_byte_span,
    count_prefix_tokens,
    mean_pool_and_normalize,
    resolve_weights,
    validate_embed_request,
    validate_tokens_request,
    window_boundaries,
)

# Offsets recorded from the pinned snapshot's own Qwen2TokenizerFast, so
# the offset arithmetic is tested against what the tokenizer really says
# rather than against a guess. Reproduce with:
#
#   tok(f"Passage: {doc}", return_offsets_mapping=True)["offset_mapping"]
#
# The tokenizer adds no special tokens, and the prefix's trailing space
# attaches to the first content token when that token starts with one.
ASCII_DOC = "hello world"
ASCII_OFFSETS = [(0, 4), (4, 7), (7, 8), (8, 14), (14, 20)]

# Four CJK characters (twelve UTF-8 bytes), a space, an emoji (four
# bytes), a space, then ASCII. Nine characters of prefix plus this is
# where character offsets and byte offsets diverge. Written as escapes so
# the file stays ASCII, per the repo's editorial rules.
WIDE_DOC = "\u4f60\u597d\u4e16\u754c \U0001f600 tail"
WIDE_OFFSETS = [
    (0, 4),
    (4, 7),
    (7, 8),
    (8, 9),
    (9, 11),
    (11, 13),
    (13, 15),
    (15, 20),
]

PASSAGE_PREFIX = "Passage: "
PASSAGE_CHARS = len(PASSAGE_PREFIX)
PASSAGE_BYTES = len(PASSAGE_PREFIX.encode("utf-8"))


# ---------------------------------------------------------------------------
# The windowing rule, against late.rs
# ---------------------------------------------------------------------------


def test_windowing_empty():
    assert window_boundaries(0, 500, 200) == []


def test_windowing_single_window_when_size_exceeds_tokens():
    # late.rs test_late_chunk_single_chunk: 3 tokens, size 10, overlap 2.
    assert window_boundaries(3, 10, 2) == [(0, 3)]


def test_windowing_multiple_windows():
    # late.rs test_late_chunk_multiple_chunks: 6 tokens, size 3, overlap 1.
    assert window_boundaries(6, 3, 1) == [(0, 3), (2, 5), (4, 6)]


def test_windowing_reproduces_the_spikes_measured_count():
    # The spike embedded the charter at 8,631 tokens and got 29 chunks at
    # the default 500 by 200. A different count here is a drift from
    # late.rs, which is the whole reason the number is written down.
    boundaries = window_boundaries(8631, 500, 200)
    assert len(boundaries) == 29
    assert boundaries[0] == (0, 500)
    assert boundaries[-1][1] == 8631


def test_windowing_tiles_with_no_gap():
    for n in (1, 2, 7, 499, 500, 501, 1000, 8631):
        boundaries = window_boundaries(n, 500, 200)
        assert boundaries[0][0] == 0
        assert boundaries[-1][1] == n
        for (a, b), (c, _d) in zip(boundaries, boundaries[1:]):
            # The next window starts inside this one, so nothing is
            # skipped between them.
            assert a < c <= b


def test_windowing_terminates_when_overlap_reaches_the_size():
    # EC-3. The floor of 1 on the step is what makes this terminate
    # rather than loop forever.
    assert window_boundaries(4, 3, 3) == [(0, 3), (1, 4)]
    assert window_boundaries(4, 3, 9) == [(0, 3), (1, 4)]
    assert window_boundaries(3, 1, 1) == [(0, 1), (1, 2), (2, 3)]


def test_windowing_last_window_clamps_to_the_token_count():
    # EC-5. A document ending mid-window has end_token == token_count.
    boundaries = window_boundaries(7, 3, 1)
    assert boundaries == [(0, 3), (2, 5), (4, 7)]
    assert boundaries[-1][1] == 7
    # A step that would overshoot is never taken, because iteration stops
    # at the first window whose end reaches n.
    assert window_boundaries(8, 3, 1) == [(0, 3), (2, 5), (4, 7), (6, 8)]


# ---------------------------------------------------------------------------
# Pooling, against late.rs's mean_pool_and_normalize
# ---------------------------------------------------------------------------


def test_pooling_matches_late_rs_worked_example():
    # late.rs test_mean_pool_and_normalize: mean of [3,0] and [0,4] is
    # [1.5, 2.0], norm 2.5, normalized [0.6, 0.8].
    out = mean_pool_and_normalize([[3.0, 0.0], [0.0, 4.0]])
    assert out[0] == pytest.approx(0.6, abs=1e-5)
    assert out[1] == pytest.approx(0.8, abs=1e-5)


def test_pooling_one_hot_rows_normalize_to_the_diagonal():
    # late.rs test_late_chunk_single_chunk's expectation.
    out = mean_pool_and_normalize(np.eye(3, dtype=np.float32))
    expected = 1.0 / math.sqrt(3.0)
    for value in out:
        assert value == pytest.approx(expected, abs=1e-5)


def test_pooling_is_unit_norm():
    rng = np.random.default_rng(7)
    rows = rng.normal(size=(11, 64)).astype(np.float32)
    out = mean_pool_and_normalize(rows)
    assert float(np.linalg.norm(out)) == pytest.approx(1.0, abs=1e-5)


def test_pooling_zero_vector_stays_zero():
    out = mean_pool_and_normalize(np.zeros((4, 8), dtype=np.float32))
    assert not np.any(out)
    assert not np.any(np.isnan(out))


def test_pooling_tiny_norm_still_normalizes():
    # late.rs test_l2_normalize_tiny_norm.
    out = mean_pool_and_normalize([[3.0e-20, 4.0e-20]])
    assert float(np.linalg.norm(out)) == pytest.approx(1.0, abs=1e-5)
    assert out[0] == pytest.approx(0.6, abs=1e-5)


def test_pooling_infinite_norm_leaves_the_vector_untouched():
    # late.rs test_l2_normalize_infinite_norm.
    big = np.float32(np.finfo(np.float32).max)
    out = mean_pool_and_normalize([[big, big]])
    assert out[0] == big and out[1] == big


def test_pooling_empty_window_is_empty():
    assert mean_pool_and_normalize(np.zeros((0, 4), dtype=np.float32)).size == 0


# ---------------------------------------------------------------------------
# Character offsets are not byte offsets
# ---------------------------------------------------------------------------


def test_char_to_byte_table_on_ascii_is_the_identity():
    table = char_to_byte_table("abc")
    assert table == [0, 1, 2, 3]


def test_char_to_byte_table_counts_utf8_bytes():
    # Four CJK characters are twelve bytes, and an emoji is four.
    table = char_to_byte_table("\u4f60\u597d\U0001f600")
    assert table == [0, 3, 6, 10]


def test_char_to_byte_table_last_entry_is_the_byte_length():
    for text in ("", "a", WIDE_DOC, PASSAGE_PREFIX + WIDE_DOC):
        table = char_to_byte_table(text)
        assert table[-1] == len(text.encode("utf-8"))


def test_count_prefix_tokens_drops_only_whole_prefix_tokens():
    # The fourth ASCII token is " hello", spanning characters 8 to 14
    # across a nine-character prefix. It straddles, so it is content.
    assert count_prefix_tokens(ASCII_OFFSETS, PASSAGE_CHARS) == 3
    # The wide document's fourth token is the prefix's trailing space
    # alone, which is prefix and nothing else.
    assert count_prefix_tokens(WIDE_OFFSETS, PASSAGE_CHARS) == 4


def _span(doc: str, offsets, start: int, end: int, n_prefix: int):
    prefixed = PASSAGE_PREFIX + doc
    return chunk_byte_span(
        offsets,
        start + n_prefix,
        end + n_prefix,
        PASSAGE_CHARS,
        PASSAGE_BYTES,
        char_to_byte_table(prefixed),
        len(doc.encode("utf-8")),
    )


def test_byte_span_on_ascii_covers_the_whole_document():
    assert _span(ASCII_DOC, ASCII_OFFSETS, 0, 2, 3) == (0, 11)
    raw = ASCII_DOC.encode("utf-8")
    assert raw[0:11].decode("utf-8") == ASCII_DOC


def test_byte_span_on_multibyte_text_slices_back_to_the_callers_text():
    # EC-4. Character offsets would give (9, 13) and (13, 20) here, which
    # slice into the middle of a codepoint. Byte offsets give (0, 12) and
    # (12, 22), and both halves decode.
    raw = WIDE_DOC.encode("utf-8")
    first = _span(WIDE_DOC, WIDE_OFFSETS, 0, 2, 4)
    second = _span(WIDE_DOC, WIDE_OFFSETS, 2, 4, 4)
    assert first == (0, 12)
    assert second == (12, 22)
    assert raw[first[0] : first[1]].decode("utf-8") == "\u4f60\u597d\u4e16\u754c"
    assert raw[second[0] : second[1]].decode("utf-8") == " \U0001f600 tail"
    # The two spans tile the document with no gap and no overlap.
    assert first[1] == second[0]
    assert second[1] == len(raw)


def test_byte_span_never_runs_past_the_document():
    whole = _span(WIDE_DOC, WIDE_OFFSETS, 0, 4, 4)
    assert whole == (0, len(WIDE_DOC.encode("utf-8")))
    assert WIDE_DOC.encode("utf-8")[whole[0] : whole[1]].decode("utf-8") == WIDE_DOC


def test_byte_span_of_a_window_with_only_empty_offsets():
    assert _span("abc", [(0, 0), (0, 0)], 0, 2, 0) == (0, 0)


# ---------------------------------------------------------------------------
# Request validation and the error envelope
# ---------------------------------------------------------------------------


def _refusal(body, op=validate_embed_request) -> ServiceError:
    with pytest.raises(ServiceError) as caught:
        op(body)
    return caught.value


def test_embed_defaults_are_500_and_200():
    request = validate_embed_request({"input": "x", "task": "retrieval.passage"})
    assert request.inputs == ["x"]
    assert request.chunk_size_tokens == 500
    assert request.chunk_overlap_tokens == 200


def test_embed_accepts_an_array_of_strings():
    request = validate_embed_request(
        {"input": ["a", "b"], "task": "code", "chunk_size_tokens": 8}
    )
    assert request.inputs == ["a", "b"]
    assert request.task == "code"
    assert request.chunk_size_tokens == 8


def test_embed_body_must_be_an_object():
    error = _refusal(["not", "an", "object"])
    assert (error.code, error.status) == ("invalid-request", 400)


def test_embed_missing_input_is_invalid_request():
    error = _refusal({"task": "retrieval.passage"})
    assert (error.code, error.status) == ("invalid-request", 400)
    assert "input" in error.message


def test_embed_missing_task_is_invalid_request():
    error = _refusal({"input": "x"})
    assert (error.code, error.status) == ("invalid-request", 400)
    assert "task" in error.message


def test_embed_empty_string_is_invalid_request_not_a_zero_vector():
    # EC-1.
    error = _refusal({"input": "", "task": "retrieval.passage"})
    assert (error.code, error.status) == ("invalid-request", 400)
    error = _refusal({"input": ["ok", ""], "task": "retrieval.passage"})
    assert error.code == "invalid-request"
    assert "index 1" in error.message


def test_embed_empty_array_is_invalid_request():
    assert _refusal({"input": [], "task": "code"}).code == "invalid-request"


def test_embed_non_string_element_is_invalid_request():
    assert _refusal({"input": ["a", 3], "task": "code"}).code == "invalid-request"


def test_embed_unknown_task_names_what_is_served():
    error = _refusal({"input": "x", "task": "retrieval"})
    assert (error.code, error.status) == ("unknown-task", 400)
    assert "retrieval.passage" in error.message


def test_embed_images_are_refused_not_ignored():
    error = _refusal({"input": "x", "task": "code", "images": []})
    assert (error.code, error.status) == ("images-unsupported", 400)
    # Even a null value counts as asking, because the field was written.
    assert _refusal({"input": "x", "task": "code", "images": None}).code == (
        "images-unsupported"
    )


def test_embed_two_tasks_in_one_request_is_a_mixed_batch():
    # EC-11. `task` is scalar, so a list is the only way to name two.
    error = _refusal({"input": ["a", "b"], "task": ["code", "retrieval.query"]})
    assert (error.code, error.status) == ("mixed-task-batch", 400)


def test_embed_a_one_element_task_list_is_not_mixed():
    request = validate_embed_request({"input": "x", "task": ["code"]})
    assert request.task == "code"


def test_embed_chunk_fields_must_be_sane_integers():
    assert _refusal({"input": "x", "task": "code", "chunk_size_tokens": 0}).code == (
        "invalid-request"
    )
    assert _refusal(
        {"input": "x", "task": "code", "chunk_size_tokens": "500"}
    ).code == "invalid-request"
    assert _refusal(
        {"input": "x", "task": "code", "chunk_overlap_tokens": -1}
    ).code == "invalid-request"
    # A bool is an int in Python and is not an integer here.
    assert _refusal(
        {"input": "x", "task": "code", "chunk_size_tokens": True}
    ).code == "invalid-request"


def test_tokens_takes_one_input():
    text, task = validate_tokens_request({"input": "x", "task": "retrieval.passage"})
    assert (text, task) == ("x", "retrieval.passage")
    assert _refusal(
        {"input": ["a", "b"], "task": "code"}, validate_tokens_request
    ).code == "invalid-request"


def test_error_envelope_shape_is_the_contracts():
    error = ServiceError("input-too-large", 400, "18204 tokens exceeds max_tokens 16384")
    assert error.envelope() == {
        "error": {
            "code": "input-too-large",
            "message": "18204 tokens exceeds max_tokens 16384",
        }
    }


def test_every_contract_code_maps_to_its_documented_status():
    expected = {
        "invalid-request": 400,
        "unknown-task": 400,
        "images-unsupported": 400,
        "mixed-task-batch": 400,
        "input-too-large": 400,
        "token-cap-exceeded": 400,
        "model-not-loaded": 503,
        "out-of-memory": 503,
        "internal": 500,
    }
    built = [
        server.invalid_request("m"),
        server.unknown_task("nope"),
        server.images_unsupported(),
        server.mixed_task_batch(),
        server.input_too_large(18204, 16384),
        server.token_cap_exceeded(900),
        server.model_not_loaded("loading"),
        server.out_of_memory(),
        server.internal("m"),
    ]
    assert {e.code: e.status for e in built} == expected
    assert server.input_too_large(18204, 16384).message == (
        "18204 tokens exceeds max_tokens 16384"
    )
    assert str(TOKENS_CAP) in server.token_cap_exceeded(900).message


# ---------------------------------------------------------------------------
# Settings and the refusal to fetch weights
# ---------------------------------------------------------------------------


def test_settings_defaults_are_the_contracts(tmp_path):
    settings = Settings({})
    assert settings.model == "jinaai/jina-embeddings-v4"
    assert settings.revision == DEFAULT_REVISION
    assert settings.device == "cuda:2"
    assert settings.max_tokens == 16384


def test_settings_read_the_environment_without_touching_the_process():
    settings = Settings(
        {
            "YEOMNA_EMBEDDER_MODEL": "other/model",
            "YEOMNA_EMBEDDER_REVISION": "deadbeef",
            "YEOMNA_EMBEDDER_DEVICE": "cpu",
            "YEOMNA_EMBEDDER_SOCKET": "/tmp/x.sock",
            "YEOMNA_EMBEDDER_WEIGHTS": "/weights",
            "YEOMNA_EMBEDDER_MAX_TOKENS": "2048",
        }
    )
    assert settings.model == "other/model"
    assert settings.revision == "deadbeef"
    assert settings.device == "cpu"
    assert settings.socket_path == "/tmp/x.sock"
    assert settings.weights == "/weights"
    assert settings.max_tokens == 2048


def test_settings_refuse_a_ceiling_the_processor_would_truncate():
    with pytest.raises(StartupRefusal):
        Settings({"YEOMNA_EMBEDDER_MAX_TOKENS": "40000"})
    with pytest.raises(StartupRefusal):
        Settings({"YEOMNA_EMBEDDER_MAX_TOKENS": "0"})
    with pytest.raises(StartupRefusal):
        Settings({"YEOMNA_EMBEDDER_MAX_TOKENS": "many"})


def test_missing_weights_refuse_to_start_rather_than_download(tmp_path):
    with pytest.raises(StartupRefusal) as caught:
        resolve_weights("jinaai/jina-embeddings-v4", DEFAULT_REVISION, str(tmp_path))
    assert "never fetches weights" in str(caught.value)


def test_an_incomplete_snapshot_refuses_to_start(tmp_path):
    snapshot = tmp_path / "hub" / "models--a--b" / "snapshots" / "rev"
    snapshot.mkdir(parents=True)
    with pytest.raises(StartupRefusal) as caught:
        resolve_weights("a/b", "rev", str(tmp_path))
    assert "config.json" in str(caught.value)
    (snapshot / "config.json").write_text("{}")
    with pytest.raises(StartupRefusal) as caught:
        resolve_weights("a/b", "rev", str(tmp_path))
    assert "adapters/adapter_config.json" in str(caught.value)


def test_resolve_weights_finds_the_pinned_snapshot_when_it_is_present():
    home = os.environ.get("YEOMNA_EMBEDDER_WEIGHTS", server.DEFAULT_WEIGHTS)
    try:
        path = resolve_weights("jinaai/jina-embeddings-v4", DEFAULT_REVISION, home)
    except StartupRefusal as refusal:
        pytest.skip(f"the pinned snapshot is not under {home}: {refusal}")
    assert path.name == DEFAULT_REVISION


# ---------------------------------------------------------------------------
# The socket, and the refusals before the weights are ready
# ---------------------------------------------------------------------------


def _settings_for(path: Path) -> Settings:
    return Settings(
        {
            "YEOMNA_EMBEDDER_SOCKET": str(path),
            "YEOMNA_EMBEDDER_DEVICE": "cpu",
        }
    )


class _Served:
    """A bound server on a background thread, with the weights unloaded.

    `Model.start` is never called, so the state stays "loading" and the
    HTTP layer can be exercised with no card and no weights.
    """

    def __init__(self, path: Path) -> None:
        settings = _settings_for(path)
        self.path = path
        self.model = Model(settings, Path("/nonexistent"))
        self.server = UnixHTTPServer(str(path), server.Handler, self.model)
        self.thread = threading.Thread(
            target=self.server.serve_forever, kwargs={"poll_interval": 0.02}
        )
        self.thread.start()

    def request(self, method: str, target: str, body=None):
        conn = http.client.HTTPConnection("localhost")
        conn.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        conn.sock.connect(str(self.path))
        payload = None if body is None else json.dumps(body).encode("utf-8")
        headers = {"Content-Type": "application/json"} if payload else {}
        conn.request(method, target, body=payload, headers=headers)
        response = conn.getresponse()
        raw = response.read()
        conn.close()
        return response.status, json.loads(raw) if raw else None

    def close(self) -> None:
        self.server.shutdown()
        self.thread.join(timeout=5)
        self.server.server_close()


@pytest.fixture
def served(tmp_path):
    instance = _Served(tmp_path / "embedder.sock")
    try:
        yield instance
    finally:
        instance.close()


def test_the_socket_is_mode_0600(served):
    mode = stat.S_IMODE(os.stat(served.path).st_mode)
    assert mode == 0o600


def test_a_missing_socket_directory_is_a_refusal_to_start(tmp_path):
    missing = tmp_path / "absent" / "embedder.sock"
    settings = _settings_for(missing)
    with pytest.raises(StartupRefusal) as caught:
        UnixHTTPServer(str(missing), server.Handler, Model(settings, Path("/x")))
    assert "does not exist" in str(caught.value)
    assert not missing.parent.exists()


def test_a_stale_socket_file_is_replaced_on_bind(tmp_path):
    # EC-7. A killed service leaves the file behind and the next start
    # must take the address rather than refuse it.
    path = tmp_path / "embedder.sock"
    path.write_bytes(b"stale")
    instance = _Served(path)
    try:
        assert stat.S_ISSOCK(os.stat(path).st_mode)
        status, _ = instance.request("GET", "/v1/info")
        assert status == 200
    finally:
        instance.close()
    assert not path.exists()


def test_info_answers_immediately_with_loaded_false(served):
    # EC-6 and FR1.
    status, body = served.request("GET", "/v1/info")
    assert status == 200
    assert body == {
        "model": "jinaai/jina-embeddings-v4",
        "model_revision": DEFAULT_REVISION,
        "dimension": DIMENSION,
        "max_tokens": 16384,
        "tasks": ["retrieval.passage", "retrieval.query", "text-matching", "code"],
        "device": "cpu",
        "loaded": False,
    }


def test_embed_refuses_with_model_not_loaded_rather_than_blocking(served):
    # EC-6. Retryable, and it does not wait on the load.
    status, body = served.request(
        "POST", "/v1/embed", {"input": "x", "task": "retrieval.passage"}
    )
    assert status == 503
    assert body["error"]["code"] == "model-not-loaded"


def test_tokens_refuses_with_model_not_loaded(served):
    status, body = served.request(
        "POST", "/v1/tokens", {"input": "x", "task": "retrieval.passage"}
    )
    assert status == 503
    assert body["error"]["code"] == "model-not-loaded"


def test_validation_runs_before_the_not_loaded_refusal(served):
    # A malformed request is malformed whether or not the weights are up,
    # and answering 400 without the model is what lets these tests exist.
    status, body = served.request("POST", "/v1/embed", {"task": "retrieval.passage"})
    assert status == 400
    assert body["error"]["code"] == "invalid-request"
    status, body = served.request("POST", "/v1/embed", {"input": "x", "task": "nope"})
    assert status == 400
    assert body["error"]["code"] == "unknown-task"
    status, body = served.request(
        "POST", "/v1/embed", {"input": "x", "task": "code", "images": [1]}
    )
    assert status == 400
    assert body["error"]["code"] == "images-unsupported"


def test_a_body_that_is_not_json_is_invalid_request(served):
    conn = http.client.HTTPConnection("localhost")
    conn.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    conn.sock.connect(str(served.path))
    conn.request("POST", "/v1/embed", body=b"{not json", headers={})
    response = conn.getresponse()
    body = json.loads(response.read())
    conn.close()
    assert response.status == 400
    assert body["error"]["code"] == "invalid-request"
    # The message says nothing about what the body contained.
    assert "not json" not in body["error"]["message"]


def test_an_unknown_path_is_not_an_operation(served):
    status, body = served.request("GET", "/v2/info")
    assert status == 404
    assert body["error"]["code"] == "invalid-request"
    status, body = served.request("POST", "/v1/info", {})
    assert status == 405
    assert body["error"]["code"] == "invalid-request"


def test_a_known_path_reached_the_wrong_way_is_405_not_404(served):
    """A routing mistake is distinguishable from an unknown path.

    The first cut sent every non-info GET to the 404 branch, so
    `GET /v1/embed` reported that `/v1/embed` is not an operation. It is,
    and telling a caller otherwise sends them looking for a typo they did
    not make.
    """
    for method, path, wanted in [
        ("GET", "/v1/embed", "POST"),
        ("GET", "/v1/tokens", "POST"),
        ("POST", "/v1/info", "GET"),
    ]:
        status, body = served.request(method, path, {} if method == "POST" else None)
        assert status == 405, f"{method} {path}"
        assert body["error"]["code"] == "invalid-request"
        assert wanted in body["error"]["message"], body["error"]["message"]


def test_every_method_answers_with_the_envelope(served):
    """No request shape leaves this service without the contract's envelope.

    `BaseHTTPRequestHandler` answers an unimplemented method with 501 and
    its own HTML error page, which is the one way out of here that is not
    the contract's shape. Each method is handled so that cannot happen.
    """
    for method in ["PUT", "DELETE", "PATCH", "OPTIONS"]:
        status, body = served.request(method, "/v1/embed")
        assert status == 405, f"{method}: {status}"
        assert body["error"]["code"] == "invalid-request", method
        assert "POST" in body["error"]["message"], method


def test_the_task_table_is_the_contracts_four_names():
    assert list(TASKS) == [
        "retrieval.passage",
        "retrieval.query",
        "text-matching",
        "code",
    ]
    assert TASKS["retrieval.passage"] == ("retrieval", "Passage")
    assert TASKS["retrieval.query"] == ("retrieval", "Query")
    # The snapshot's _validate_encoding_params forces the Query prefix for
    # text-matching whatever prompt_name says.
    assert TASKS["text-matching"] == ("text-matching", "Query")
    assert TASKS["code"] == ("code", "Query")


# ---------------------------------------------------------------------------
# GPU-gated. These load the weights onto the card.
# ---------------------------------------------------------------------------


def _gpu_reason() -> str | None:
    if os.environ.get("YEOMNA_EMBEDDER_GPU_TESTS") != "1":
        return "YEOMNA_EMBEDDER_GPU_TESTS is not 1, so the weights are not loaded"
    home = os.environ.get("YEOMNA_EMBEDDER_WEIGHTS", server.DEFAULT_WEIGHTS)
    try:
        resolve_weights("jinaai/jina-embeddings-v4", DEFAULT_REVISION, home)
    except StartupRefusal as refusal:
        return f"the pinned snapshot is absent: {refusal}"
    try:
        import torch
    except ImportError:
        return "torch is not installed"
    if not torch.cuda.is_available():
        return "torch reports no CUDA device"
    return None


gpu = pytest.mark.skipif(_gpu_reason() is not None, reason=_gpu_reason() or "")


@pytest.fixture(scope="module")
def loaded_model():
    reason = _gpu_reason()
    if reason is not None:
        pytest.skip(reason)
    settings = Settings(dict(os.environ))
    weights = resolve_weights(settings.model, settings.revision, settings.weights)
    # This writes the process environment, which a test normally must not
    # do. It is correct here and only here. The function exists to set the
    # offline and allocator variables *before* transformers is imported, so
    # a test that loads the model and does not call it is not testing the
    # thing production runs. There is no race to create either: pytest runs
    # this module in one process, this fixture is module-scoped and runs
    # once, and nothing else here reads these variables.
    server.seal_environment(settings.weights)
    model = Model(settings, weights)
    model.start().join()
    if not model.loaded:
        pytest.fail(f"the weights did not load: {model.state}")
    return model


@gpu
def test_gpu_embed_returns_unit_vectors_of_the_right_dimension(loaded_model):
    request = validate_embed_request(
        {
            "input": "The appliance is sealed and the verb layer is the only surface.",
            "task": "retrieval.passage",
        }
    )
    body = loaded_model.embed(request)
    assert body["dimension"] == DIMENSION
    assert body["task"] == "retrieval.passage"
    assert body["model_revision"] == DEFAULT_REVISION
    assert len(body["results"]) == 1
    result = body["results"][0]
    assert result["index"] == 0
    assert result["chunks"], "the response is always chunked"
    for chunk in result["chunks"]:
        assert len(chunk["vector"]) == DIMENSION
        assert float(np.linalg.norm(chunk["vector"])) == pytest.approx(1.0, abs=1e-4)
        assert chunk["total_chunks"] == len(result["chunks"])
    assert result["chunks"][-1]["end_token"] == result["token_count"]


@gpu
def test_gpu_byte_spans_slice_multibyte_text_back(loaded_model):
    doc = "\u4f60\u597d\u4e16\u754c \U0001f600 " + "sealed appliance " * 40
    request = validate_embed_request(
        {
            "input": doc,
            "task": "retrieval.passage",
            "chunk_size_tokens": 20,
            "chunk_overlap_tokens": 5,
        }
    )
    result = loaded_model.embed(request)["results"][0]
    raw = doc.encode("utf-8")
    assert len(result["chunks"]) > 1
    for chunk in result["chunks"]:
        piece = raw[chunk["start_byte"] : chunk["end_byte"]]
        # Round-trips, so the span never cuts a codepoint in half.
        assert piece.decode("utf-8").encode("utf-8") == piece
    assert result["chunks"][0]["start_byte"] == 0
    assert result["chunks"][-1]["end_byte"] == len(raw)


@gpu
def test_gpu_the_pooling_identity_holds_against_the_token_view(loaded_model):
    # The oracle, in Python. The Rust test does the same through
    # late_chunk_embeddings and requires the same cosine.
    doc = "Basis is derived at ingest and written explicitly. " * 12
    tokens = loaded_model.tokens(doc, "retrieval.passage")
    assert tokens["token_count"] == len(tokens["hidden_states"])
    assert tokens["token_count"] == len(tokens["offsets"])
    assert tokens["prefix_bytes"] == len("Passage: ")
    hidden = np.asarray(tokens["hidden_states"], dtype=np.float32)

    request = validate_embed_request(
        {
            "input": doc,
            "task": "retrieval.passage",
            "chunk_size_tokens": 60,
            "chunk_overlap_tokens": 20,
        }
    )
    result = loaded_model.embed(request)["results"][0]
    assert result["token_count"] == tokens["token_count"]
    boundaries = window_boundaries(tokens["token_count"], 60, 20)
    assert len(boundaries) == len(result["chunks"])
    for (start, end), chunk in zip(boundaries, result["chunks"]):
        pooled = mean_pool_and_normalize(hidden[start:end])
        served_vector = np.asarray(chunk["vector"], dtype=np.float32)
        cosine = float(np.dot(pooled, served_vector))
        assert cosine >= 0.9999, f"chunk {chunk['chunk_index']} cosine {cosine}"


@gpu
def test_gpu_tokens_refuses_above_its_cap(loaded_model):
    with pytest.raises(ServiceError) as caught:
        loaded_model.tokens("sealed appliance " * 400, "retrieval.passage")
    assert caught.value.code == "token-cap-exceeded"
    assert caught.value.status == 400


@gpu
def test_gpu_an_input_above_the_ceiling_is_refused_not_truncated(loaded_model):
    settings = Settings({"YEOMNA_EMBEDDER_MAX_TOKENS": "64", "YEOMNA_EMBEDDER_DEVICE": loaded_model.settings.device})
    narrow = Model(settings, loaded_model.weights)
    # Borrow the loaded weights rather than loading a second copy.
    with loaded_model._lock:  # noqa: SLF001 - the test is inside the module
        narrow._torch = loaded_model._torch
        narrow._model = loaded_model._model
        narrow._tokenizer = loaded_model._tokenizer
        narrow._state = "ready"
        narrow._detail = "ready"
    with pytest.raises(ServiceError) as caught:
        narrow.embed(validate_embed_request({"input": "word " * 200, "task": "code"}))
    assert caught.value.code == "input-too-large"
    assert "max_tokens 64" in caught.value.message
