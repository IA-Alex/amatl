#!/usr/bin/env python3
"""Regression checks for the independent-relevance agreement artifacts."""
import hashlib
import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / "docs/evaluation/independent-relevance/new-corpus-v1"
PRELABEL = BASE / "prelabel"
OUT = BASE / "adjudication"


class IndependentRelevanceAgreementTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        subprocess.run([sys.executable, str(ROOT / "tools/validate_independent_relevance_adjudication.py")], cwd=ROOT, check=True)
        cls.report = json.loads((OUT / "agreement-report.json").read_text())
        cls.packet = json.loads((OUT / "adjudication-packet.json").read_text())

    def test_inputs_are_complete_and_identical_to_prelabel(self):
        self.assertEqual(self.report["LABELER_A_VALIDATION"], "PASS")
        self.assertEqual(self.report["LABELER_B_VALIDATION"], "PASS")
        for name in ("labeler_a_validation", "labeler_b_validation"):
            result = self.report[name]
            self.assertEqual(result["TOTAL_ROWS"], 368)
            self.assertEqual(result["VALID_ROWS"], 368)
            for field in ("MISSING_ROWS", "EXTRA_ROWS", "DUPLICATE_ROW_IDS", "INVALID_LABELS", "EMPTY_LABELS", "ORIGINAL_FIELD_MISMATCH"):
                self.assertEqual(result[field], [])

    def test_agreement_math_and_matrix_are_complete(self):
        matrix = self.report["DISAGREEMENT_MATRIX"]
        self.assertEqual(sum(sum(row.values()) for row in matrix.values()), 368)
        self.assertEqual(self.report["AGREEMENTS"] + self.report["DISAGREEMENTS"], 368)
        self.assertEqual(self.report["DISAGREEMENTS"], 32)
        self.assertAlmostEqual(self.report["PERCENT_AGREEMENT"], 100 * 336 / 368, places=6)
        self.assertAlmostEqual(self.report["COHEN_KAPPA"], 0.684441824, places=9)

    def test_packet_is_exactly_the_disagreements_and_has_no_system_outputs(self):
        rows = self.packet["rows"]
        self.assertEqual(len(rows), self.report["DISAGREEMENTS"])
        self.assertEqual({row["row_id"] for row in rows}, set(sum(self.report["DISAGREEMENT_ROW_IDS"].values(), [])))
        self.assertTrue(all(row["label_a"] != row["label_b"] for row in rows))
        forbidden = {"arm_a", "arm_b", "score", "embedding", "prediction", "ranking"}
        self.assertTrue(all(not (forbidden & set(row)) for row in rows))
        self.assertEqual(hashlib.sha256((OUT / "adjudication-packet.json").read_bytes()).hexdigest(), self.report["ADJUDICATION_PACKET_HASH"])

    def test_complete_human_adjudication_enables_only_the_rule_based_final_corpus(self):
        self.assertEqual(self.report["WORK_PACKAGE_STATUS"], "COMPLETE")
        self.assertEqual(self.report["ADJUDICATION_STATUS"], "COMPLETE")
        self.assertTrue((OUT / "final-ground-truth.json").exists())
        final = json.loads((OUT / "final-ground-truth.json").read_text())
        adjudicated = {row["row_id"]: row["adjudicated_label"] for row in self.packet["rows"]}
        self.assertEqual(len(final["rows"]), 368)
        self.assertTrue(all(row["final_label"] == adjudicated[row["row_id"]]
                            for row in final["rows"] if row["row_id"] in adjudicated))

    def test_validation_preserves_a_human_adjudication_entry(self):
        # Never mutate the actual human adjudication record to test preservation.
        with tempfile.TemporaryDirectory() as temp:
            output = Path(temp)
            packet_path = output / "adjudication-packet.json"
            shutil.copy2(OUT / "adjudication-packet.json", packet_path)
            packet = json.loads(packet_path.read_text())
            original = packet["rows"][0]["adjudicated_label"]
            subprocess.run([
                sys.executable, str(ROOT / "tools/validate_independent_relevance_adjudication.py"),
                "--output-dir", str(output),
            ], cwd=ROOT, check=True)
            preserved = json.loads(packet_path.read_text())
            self.assertEqual(preserved["rows"][0]["adjudicated_label"], original)


if __name__ == "__main__":
    unittest.main()
