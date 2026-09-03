#!/usr/bin/env python3
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
from supplemental_pilot_contract import PilotAccumulator, load_json

OUT = ROOT / "docs/evaluation/independent-relevance/new-corpus-v1/adjudication"
REPORT = OUT / "supplemental-pilot-preflight-report.json"


def result(url, rank=1): return {"original_url": url, "canonical_url": url, "rank": rank, "title": "title", "snippet": "snippet"}


class SupplementalPilotPreflightTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        # The repository configuration may contain an authorized endpoint.  The
        # failure-mode test must instead exercise an isolated missing-endpoint
        # fixture, without rewriting frozen v1 artifacts or its report.
        with tempfile.TemporaryDirectory() as temp:
            temp = Path(temp)
            config = json.loads((OUT / "supplemental-pilot-searxng-capture-config.json").read_text())
            config.pop("searxng_endpoint_binding")
            config_path, report_path = temp / "missing-endpoint.json", temp / "report.json"
            config_path.write_text(json.dumps(config), encoding="utf-8")
            subprocess.run([sys.executable, str(ROOT / "tools/validate_supplemental_pilot_preflight.py"), "--capture-config", str(config_path), "--output", str(report_path)], cwd=ROOT, check=True, stdout=subprocess.DEVNULL)
            cls.report = json.loads(report_path.read_text())
        cls.universe = load_json(OUT / "supplemental-query-universe-v1.json")

    def test_preflight_is_offline_and_fail_closed_without_operator_endpoint(self):
        self.assertEqual(self.report["status"], "BLOCKED")
        self.assertEqual(self.report["endpoint_status"], "BLOCKED_MISSING_EXPLICIT_OPERATOR_ENDPOINT")
        self.assertEqual(self.report["network_requests"], 0)
        self.assertEqual(self.report["provider_contract"], "SEARXNG_ONLY")
        self.assertFalse(self.report["marginalia_reachable_from_pilot_path"])
        self.assertEqual(self.report["dynamic_expansion"], "DISABLED")

    def test_capacity_has_margin_and_s3_balance(self):
        self.assertEqual(self.report["structural_capacity"], {"S1_PROCEDURAL_SHALLOW": 90, "S2_INFORMATIONAL_SHALLOW": 90, "S3_MIXED_DEPTH_CONTROL": 200})
        self.assertEqual(self.report["query_counts"], {"S1_PROCEDURAL_SHALLOW": 30, "S2_INFORMATIONAL_SHALLOW": 30, "S3_MIXED_DEPTH_CONTROL": 40})

    def test_runner_rejects_contamination_and_wrong_contract(self):
        first = self.universe["queries"][0]
        gate = PilotAccumulator(self.universe, v1_urls={"https://v1.example/"}, historical_urls={"https://history.example/"})
        self.assertEqual(gate.accept_or_reject("not-frozen", "searxng", result("https://x.example/"))["reason_code"], "OUTSIDE_FROZEN_UNIVERSE")
        self.assertEqual(gate.accept_or_reject(first["query_id"], "marginalia", result("https://x.example/"))["reason_code"], "OUTSIDE_PROVIDER_CONTRACT")
        self.assertEqual(gate.accept_or_reject(first["query_id"], "searxng", result("https://x.example/", 4))["reason_code"], "OUTSIDE_ALLOWED_DEPTH")
        self.assertEqual(gate.accept_or_reject(first["query_id"], "searxng", result("https://v1.example/"))["reason_code"], "OVERLAP_V1")
        self.assertEqual(gate.accept_or_reject(first["query_id"], "searxng", result("https://history.example/"))["reason_code"], "OVERLAP_HISTORICAL")
        self.assertEqual(gate.accept_or_reject(first["query_id"], "searxng", result("https://accepted.example/"))["reason_code"], "ACCEPTED")
        self.assertEqual(gate.accept_or_reject(first["query_id"], "searxng", result("https://accepted.example/"))["reason_code"], "DUPLICATE_CURRENT_PILOT")

    def test_quota_and_exhaustion_cannot_expand_universe(self):
        gate = PilotAccumulator(self.universe)
        first = self.universe["queries"][0]
        for index in range(60): self.assertEqual(gate.accept_or_reject(first["query_id"], "searxng", result(f"https://quota-{index}.example/"))["reason_code"], "ACCEPTED")
        self.assertEqual(gate.accept_or_reject(first["query_id"], "searxng", result("https://over-quota.example/"))["reason_code"], "QUOTA_FILLED")
        self.assertEqual(gate.exhausted_status(), "EXHAUSTED_FROZEN_UNIVERSE")

    def test_offline_runner_never_uses_network(self):
        completed = subprocess.run([sys.executable, str(ROOT / "tools/run_supplemental_pilot.py")], cwd=ROOT, check=True, capture_output=True, text=True)
        output = json.loads(completed.stdout)
        self.assertEqual(output["network_requests"], 0)
        self.assertEqual(output["source"], "offline-results only")
        self.assertEqual(output["status"], "EXHAUSTED_FROZEN_UNIVERSE")


if __name__ == "__main__": unittest.main()
