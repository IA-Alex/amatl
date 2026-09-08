#!/usr/bin/env python3
import json
import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "docs/evaluation/independent-relevance/new-corpus-v1/adjudication"
sys.path.insert(0, str(ROOT / "tools"))
from supplemental_v2_contract import build_accumulator

class SupplementalV2PreflightTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        subprocess.run([sys.executable, str(ROOT / "tools/freeze_supplemental_query_universe_v2.py")], cwd=ROOT, check=True)
        subprocess.run([sys.executable, str(ROOT / "tools/validate_supplemental_v2_preflight.py")], cwd=ROOT, check=True)
        cls.report = json.loads((OUT / "supplemental-v2-preflight-report.json").read_text())
        cls.manifest = json.loads((OUT / "supplemental-query-universe-v2-manifest.json").read_text())

    def test_v2_is_frozen_s1_only_and_offline(self):
        self.assertEqual(self.report["SUPPLEMENTAL_V2_PREFLIGHT"], "PASS")
        self.assertEqual(self.report["SUPPLEMENTAL_V2_SCOPE"], "S1_ONLY")
        self.assertEqual(self.report["NETWORK_REQUESTS"], 0)
        self.assertEqual(self.manifest["valid_deficit"], 8)
        self.assertGreaterEqual(self.manifest["structural_capacity"], 32)

    def test_v2_multirun_contracts_are_present(self):
        self.assertEqual(self.report["MULTIRUN_DEDUP_CONTRACT"], "PASS")
        self.assertEqual(self.report["MULTIRUN_PROVENANCE_CONTRACT"], "PASS")
        self.assertEqual(self.report["FINAL_EXPECTED_COMPOSITION"], "172+8")

    def test_gate_preserves_v1_and_marks_v2_provenance(self):
        universe = json.loads((OUT / "supplemental-query-universe-v2.json").read_text())
        gate = build_accumulator(universe)
        query = universe["queries"][0]
        existing = next(iter(gate.supplemental_v1_urls))
        self.assertEqual(gate.accept_or_reject(query["query_id"], "searxng", {"original_url": existing, "rank": 1, "title": "x", "snippet": "x"})["reason_code"], "OVERLAP_SUPPLEMENTAL_V1")
        accepted = gate.accept_or_reject(query["query_id"], "searxng", {"original_url": "https://new-v2.example/", "rank": 1, "title": "x", "snippet": "x"})
        self.assertEqual(accepted["SOURCE_RUN"], "supplemental-v2")

if __name__ == "__main__": unittest.main()
