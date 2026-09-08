#!/usr/bin/env python3
"""Focused regression tests for the STEP4E offline evaluator."""
import unittest

import step4e_metrics


class Step4eMetricsTest(unittest.TestCase):
    def test_metric_block_has_complete_confusion_and_weighted_f1(self):
        metrics = step4e_metrics.metric_block(
            ["Relevant", "PossiblyRelevant", "NotRelevant"],
            ["Relevant", "NotRelevant", "NotRelevant"],
        )
        self.assertEqual(metrics["confusion_matrix"]["NotRelevant"]["PossiblyRelevant"], 1)
        self.assertEqual(metrics["prediction_distribution"]["Relevant"], 1)
        self.assertAlmostEqual(metrics["weighted_f1"], 7 / 9)

    def test_error_analysis_keeps_exact_ids_and_direction(self):
        ids = ["one-direct", "two-limited"]
        rows = {
            "one-direct": {"query": "same query"},
            "two-limited": {"query": "same query"},
        }
        analysis = step4e_metrics.error_analysis(
            ids,
            ["PossiblyRelevant", "NotRelevant"],
            ["Relevant", "PossiblyRelevant"],
            rows,
        )
        self.assertEqual(
            [error["row_id"] for error in analysis["errors"]], ids
        )
        self.assertEqual(
            analysis["directional_confusion"]["Relevant"]["PossiblyRelevant"], 1
        )
        self.assertEqual(analysis["query_error_concentration"][0]["error_count"], 2)

    def test_alignment_rejects_missing_rows(self):
        with self.assertRaisesRegex(ValueError, "row ID mismatch"):
            step4e_metrics.validate_alignment(
                {"one": {}}, {"two": {"prediction": "Relevant"}}, "candidate"
            )


if __name__ == "__main__":
    unittest.main()
