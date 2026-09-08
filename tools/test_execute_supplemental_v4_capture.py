"""Offline integrity tests for the frozen V4 capture executor.

These tests never touch the network and never write into the frozen V4 output
directory. They exercise the pure contract gate (V4Accumulator), the frozen
integrity preflight (precapture), canonicalization, exclusion-set loading, and
the A/B packet writer against a temporary directory.
"""
import json
import tempfile
from pathlib import Path

import execute_supplemental_v4_capture as exe


def _synthetic_universe():
    return {"universe_id": "synthetic", "queries": [
        {"query_id": "v4-9001", "query_text": "alpha query", "designation": "treatment",
         "capture_order": 1, "permitted_positions": [1, 2, 3], "strategy_class": "T1"},
        {"query_id": "v4-9002", "query_text": "beta query", "designation": "control",
         "capture_order": 2, "permitted_positions": [1, 2, 3], "strategy_class": "T1"},
    ]}


def _acc(exclusions=()):
    return exe.V4Accumulator(_synthetic_universe(), *[{n for n in exclusions} for _ in range(4)])


def test_canonical_normalization():
    assert exe.canonical("https://Example.COM/Path//") == "https://example.com/Path"
    assert exe.canonical("http://example.com") == "http://example.com/"
    assert exe.canonical("https://example.com/a?b=1&c=2") == "https://example.com/a?b=1&c=2"
    for bad in ("", "not a url", "ftp://example.com/x", "javascript:void(0)"):
        try:
            exe.canonical(bad)
        except ValueError:
            continue
        raise AssertionError("expected ValueError for %r" % bad)


_FULL_EXTRA = {"attempt_number": 1, "acquired_at": "2026-09-02T00:00:00Z",
               "raw_response_path": "x", "raw_response_sha256": "0" * 64}


def _full_extra(**overrides):
    extra = dict(_FULL_EXTRA)
    extra.update(overrides)
    return extra


def test_accumulator_accepts_valid_result():
    acc = _acc()
    row = acc.accept("v4-9001", 1, "Title", "Snippet", "https://fresh.example.com/a",
                     _full_extra())
    assert row is not None
    assert row["designation"] == "treatment"
    assert row["canonical_url"] == "https://fresh.example.com/a"
    # row_id must be non-arm-encoding and shared with packets; hypothesis_feature
    # is not part of the frozen V4 universe and must not appear in corpus rows.
    assert row["row_id"] == "sup-v4-q001-001"
    assert "t-" not in row["row_id"] and "c-" not in row["row_id"]
    assert "hypothesis_feature" not in row
    assert acc.ledger[-1]["reason_code"] == "ACCEPTED"


def test_accumulator_rejects_invalid_and_missing_fields():
    acc = _acc()
    assert acc.accept("v4-9001", 1, "T", "S", "not a url", _full_extra()) is None
    assert acc.accept("v4-9001", 1, "", "S", "https://x.example.com/1", _full_extra()) is None
    assert acc.accept("v4-9001", 1, "T", "", "https://x.example.com/1", _full_extra()) is None
    assert acc.rejected["INVALID_RESULT"] == 3


def test_accumulator_rejects_duplicate():
    acc = _acc()
    assert acc.accept("v4-9001", 1, "T", "S", "https://dup.example.com/", _full_extra()) is not None
    assert acc.accept("v4-9001", 2, "T", "S", "https://dup.example.com/", _full_extra()) is None
    assert acc.rejected["DUPLICATE_CURRENT"] == 1



def test_accumulator_rejects_overlap_sets():
    acc = _acc(exclusions={"https://known.example.com/"})
    assert acc.accept("v4-9001", 1, "T", "S", "https://known.example.com/",
                      {"attempt_number": 1}) is None
    reasons = [r["reason_code"] for r in acc.ledger]
    assert reasons and all(r in ("OVERLAP_V1", "OVERLAP_SUPPLEMENTAL", "OVERLAP_V3",
                                 "HISTORICAL_OVERLAP") for r in reasons)


def test_accumulator_outside_allowed_depth_and_unknown_query():
    acc = _acc()
    assert acc.accept("v4-9001", 4, "T", "S", "https://d.example.com/", _full_extra()) is None
    assert acc.accept("v4-9999", 1, "T", "S", "https://u.example.com/", _full_extra()) is None
    assert acc.rejected["OUTSIDE_ALLOWED_DEPTH"] == 1
    assert acc.rejected["OUTSIDE_FROZEN_UNIVERSE"] == 1


