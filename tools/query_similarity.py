"""Canonical query normalization and lexical similarity primitives."""
from __future__ import annotations

import re
import unicodedata


def normalize_query(value: str) -> str:
    return " ".join(unicodedata.normalize("NFC", value).casefold().strip().split())


def tokens(value: str) -> frozenset[str]:
    return frozenset(re.findall(r"[\w]+", normalize_query(value), flags=re.UNICODE))


def jaccard(left: frozenset[str], right: frozenset[str]) -> float:
    union = left | right
    return len(left & right) / len(union) if union else 1.0
