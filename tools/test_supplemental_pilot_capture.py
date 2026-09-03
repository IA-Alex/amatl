#!/usr/bin/env python3
import json
import copy
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
from supplemental_pilot_capture import capture
from supplemental_pilot_capture_contract import load_json, validate_config, validate_endpoint_binding, validate_raw_evidence

OUT = ROOT / "docs/evaluation/independent-relevance/new-corpus-v1/adjudication"


class SupplementalPilotCaptureTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.config = load_json(OUT / "supplemental-pilot-searxng-capture-config.json")
        cls.universe = load_json(OUT / "supplemental-query-universe-v1.json")
        cls.query = cls.universe["queries"][0]

    def fixture(self):
        return {self.query["query_id"]: [{"results": [
            {"title": "one", "url": "https://one.example/", "content": "first"},
            {"title": "two", "url": "https://two.example/", "content": "second"},
            {"title": "three", "url": "https://three.example/", "content": "third"},
            {"title": "four", "url": "https://four.example/", "content": "excluded"},
        ]}]}

    def authorized_config(self):
        config = copy.deepcopy(self.config)
        config["searxng_endpoint_binding"]["endpoint"] = "https://searxng.operator.example/search"
        config["searxng_endpoint_binding"]["endpoint_sha256"] = "fabd15b4681bc33203c6db8efa5ef33ab51857d1af48890fcafdc91fdc11abb2"
        return config

    def test_fixture_capture_is_searxng_only_and_depth_bounded(self):
        config = self.authorized_config()
        evidence = capture(config, self.universe, [self.query["query_id"]], self.fixture())
        self.assertTrue(validate_raw_evidence(evidence, self.universe, config))
        self.assertEqual(evidence["provider"], "searxng")
        self.assertEqual([r["rank"] for r in evidence["attempts"][0]["results"]], [1, 2, 3])
        self.assertIn("artifact_sha256", evidence)

    def test_empty_and_timeout_retry_are_retained(self):
        config = self.authorized_config()
        empty = capture(config, self.universe, [self.query["query_id"]], {self.query["query_id"]: [{"results": []}]})
        self.assertEqual(empty["attempts"][0]["result_count"], 0)
        retried = capture(config, self.universe, [self.query["query_id"]], {self.query["query_id"]: [{"raise": "simulated"}, {"results": []}]})
        self.assertEqual([a["attempt_number"] for a in retried["attempts"]], [1, 2])
        self.assertEqual(retried["attempts"][0]["error"]["error_class"], "TIMEOUT")

    def test_failure_closed_for_provider_hash_and_membership(self):
        config = self.authorized_config()
        bad = dict(config); bad["provider"] = "marginalia"
        with self.assertRaisesRegex(ValueError, "CAPTURE_PROVIDER_NOT_SEARXNG"):
            validate_config(bad, self.universe)
        bad = dict(config); bad["query_universe_hash"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "CAPTURE_UNIVERSE_HASH_MISMATCH"):
            validate_config(bad, self.universe)
        with self.assertRaisesRegex(ValueError, "CAPTURE_QUERY_OUTSIDE_FROZEN_UNIVERSE"):
            capture(config, self.universe, ["not-frozen"], self.fixture())
        evidence = capture(config, self.universe, [self.query["query_id"]], self.fixture())
        bad_evidence = copy.deepcopy(evidence); bad_evidence["provider"] = "marginalia"
        with self.assertRaisesRegex(ValueError, "RAW_EVIDENCE_PROVIDER_NOT_SEARXNG"):
            validate_raw_evidence(bad_evidence, self.universe, config)
        bad_evidence = copy.deepcopy(evidence); bad_evidence["attempts"][0]["results"][0]["rank"] = 9
        with self.assertRaisesRegex(ValueError, "RAW_EVIDENCE_RESULT_OUTSIDE_DEPTH"):
            validate_raw_evidence(bad_evidence, self.universe, config)
        secret_fixture = {self.query["query_id"]: [{"results": [{"title": "x", "url": "https://x.example/?token=no", "content": "x"}]}]}
        evidence = capture(config, self.universe, [self.query["query_id"]], secret_fixture)
        self.assertEqual(evidence["attempts"][0]["error"]["error_class"], "TRANSPORT_FAILURE")

    def test_endpoint_binding_is_explicit_validated_and_immutable(self):
        config = self.authorized_config()
        self.assertEqual(validate_endpoint_binding(config), "https://searxng.operator.example/search")
        missing = copy.deepcopy(config); del missing["searxng_endpoint_binding"]
        with self.assertRaisesRegex(ValueError, "CAPTURE_ENDPOINT_BINDING_MISSING"): validate_config(missing, self.universe)
        for endpoint, error in (("", "CAPTURE_ENDPOINT_MISSING"), ("searxng.example", "CAPTURE_ENDPOINT_INVALID_URL"),
                                ("https://user:pass@searxng.example", "CAPTURE_ENDPOINT_EMBEDDED_CREDENTIALS_FORBIDDEN")):
            bad = self.authorized_config(); bad["searxng_endpoint_binding"]["endpoint"] = endpoint
            with self.assertRaisesRegex(ValueError, error): validate_config(bad, self.universe)
        bad = self.authorized_config(); bad["searxng_endpoint_binding"]["endpoint"] = "https://other.operator.example"
        with self.assertRaisesRegex(ValueError, "CAPTURE_ENDPOINT_AUTHORIZED_VALUE_MISMATCH"):
            validate_config(bad, self.universe)
        bad = self.authorized_config(); bad["searxng_endpoint_binding"]["endpoint"] = "https://other.operator.example?fallback=yes"
        with self.assertRaisesRegex(ValueError, "CAPTURE_ENDPOINT_INVALID_URL"):
            validate_config(bad, self.universe)

    def test_dynamic_provider_fallback_and_marginalia_are_impossible(self):
        for field, value, error in (("provider_fallback", "marginalia", "CAPTURE_PROVIDER_FALLBACK_FORBIDDEN"),
                                    ("dynamic_provider_selection", True, "CAPTURE_DYNAMIC_PROVIDER_SELECTION_FORBIDDEN"),
                                    ("provider", "marginalia", "CAPTURE_PROVIDER_NOT_SEARXNG")):
            bad = self.authorized_config(); bad[field] = value
            with self.assertRaisesRegex(ValueError, error): validate_config(bad, self.universe)

    def test_general_multi_provider_config_cannot_leak(self):
        general = {"providers": {"enabled": ["searxng", "marginalia"]}}
        evidence = capture(self.authorized_config(), self.universe, [self.query["query_id"]], self.fixture())
        self.assertEqual(general["providers"]["enabled"], ["searxng", "marginalia"])
        self.assertEqual({a["provider"] for a in evidence["attempts"]}, {"searxng"})

    def test_transport_to_runner_handoff(self):
        evidence = capture(self.authorized_config(), self.universe, [self.query["query_id"]], self.fixture())
        with tempfile.TemporaryDirectory() as directory:
            raw = Path(directory) / "raw.json"
            config = Path(directory) / "capture-config.json"
            raw.write_text(json.dumps(evidence), encoding="utf-8")
            config.write_text(json.dumps(self.authorized_config()), encoding="utf-8")
            completed = subprocess.run([sys.executable, str(ROOT / "tools/run_supplemental_pilot.py"), "--raw-evidence", str(raw),
                                        "--capture-config", str(config)],
                                       cwd=ROOT, check=True, capture_output=True, text=True)
        report = json.loads(completed.stdout)
        self.assertEqual(report["network_requests"], 0)
        self.assertEqual(report["valid_counts"][self.query["assigned_stratum"]], 3)


if __name__ == "__main__": unittest.main()
