# Review Notes: extraction service port

Reviewer: Claude, with Todd. Date: 2026-08-12. Every file was read at spec
time and the renames verified by grep at the move.

## Accounting

| Source (`services/extraction/`) | Destination | Lines | Diff class |
|---|---|---|---|
| `__init__.py` | same path here | 1 | Brand line reworded |
| `config.py` | same | 35 | Env vars to `YEOMNA_EXTRACTOR_*`, socket default to `/run/yeomna/extractor.sock` |
| `server.py` | same | 378 | Docstring, the stub import to `yeomna.extraction`, socket-group comment |
| `docling_backend.py` | same | 281 | Byte-identical |
| `latex_backend.py` | same | 278 | Byte-identical |

Ten rename edits total across three files, counted by the move script.

New files: `services/generated/yeomna/extraction/` (stubs regenerated from
this repo's proto, closing the wire-rename loop: the generated grpc module
imports `from yeomna.extraction import extraction_pb2`), a trimmed
`pyproject.toml` (extraction dependencies only, `pymupdf` added since
`docling_backend.py` imports `fitz` and the reference pyproject never
declared it, a seventh dependency correction), a trimmed `Makefile`
(proto-gen against `../proto`, extraction targets only), and the renamed
systemd templates with `SupplementaryGroups=weaver-admin arangodb` dropped,
the dead store group having no business in a Yeomna unit.

## Arrival state, recorded plainly

- **Zero tests.** The reference's `services/tests/` are all training-service
  tests, excluded with the graph-embed cluster. Service-level tests that
  mock the backends join the holes ledger.
- **Verification the environment allowed:** stub generation from our proto,
  `py_compile` clean across every file including generated, the stub package
  imports and resolves `SOURCE_TYPE_PDF`, and `ExtractionConfig` constructs
  with the renamed defaults (`/run/yeomna/extractor.sock`, `cuda:2`, 900s
  idle unload). Standing the service up with the Docling stack on GPU 2 is
  the functionality-test step of the severance sequence.
- The Rust workspace gate is unaffected.

## The port era closes with this PR

This is the last living port. What remains of the reference is excluded by
the severance ruling and named as holes.

## CodeRabbit round, triaged under the docling-rs signal

Todd flagged mid-review that the Docling backend may be replaced by
docling-rs now that the port era is closing, so the round was triaged by
what survives that decision.

**The important find, recorded as a hole rather than deep-fixed:**
`DocumentConverter.convert_single` is Docling v1 API, and the dependency
demands 2.x. The reference's primary extraction path most likely raised
AttributeError on every PDF and silently rode the PyMuPDF fallback, joining
the pattern of documented-but-never-wired features (the gRPC embedder, late
chunking). The related v2-shape findings (format routing narrower than the
advertised Capabilities, caption and page-count accessors) are part of the
same hole: **"extraction backend: repair against Docling v2 or replace with
docling-rs, decided at the functionality-test step."** A one-line
convert-single-to-convert stopgap exists if the Python path is kept.

**Applied, because they survive any backend:** defensive config parsing
(non-numeric timeout no longer crashes boot, negatives rejected, symmetric
boolean parsing), the unread `device` setting removed with the conf comment
naming CUDA_VISIBLE_DEVICES as the effective control, `table*` starred
environments matched, tar's explicit data filter, the tmpfiles config the
reference relied on but never shipped (with the RuntimeDirectory trap
documented), unit ordering after tmpfiles setup, and the docling 2.x upper
bound.

**Skipped with the fate reason:** the server-architecture findings (shared
single-concurrency executor for ML branches, async temp-file handling, idle
monitor accessors, gRPC error classification), the Makefile PID management,
pyproject packaging, and portable unit paths. Every one is real, and every
one is polish on a Python server that a docling-rs decision may replace
wholesale. They ride the same hole entry and get resolved with it.
