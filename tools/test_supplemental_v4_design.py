"""Reproducible offline tests for the frozen V4 design artifacts."""
import hashlib
import json
from pathlib import Path

from freeze_supplemental_v4_design import ROOT, build_queries, norm_query, required_n


V4 = ROOT / "docs/evaluation/independent-relevance/new-corpus-v1/v4"


def load(name):
    return json.loads((V4 / name).read_text(encoding="utf-8"))


def test_design_sample_size_and_contract():
    d = load("supplemental-v4-confirmatory-design.json")
    assert d["experiment_type"] == "CONFIRMATORY"
    assert d["primary_endpoint"] == "STRICT_RELEVANCE_YIELD"
    assert d["sample_size"]["required_n_per_arm_power80"] == required_n(.05, .10, .80) == 343
    assert d["sample_size"]["required_n_per_arm_power90"] == required_n(.05, .10, .90) == 474
    assert d["sample_size"]["frozen_n_per_arm"] == 474
    assert d["alpha"] == .05 and d["power_target"] == .90


def test_universe_integrity_and_arm_balance():
    u = load("supplemental-v4-query-universe.json")
    qs = u["queries"]
    ids = [q["query_id"] for q in qs]
    texts = [norm_query(q["query_text"]) for q in qs]
    assert len(qs) == 600
    assert len(set(ids)) == len(ids)
    assert len(set(texts)) == len(texts)
    assert sum(q["designation"] == "treatment" for q in qs) == 300
    assert sum(q["designation"] == "control" for q in qs) == 300
    assert {q["expected_depth"] for q in qs} == {3}
    assert [q["capture_order"] for q in qs] == list(range(1, 601))


def test_manifest_attestation_hash_chain_and_no_network():
    u_path = V4 / "supplemental-v4-query-universe.json"
    m_path = V4 / "supplemental-v4-query-universe-manifest.json"
    a_path = V4 / "supplemental-v4-query-universe-attestation.json"
    m = load(m_path.name)
    a = load(a_path.name)
    digest = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
    assert m["universe_sha256"] == digest(u_path)
    assert a["universe_sha256"] == digest(u_path)
    assert a["manifest_sha256"] == digest(m_path)
    assert a["chain"] == "universe -> manifest -> attestation"
    assert a["self_hash_in_manifest"] is False
    assert a["network_requests"] == 0


def test_offline_preflight_passes():
    r = load("supplemental-v4-preflight-offline-report.json")
    assert r["preflight"] == "PASS"
    assert r["network_requests"] == 0
    assert r["checks"]["known_query_overlap"] == 0
    assert r["work_package_status"] == "COMPLETE_READY_FOR_CONTROLLED_CAPTURE"
