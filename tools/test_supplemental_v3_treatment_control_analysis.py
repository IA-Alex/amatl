import json
import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "docs/evaluation/independent-relevance/new-corpus-v1/adjudication"
SCRIPT = ROOT / "tools/analyze_supplemental_v3_treatment_control.py"


class SupplementalV3AnalysisTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        subprocess.run([sys.executable, str(SCRIPT)], cwd=ROOT, check=True)
        cls.a = json.loads((OUT / "supplemental-v3-treatment-control-analysis.json").read_text())
        cls.gt = json.loads((OUT / "supplemental-v3-ground-truth-v1.json").read_text())

    def test_partition_and_counts(self):
        self.assertEqual(self.a["aggregate_distribution"]["rows"], 60)
        self.assertEqual(self.a["treatment_distribution"]["rows"], 39)
        self.assertEqual(self.a["control_distribution"]["rows"], 21)
        self.assertEqual(sum(self.a["treatment_distribution"]["counts"].values()), 39)
        self.assertEqual(sum(self.a["control_distribution"]["counts"].values()), 21)
        self.assertEqual(self.a["aggregate_distribution"]["counts"]["Relevant"], 5)

    def test_metrics_and_fisher_accounting(self):
        t = self.a["treatment_distribution"]; c = self.a["control_distribution"]
        self.assertEqual(t["counts"]["Relevant"] + c["counts"]["Relevant"], 5)
        self.assertAlmostEqual(t["strict_yield"], 4 / 39)
        self.assertAlmostEqual(c["strict_yield"], 1 / 21)
        self.assertEqual(sum(self.a["strict_metrics"]["fisher"]["table"][0]), 39)
        self.assertEqual(sum(self.a["strict_metrics"]["fisher"]["table"][1]), 21)
        self.assertAlmostEqual(self.a["strict_metrics"]["fisher"]["odds_ratio"], 16 / 7)
        self.assertAlmostEqual(self.a["strict_metrics"]["fisher"]["p_value_two_sided"], 0.6485973115137347)
        self.assertEqual(self.a["overlap_validation"]["counts"], {"v1": 0, "previous_supplemental": 0, "historical": 0})

    def test_ci_hashes_and_stop_boundary(self):
        self.assertEqual(self.a["input_integrity"]["status"], "PASS")
        self.assertEqual(self.a["overlap_validation"]["status"], "PASS")
        self.assertEqual(self.a["network_requests"], 0)
        self.assertFalse(self.a["frozen_artifacts_changed"])
        self.assertEqual(self.a["decision"], "RUN_CONFIRMATORY_V4")
        for ci in (self.a["treatment_distribution"]["strict_ci95_wilson"], self.a["control_distribution"]["strict_ci95_wilson"]):
            self.assertGreaterEqual(ci[0], 0); self.assertLessEqual(ci[1], 1); self.assertLess(ci[0], ci[1])


if __name__ == "__main__":
    unittest.main()