def test_accumulator_stops_at_per_arm_target():
    universe = {"universe_id": "synthetic", "queries": [
        {"query_id": "v4-9101", "query_text": "t", "designation": "treatment",
         "capture_order": 1, "permitted_positions": [1], "strategy_class": "T1"},
        {"query_id": "v4-9102", "query_text": "c", "designation": "control",
         "capture_order": 2, "permitted_positions": [1], "strategy_class": "T1"}]}
    acc = exe.V4Accumulator(universe, set(), set(), set(), set())
    for i in range(exe.TARGET_PER_ARM + 3):
        acc.accept("v4-9101", 1, "T", "S", "https://t.example.com/%d" % i, _full_extra())
    assert sum(1 for r in acc.accepted if r["designation"] == "treatment") == exe.TARGET_PER_ARM
    assert acc.rejected["TREATMENT_TARGET_FILLED"] == 3
    assert acc.rejected["DUPLICATE_CURRENT"] == 0


def test_precapture_integrity_gate_passes_offline():
    # precapture() raises SystemExit(BLOCKED) on any frozen-contract violation.
    exe.precapture()


def test_exclusion_sets_are_populated_and_disjoint():
    v1, supplemental, v3, historical = exe.exclusion_sets()
    assert len(v1) >= 300 and len(supplemental) >= 100 and len(v3) >= 50
    assert len(historical) > 0
    # The three corpora must not reuse each other's URLs; historical is a
    # superset that legitimately includes pilot/V1 URLs, so it is not disjoint.
    assert not (v1 & supplemental) and not (v1 & v3) and not (supplemental & v3)


def test_write_packets_creates_blinded_unlabeled_packets(monkeypatch):
    corpus = {"schema": "amatl.relevance.supplemental-v4-final-prelabel-corpus.v1",
              "corpus_id": "synthetic-v4", "status": "FROZEN_PRELABEL",
              "rows": [{"row_id": "sup-v4-q001-r001", "query_id": "sup-v4-t-0001",
                        "query": "what is x", "designation": "treatment",
                        "strategy_class": "S2_COMPACT_INFORMATIONAL_TREATMENT",
                        "original_url": "https://a.example.com/", "canonical_url": "https://a.example.com/",
                        "domain": "a.example.com", "title": "T", "snippet": "S", "rank": 1,
                        "acquisition_run_id": "independent-relevance-supplemental-v4",
                        "provider_metadata": {"engine": "wiby"}}]}
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        monkeypatch.setattr(exe, "OUT", tmp)
        corpus_path = tmp / "corpus.json"
        corpus_path.write_text(exe.canonical_json(corpus), encoding="utf-8")
        manifest = exe.write_packets(corpus, corpus_path)
        for name in (exe.PACKET_A, exe.PACKET_B, exe.PACKET_MANIFEST):
            assert (tmp / name).exists()
        pa = json.loads((tmp / exe.PACKET_A).read_text(encoding="utf-8"))
        pb = json.loads((tmp / exe.PACKET_B).read_text(encoding="utf-8"))
        assert pa["packet_id"] == "A" and pb["packet_id"] == "B"
        assert pa["rows"] == pb["rows"]
        prow = pa["rows"][0]
        assert prow["row_id"] == "sup-v4-q001-r001"
        assert prow["label"] == "" and prow["annotation_note"] == ""
        # Arm blinding: designation/strategy_class/query_id and acquisition
        # metadata must not leak into packet rows.
        for banned in ("designation", "strategy_class", "hypothesis_feature", "query_id",
                       "acquisition_run_id", "provider_metadata", "raw_response_path",
                       "raw_response_sha256", "provider_or_source", "acquired_at"):
            assert banned not in prow, banned
        assert manifest["packet_rows"] == 1
        assert manifest["packet_hashes"]["A"] == exe.sha(tmp / exe.PACKET_A)
        assert manifest["packet_hashes"]["B"] == exe.sha(tmp / exe.PACKET_B)
        assert exe.sha(tmp / exe.PACKET_A) != exe.sha(tmp / exe.PACKET_B)


