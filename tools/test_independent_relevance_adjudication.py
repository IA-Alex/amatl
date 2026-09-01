#!/usr/bin/env python3
"""Regression checks for the independent-relevance agreement artifacts."""
import hashlib
import json
import subprocess
import sys
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

    def test_no_ground_truth_is_created_before_human_adjudication(self):
        self.assertEqual(self.report["WORK_PACKAGE_STATUS"], "BLOCKED_WAITING_FOR_ADJUDICATION")
        self.assertEqual(self.report["ADJUDICATION_STATUS"], "WAITING_FOR_HUMAN")
        self.assertFalse((OUT / "final-ground-truth.json").exists())

    def test_validation_preserves_a_human_adjudication_entry(self):
        packet_path = OUT / "adjudication-packet.json"
        packet = json.loads(packet_path.read_text())
        packet["rows"][0]["adjudicated_label"] = "Relevant"
        packet_path.write_text(json.dumps(packet, indent=2) + "\n")
        try:
            subprocess.run([sys.executable, str(ROOT / "tools/validate_independent_relevance_adjudication.py")], cwd=ROOT, check=True)
            preserved = json.loads(packet_path.read_text())
            self.assertEqual(preserved["rows"][0]["adjudicated_label"], "Relevant")
        finally:
            packet["rows"][0]["adjudicated_label"] = ""
            packet_path.write_text(json.dumps(packet, indent=2) + "\n")
            subprocess.run([sys.executable, str(ROOT / "tools/validate_independent_relevance_adjudication.py")], cwd=ROOT, check=True)


if __name__ == "__main__":
    unittest.main()
