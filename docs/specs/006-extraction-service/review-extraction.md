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
