# Specification: 006 Extraction Service

Parent PRD: `docs/PRD-pipeline-libraries.md`, Phase 6 as collapsed by the
v0.2 severance ruling.
Status: draft, 2026-08-12.

Editorial rules: ASCII only, no em-dashes, no semicolons, never the words
genuinely, honestly, or actually. These govern prose. Code blocks keep the
syntax their language requires.

---

## Overview

The last living port: `services/extraction/`, the Docling document pipeline.
Python by design (Docling's VLM, the LaTeX parser, the PyMuPDF fallback are
ML and parsing stacks with no Rust replacement story), zero store coupling,
about 970 lines plus the server. After this merges, every port is done and
the severance closes HADES-Burn.

The embedding service is **not** ported, per the ruling: it is boilerplate
around a loader the SPU replaces, and its absence is a named hole. The
`services/tests/` directory is **not** ported: all three files test the
training service, which is excluded with the graph-embed cluster. The
extraction service therefore arrives with zero tests, recorded plainly.

## Task Scope

### This Task Will

1. Move `services/extraction/` (config, server, docling backend, latex
   backend, init) into `services/extraction/` here.
2. Regenerate the Python gRPC stubs from this repo's
   `proto/yeomna/extraction/extraction.proto` into `services/generated/`,
   closing the wire-rename loop: the server imports
   `yeomna.extraction`, and the deployed protocol matches the Rust client.
3. Apply the naming rulings as part of the move, since this port was always
   scheduled to carry them: `YEOMNA_EXTRACTOR_*` env vars,
   `/run/yeomna/extractor.sock` (where the Rust client already points),
   docstrings and logger names swept.
4. Carry a trimmed `services/pyproject.toml` (extraction dependencies only)
   and an adapted `services/Makefile` (stub generation against `../proto`,
   extraction targets only).
5. Carry the systemd unit and env-file templates renamed
   (`yeomna-extractor.service`, `extractor.conf`), as deployment templates,
   not enabled anything.
6. Best-effort verification, bounded and stated: stub generation succeeds,
   every file passes `py_compile`, `config.py` imports and resolves its
   defaults without Docling installed. Standing the service up on GPU 2 with
   the full Docling stack is the functionality-test step of the severance
   sequence, after the port.

### Out of Scope

- The embedding and training services, the Makefile targets for them, and
  their proto packages. Holes and exclusions per the ruling.
- Any change to extraction behavior, backend selection, or the VLM
  idle-unload logic.
- Installing the Docling dependency stack. Deployment work, later step.
- The config-file-versus-env question: the service keeps its env-var surface
  under the new names until that ruling lands.

## Naming applied at the move, per the standing rulings

| Old | New |
|---|---|
| `HADES_EXTRACTOR_SOCKET` and siblings | `YEOMNA_EXTRACTOR_*` |
| `/run/hades/extractor.sock` | `/run/yeomna/extractor.sock` |
| `from persephone.extraction import ...` | `from yeomna.extraction import ...` (regenerated) |
| Persephone/HADES docstrings, titles, loggers | Yeomna framing |
| `hades-extractor.service` | `yeomna-extractor.service` |

`cuda:2` stays: it is the assigned GPU, not a brand.

## Requirements

**FR-X1.** Backend behavior unchanged: Docling primary, LaTeX backend for
`.tex`, PyMuPDF fallback when Docling fails, VLM idle unload after the
configured timeout.

**FR-X2.** The regenerated stubs come from this repo's proto file and no
other, so the Python server and the Rust client share one wire definition by
construction.

**FR-X3.** The service binds a Unix socket only. No TCP surface, matching
the charter and the extractor's own history (it never compromised).

**FR-X4.** Zero tests is the honest arrival state, recorded in the review
notes, and the follow-up (service-level tests that mock the backends) joins
the holes ledger rather than being improvised here.

## Success Criteria

1. `python -m py_compile` clean across every moved file.
2. Stub generation from `proto/yeomna/extraction/` succeeds and the server
   imports resolve against the generated package (verified by import in an
   environment with grpcio only, backends stubbed out of the import path or
   the failure recorded if module-level imports prevent it).
3. `grep -ri 'hades|persephone'` clean across `services/` except provenance.
4. The Rust workspace gate is unaffected.
5. Review notes account for every file and every rename.

## QA Acceptance Criteria

1. Issue and draft PR per the standing workflow.
2. The PR body states what verification the environment allowed and what
   waits for the functionality-test step.
