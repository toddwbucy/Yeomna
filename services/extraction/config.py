"""Extraction service configuration."""

from __future__ import annotations

import os
from dataclasses import dataclass


@dataclass
class ExtractionConfig:
    """Configuration for the extraction service."""

    socket_path: str = "/run/yeomna/extractor.sock"
    use_ocr: bool = False
    idle_timeout_seconds: float = 900.0  # 15 min — unload VLM after idle
    use_fallback: bool = True  # PyMuPDF fallback if docling fails

    @classmethod
    def from_env(cls) -> ExtractionConfig:
        def parse_bool(name: str, default: bool) -> bool:
            raw = os.environ.get(name)
            if raw is None:
                return default
            return raw.lower() in ("1", "true", "yes")

        def parse_timeout(name: str, default: float) -> float:
            raw = os.environ.get(name)
            if raw is None:
                return default
            try:
                value = float(raw)
            except ValueError:
                return default
            return value if value >= 0 else default

        return cls(
            socket_path=os.environ.get("YEOMNA_EXTRACTOR_SOCKET", cls.socket_path),
            use_ocr=parse_bool("YEOMNA_EXTRACTOR_OCR", cls.use_ocr),
            idle_timeout_seconds=parse_timeout(
                "YEOMNA_EXTRACTOR_IDLE_TIMEOUT", cls.idle_timeout_seconds
            ),
            use_fallback=parse_bool("YEOMNA_EXTRACTOR_FALLBACK", cls.use_fallback),
        )
