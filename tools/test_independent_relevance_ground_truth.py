#!/usr/bin/env python3
"""Protocol tests for the frozen adjudicated V1 ground truth."""
import hashlib
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "docs/evaluation/independent-relevance/new-corpus-v1/adjudication"


class GroundTruthTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        subprocess.run([sys.executable, str(ROOT / "tools/freeze_independent_relevance_v1.py")], cwd=ROOT, check=True)
        cls.ground = json.loads((OUT / "v1-ground-truth.json").read_text())
        cls.manifest = json.loads((OUT / "v1-ground-truth-manifest.json").read_text())
        cls.plan = json.loads((OUT / "supplemental-pilot-design.json").read_text())

    def test_ground_truth_reconstruction_and_distribution(self):
        rows = self.ground["rows"]
        self.assertEqual(len(rows), 368)
        self.assertEqual(len({r["row_id"] for r in rows}), 368)
        self.assertEqual(sum(r["final_label_source"] == "AGREEMENT" for r in rows), 336)
        self.assertEqual(sum(r["final_label_source"] == "ADJUDICATION" for r in rows), 32)
        self.assertTrue(all(r["final_label"] == r["label_a"] == r["label_b"] for r in rows if r["final_label_source"] == "AGREEMENT"))
        self.assertTrue(all(r["final_label"] in {"Relevant", "PossiblyRelevant", "NotRelevant", "Unknown"} for r in rows))
        self.assertEqual(self.manifest["CLASS_DISTRIBUTION"], {"Relevant": 9, "PossiblyRelevant": 51, "NotRelevant": 308})

    def test_hash_is_reproducible_and_covers_labels_and_original_fields(self):
        raw = (OUT / "v1-ground-truth.json").read_bytes()
        self.assertEqual(hashlib.sha256(raw).hexdigest(), self.manifest["GROUND_TRUTH_HASH"])
        changed_label = json.loads(raw); changed_label["rows"][0]["final_label"] = "Unknown"
        changed_source = json.loads(raw); changed_source["rows"][0]["title"] += " changed"
        def canonical(value):
            return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()
        self.assertNotEqual(canonical(changed_label), self.manifest["GROUND_TRUTH_CANONICAL_HASH"])
        self.assertNotEqual(canonical(changed_source), self.manifest["GROUND_TRUTH_CANONICAL_HASH"])

    def test_role_deficits_and_prelabel_only_plan(self):
        self.assertEqual(self.manifest["V1_ROLE"], "KNOWN_INDEPENDENT_GROUND_TRUTH")
        self.assertEqual({k: max(0, 100-v) for k, v in self.manifest["CLASS_DISTRIBUTION"].items() if k != "Unknown"}, {"Relevant": 91, "PossiblyRelevant": 49, "NotRelevant": 0})
        forbidden = {"final_label", "expected_class", "prediction", "score", "embedding"}
        self.assertEqual(self.plan["execution_status"], "BLOCKED")
        self.assertTrue(all(not (forbidden & set(s)) for s in self.plan["strata"]))
        self.assertEqual(sum(s["RAW_TARGET"] for s in self.plan["strata"]), self.plan["pilot_raw_target"])

    def test_rerun_with_different_source_commit_preserves_manifest_and_hashes(self):
        paths = [OUT / name for name in ("v1-ground-truth.json", "v1-yield-analysis.json",
                                         "supplemental-pilot-design.json", "v1-ground-truth-manifest.json")]
        before = {path: path.read_bytes() for path in paths}
        subprocess.run([sys.executable, str(ROOT / "tools/freeze_independent_relevance_v1.py"),
                        "--source-commit", "1" * 40], cwd=ROOT, check=True)
        self.assertEqual({path: path.read_bytes() for path in paths}, before)
        self.assertEqual(self.manifest["SOURCE_COMMIT"], "b0efd57af5291c1fb2e030ab938b9377957084e7")


if __name__ == "__main__":
    unittest.main()
