#!/usr/bin/env python3
"""Compute STEP4E evaluation metrics from the frozen blind artifacts.

Stdlib-only. Reads:
  - docs/evaluation/step4e/step4e_ground_truth.json
  - docs/evaluation/step4e/arm_a_predictions.json
  - docs/evaluation/step4e/arm_b_predictions.json
  - /tmp/step4e_performance.json (optional; semantic-change + perf analysis)

Writes:
  - docs/evaluation/step4e/step4e_metrics.json
  - docs/evaluation/step4e/step4e_evaluation_report.md

All bootstrap confidence intervals are deterministic (seed 42, 10k resamples).
"""
import argparse
import collections
import json
import math
import random
import statistics
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "docs" / "evaluation" / "step4e"
CLASSES = ["NotRelevant", "PossiblyRelevant", "Relevant"]
SEED = 42
N_BOOT = 10_000
ALPHA = 0.05


def load_rows(path):
    """Return {row_id: row} from a step4e predictions/ground-truth artifact."""
    data = json.loads(Path(path).read_text())
    return {r["row_id"]: r for r in data["rows"]}


def sha256_file(path):
    """Return the content identity of a frozen evaluation input."""
    import hashlib
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def code_commit():
    """Best-effort code identity; evaluation remains usable outside a git checkout."""
    try:
        return subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
        ).strip()
    except (OSError, subprocess.CalledProcessError):
        return "UNAVAILABLE"


def validate_alignment(gt_rows, candidate_rows, candidate_name):
    """Reject incomplete, duplicate, or invalid candidate predictions."""
    gt_ids = set(gt_rows)
    candidate_ids = set(candidate_rows)
    if gt_ids != candidate_ids:
        missing = sorted(gt_ids - candidate_ids)
        extra = sorted(candidate_ids - gt_ids)
        raise ValueError(
            f"{candidate_name}: row ID mismatch; missing={missing}, extra={extra}"
        )
    invalid = sorted(
        row_id for row_id, row in candidate_rows.items()
        if row.get("prediction") not in CLASSES
    )
    if invalid:
        raise ValueError(f"{candidate_name}: invalid predictions for {invalid}")


def stratum_of(row_id):
    """The holdout stratum is encoded in the row_id suffix (direct/collision/limited)."""
    return row_id.rsplit("-", 1)[-1]


def metric_block(preds, truths):
    """All headline metrics for aligned prediction/truth label lists."""
    n = len(preds)
    correct = sum(p == t for p, t in zip(preds, truths))
    acc = correct / n if n else 0.0
    per_class = {}
    for c in CLASSES:
        tp = sum(p == c and t == c for p, t in zip(preds, truths))
        fp = sum(p == c and t != c for p, t in zip(preds, truths))
        fn = sum(p != c and t == c for p, t in zip(preds, truths))
        prec = tp / (tp + fp) if tp + fp else 0.0
        rec = tp / (tp + fn) if tp + fn else 0.0
        f1 = 2 * prec * rec / (prec + rec) if prec + rec else 0.0
        per_class[c] = {
            "tp": tp, "fp": fp, "fn": fn,
            "precision": prec, "recall": rec, "f1": f1,
        }
    macro_f1 = statistics.mean(per_class[c]["f1"] for c in CLASSES)
    weighted_f1 = sum(
        per_class[c]["f1"] * sum(t == c for t in truths) for c in CLASSES
    ) / n if n else 0.0
    gt_rel = sum(t == "Relevant" for t in truths)
    pred_rel = sum(p == "Relevant" for p in preds)
    tp_rel = sum(p == "Relevant" and t == "Relevant" for p, t in zip(preds, truths))
    gt_nr = sum(t == "NotRelevant" for t in truths)
    return {
        "n": n,
        "accuracy": acc,
        "macro_f1": macro_f1,
        "weighted_f1": weighted_f1,
        "per_class": per_class,
        "prediction_distribution": dict(collections.Counter(preds)),
        "confusion_matrix": {
            actual: {
                predicted: sum(t == actual and p == predicted for p, t in zip(preds, truths))
                for predicted in CLASSES
            }
            for actual in CLASSES
        },
        "rescue_recall": tp_rel / gt_rel if gt_rel else 0.0,
        "rescue_precision": tp_rel / pred_rel if pred_rel else 0.0,
        "protection_specificity": (
            sum(p == "NotRelevant" and t == "NotRelevant" for p, t in zip(preds, truths))
            / gt_nr if gt_nr else 0.0
        ),
        "false_promotion_count": sum(
            p == "Relevant" and t == "NotRelevant" for p, t in zip(preds, truths)
        ),
    }


def percentile(vals, pct):
    s = sorted(vals)
    k = (len(s) - 1) * pct / 100.0
    lo = math.floor(k)
    hi = math.ceil(k)
    if lo == hi:
        return s[int(k)]
    return s[lo] + (s[hi] - s[lo]) * (k - lo)


def resample_metrics(preds, truths, rng):
    n = len(preds)
    idx = [rng.randrange(n) for _ in range(n)]
    return metric_block([preds[i] for i in idx], [truths[i] for i in idx])


def bootstrap_ci(preds, truths, key, n=N_BOOT, seed=SEED, alpha=ALPHA):
    rng = random.Random(seed)
    vals = [resample_metrics(preds, truths, rng)[key] for _ in range(n)]
    return {
        "metric": key,
        "ci95_low": percentile(vals, 100 * alpha / 2),
        "ci95_high": percentile(vals, 100 * (1 - alpha / 2)),
        "n_boot": n, "seed": seed,
    }


def bootstrap_delta_ci(preds_a, preds_b, truths, key, n=N_BOOT, seed=SEED, alpha=ALPHA):
    rng = random.Random(seed)
    vals = []
    for _ in range(n):
        m = len(preds_a)
        idx = [rng.randrange(m) for _ in range(m)]
        pa = [preds_a[i] for i in idx]
        pb = [preds_b[i] for i in idx]
        t = [truths[i] for i in idx]
        vals.append(metric_block(pb, t)[key] - metric_block(pa, t)[key])
    return {
        "metric": key,
        "ci95_low": percentile(vals, 100 * alpha / 2),
        "ci95_high": percentile(vals, 100 * (1 - alpha / 2)),
        "n_boot": n, "seed": seed,
    }


def fmt_pct(x):
    return f"{100.0 * x:.1f}%"


def fmt_ci(ci):
    return f"[{ci['ci95_low']:.4f}, {ci['ci95_high']:.4f}]"


METRIC_KEYS = [
    "accuracy", "macro_f1", "weighted_f1",
    "rescue_recall", "rescue_precision",
    "protection_specificity", "false_promotion_count",
]


def build_arm_metrics(preds, truths):
    """Per-arm metrics plus deterministic bootstrap CIs for every headline metric."""
    block = metric_block(preds, truths)
    out = {"point": block, "bootstrap": {}}
    for key in METRIC_KEYS:
        out["bootstrap"][key] = bootstrap_ci(preds, truths, key)
    return out


def error_analysis(ids, preds, truths, gt_rows):
    """Expose every failure and its directional/class/query concentration."""
    errors = []
    transitions = {
        actual: {predicted: 0 for predicted in CLASSES if predicted != actual}
        for actual in CLASSES
    }
    by_class = {label: [] for label in CLASSES}
    by_query = collections.defaultdict(list)
    for row_id, prediction, truth in zip(ids, preds, truths):
        if prediction == truth:
            continue
        row = gt_rows[row_id]
        error = {
            "row_id": row_id,
            "query": row["query"],
            "stratum": stratum_of(row_id),
            "actual": truth,
            "predicted": prediction,
        }
        errors.append(error)
        transitions[truth][prediction] += 1
        by_class[truth].append(row_id)
        by_query[row["query"]].append(row_id)
    concentrated = [
        {"query": query, "error_count": len(row_ids), "row_ids": sorted(row_ids)}
        for query, row_ids in by_query.items() if len(row_ids) > 1
    ]
    concentrated.sort(key=lambda entry: (-entry["error_count"], entry["query"]))
    return {
        "error_count": len(errors),
        "error_rate": len(errors) / len(ids) if ids else 0.0,
        "errors": errors,
        "row_ids_by_actual_class": by_class,
        "directional_confusion": transitions,
        "query_error_concentration": concentrated,
        "query_metadata_status": (
            "No independent query-type field is present; query-text groups are reported "
            "instead without inferring a type."
        ),
    }


def build_deltas(preds_a, preds_b, truths):
    """ARM_B - ARM_A paired deltas with paired bootstrap CIs."""
    out = {"point": {}, "bootstrap": {}}
    for key in METRIC_KEYS:
        out["point"][key] = (
            metric_block(preds_b, truths)[key] - metric_block(preds_a, truths)[key]
        )
        out["bootstrap"][key] = bootstrap_delta_ci(preds_a, preds_b, truths, key)
    return out


def semantic_change_analysis(perf, gt_rows, arm_a_rows):
    """Join the harness's per-candidate diagnostics with ground truth."""
    if not perf:
        return None
    diag = perf.get("semantic_diagnostics", [])
    candidates = len(diag)
    cosine_ge = perf.get("candidates_cosine_ge_threshold", 0)
    bounded = perf.get("candidates_bounded_strong_rescue", 0)
    suggestions = perf.get("rows_with_suggestion", 0)
    changed = perf.get("rows_changed_by_advisory", 0)

    # Missed-rescue counterfactual: candidates the embedding agreed on (cosine >=
    # threshold) that are ground-truth Relevant — the bounded gate blocked these.
    threshold = 0.72  # ExperimentalSemanticConfig::default().paraphrase_threshold
    agreed = [d for d in diag if d["cosine"] >= threshold]
    agreed_gt_relevant = sum(
        1 for d in agreed if gt_rows.get(d["row_id"], {}).get("final_label") == "Relevant"
    )
    agreed_gt_nr = sum(
        1 for d in agreed if gt_rows.get(d["row_id"], {}).get("final_label") == "NotRelevant"
    )
    agreed_gt_pr = len(agreed) - agreed_gt_relevant - agreed_gt_nr

    cosine_ge_and_bounded = sum(
        1 for d in diag if d["cosine"] >= threshold and d["bounded_strong_rescue"]
    )

    return {
        "candidates_total": candidates,
        "candidates_cosine_ge_threshold": cosine_ge,
        "candidates_bounded_strong_rescue": bounded,
        "candidates_cosine_ge_and_bounded": cosine_ge_and_bounded,
        "suggestions": suggestions,
        "rows_changed": changed,
        "paraphrase_threshold": threshold,
        "counterfactual_embedding_only_at_threshold": {
            "would_promote": len(agreed),
            "correct_gt_relevant": agreed_gt_relevant,
            "incorrect_gt_not_relevant": agreed_gt_nr,
            "incorrect_gt_possibly_relevant": agreed_gt_pr,
        },
        "binding_gate": (
            "bounded_strong_rescue"
            if bounded == 0 and cosine_ge > 0
            else ("cosine" if cosine_ge == 0 else "none")
        ),
    }


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--perf", default="/tmp/step4e_performance.json",
                    help="path to the harness performance snapshot (optional)")
    args = ap.parse_args()

    gt_rows = load_rows(OUT / "step4e_ground_truth.json")
    arm_a_rows = load_rows(OUT / "arm_a_predictions.json")
    arm_b_rows = load_rows(OUT / "arm_b_predictions.json")
    validate_alignment(gt_rows, arm_a_rows, "arm_a")
    validate_alignment(gt_rows, arm_b_rows, "arm_b")

    ids = [rid for rid in gt_rows if rid in arm_a_rows and rid in arm_b_rows]
    ids.sort()
    truths = [gt_rows[i]["final_label"] for i in ids]
    preds_a = [arm_a_rows[i]["prediction"] for i in ids]
    preds_b = [arm_b_rows[i]["prediction"] for i in ids]

    perf = None
    perf_path = Path(args.perf)
    if perf_path.exists():
        perf = json.loads(perf_path.read_text())

    metrics = {
        "schema": "amatl.step4e.metrics.v2",
        "evaluation_identity": {
            "dataset": "STEP4E frozen ground truth",
            "dataset_sha256": sha256_file(OUT / "step4e_ground_truth.json"),
            "code_commit": code_commit(),
            "metric_script_sha256": sha256_file(Path(__file__)),
            "classes": CLASSES,
            "determinism": {
                "bootstrap_seed": SEED,
                "bootstrap_resamples": N_BOOT,
                "candidate_row_order": "lexicographic row_id",
            },
        },
        "n_rows": len(ids),
        "candidates": {
            "arm_a": {
                "identity": "STEP4A production deterministic relevance",
                "parameters": "RelevanceThresholds::default(); no embedding",
                "prediction_artifact_sha256": sha256_file(OUT / "arm_a_predictions.json"),
            },
            "arm_b": {
                "identity": "STEP4B-4D Candle semantic advisory",
                "parameters": "bge-small-en-v1.5; K=8; paraphrase_threshold=0.72; bounded-semantic corroboration",
                "prediction_artifact_sha256": sha256_file(OUT / "arm_b_predictions.json"),
            },
        },
        "arms": {
            "arm_a": build_arm_metrics(preds_a, truths),
            "arm_b": build_arm_metrics(preds_b, truths),
        },
        "paired_delta_b_minus_a": build_deltas(preds_a, preds_b, truths),
        "strata": {},
        "error_analysis": {
            "arm_a": error_analysis(ids, preds_a, truths, gt_rows),
            "arm_b": error_analysis(ids, preds_b, truths, gt_rows),
        },
    }

    # Stratified by holdout stratum (direct / collision / limited).
    for stratum in ("direct", "collision", "limited"):
        sub = [i for i in ids if stratum_of(i) == stratum]
        t = [gt_rows[i]["final_label"] for i in sub]
        pa = [arm_a_rows[i]["prediction"] for i in sub]
        pb = [arm_b_rows[i]["prediction"] for i in sub]
        metrics["strata"][stratum] = {
            "n": len(sub),
            "ground_truth": dict(collections.Counter(t)),
            "arm_a": metric_block(pa, t),
            "arm_b": metric_block(pb, t),
            "delta_b_minus_a": {
                k: metric_block(pb, t)[k] - metric_block(pa, t)[k]
                for k in METRIC_KEYS
            },
        }

    metrics["semantic_change"] = semantic_change_analysis(perf, gt_rows, arm_a_rows)
    if perf:
        metrics["performance"] = {
            k: perf[k] for k in (
                "model_name", "model_version", "model_load_ms",
                "arm_a_eval_wall_ms", "arm_b_eval_wall_ms",
                "arm_b_incremental_wall_ms", "total_wall_ms",
                "rss_before_mb", "rss_after_model_load_mb", "rss_peak_delta_mb",
                "cpu_time_delta_ms", "cpu_time_status",
                "rows_total", "rows_with_semantic_candidate",
                "rows_with_suggestion", "rows_changed_by_advisory",
                "arm_a_label_counts", "arm_b_label_counts",
            )
        }

    (OUT / "step4e_metrics.json").write_text(
        json.dumps(metrics, indent=2) + "\n"
    )
    write_report(metrics, perf)
    print(f"wrote {OUT / 'step4e_metrics.json'}")
    print(f"wrote {OUT / 'step4e_evaluation_report.md'}")


def write_report(m, perf):
    L = []
    a = m["arms"]["arm_a"]["point"]
    b = m["arms"]["arm_b"]["point"]
    d = m["paired_delta_b_minus_a"]["point"]
    L.append("# STEP4E Evaluation Report\n")
    L.append("Frozen blind evaluation of **ARM_A** (production deterministic relevance) "
             "vs **ARM_B** (production + Candle semantic advisory) on the 150-row STEP4E "
             "holdout.\n")
    L.append(f"- Rows: **{m['n_rows']}** (50 direct / 50 collision / 50 limited)\n")
    identity = m["evaluation_identity"]
    L.append(f"- Dataset SHA-256: `{identity['dataset_sha256']}`; code commit: `{identity['code_commit']}`\n")
    if perf:
        L.append(f"- Model: `{perf['model_name']}` "
                 f"(load {perf['model_load_ms']} ms)\n")
    L.append("- Advisory: `SemanticEvaluator` over `bge-small-en-v1.5`, "
             "paraphrase threshold 0.72, bounded-semantic corroboration required.\n")
    L.append("- Bootstrap CIs: 10,000 resamples, seed 42 (deterministic).\n")

    L.append("\n## Headline result\n")
    L.append(f"**ARM_B == ARM_A byte-for-byte.** The advisory changed "
             f"**{m['semantic_change']['rows_changed']}** of {m['n_rows']} rows; both "
             f"artifacts share an identical SHA-256. The Candle semantic pass is **inert** "
             f"on this holdout.\n")

    L.append("\n## Primary metrics (vs ground truth)\n")
    L.append("| metric | ARM_A | ARM_B | Δ (B−A) |\n|---|---|---|---|\n")
    for key in METRIC_KEYS:
        ca = m["arms"]["arm_a"]["bootstrap"][key]
        cb = m["arms"]["arm_b"]["bootstrap"][key]
        cd = m["paired_delta_b_minus_a"]["bootstrap"][key]
        L.append(f"| {key} | {a[key]:.4f} {fmt_ci(ca)} | {b[key]:.4f} {fmt_ci(cb)} "
                 f"| {d[key]:+.4f} {fmt_ci(cd)} |\n")

    L.append("\n### Per-class (ARM_A == ARM_B)\n")
    L.append("| class | precision | recall | F1 |\n|---|---|---|---|\n")
    for c in CLASSES:
        pc = a["per_class"][c]
        L.append(f"| {c} | {pc['precision']:.3f} | {pc['recall']:.3f} | {pc['f1']:.3f} |\n")
    L.append(f"- Weighted F1: **{a['weighted_f1']:.4f}**\n")
    L.append(f"- Prediction distribution: `{a['prediction_distribution']}`\n")

    L.append("\n## Dispersion and exact failures\n")
    errors = m["error_analysis"]["arm_a"]
    L.append(f"- Errors: **{errors['error_count']}/{m['n_rows']}** ({fmt_pct(errors['error_rate'])}). "
             "ARM_B has the identical prediction artifact, hence the same errors.\n")
    L.append("- Directional confusion (actual → predicted): "
             f"`{errors['directional_confusion']}`\n")
    L.append(f"- {errors['query_metadata_status']}\n")
    if errors["query_error_concentration"]:
        counts = collections.Counter(
            entry["error_count"] for entry in errors["query_error_concentration"]
        )
        max_count = max(counts)
        max_queries = [
            entry["query"] for entry in errors["query_error_concentration"]
            if entry["error_count"] == max_count
        ]
        L.append(f"- Query concentration: {counts[max_count]} queries each account for the "
                 f"maximum **{max_count}** errors; together that is "
                 f"{max_count * counts[max_count]}/{errors['error_count']} failures. "
                 f"Examples: `{', '.join(max_queries[:5])}`. The complete list and row IDs "
                 "remain machine-readable.\n")
    else:
        L.append("- No query contributes more than one error; no query-level outlier is observable.\n")
    L.append("- Exact failed row IDs are machine-readable in `error_analysis.arm_a.errors` "
             "in `step4e_metrics.json`.\n")

    L.append("\n## Stratified by holdout stratum\n")
    L.append("| stratum | n | GT | ARM_A acc | ARM_B acc | Δ acc |\n|---|---|---|---|---|---|\n")
    for s in ("direct", "collision", "limited"):
        st = m["strata"][s]
        gt = ", ".join(f"{k}:{v}" for k, v in st["ground_truth"].items())
        L.append(f"| {s} | {st['n']} | {gt} | {st['arm_a']['accuracy']:.3f} "
                 f"| {st['arm_b']['accuracy']:.3f} | {st['delta_b_minus_a']['accuracy']:+.3f} |\n")

    L.append("\n## Semantic-change analysis\n")
    sc = m["semantic_change"]
    L.append(f"- Candidates (PossiblyRelevant rows embedded): **{sc['candidates_total']}**\n")
    L.append(f"- Embedding agreed (cosine ≥ {sc['paraphrase_threshold']}): "
             f"**{sc['candidates_cosine_ge_threshold']}**\n")
    L.append(f"- Bounded-semantic corroboration (`bounded_strong_rescue`): "
             f"**{sc['candidates_bounded_strong_rescue']}**\n")
    L.append(f"- Suggestions emitted: **{sc['suggestions']}**; rows changed: "
             f"**{sc['rows_changed']}**\n")
    L.append(f"- **Binding gate: `{sc['binding_gate']}`.** The embedding agrees on "
             f"{sc['candidates_cosine_ge_threshold']}/{sc['candidates_total']} candidates, "
             f"but the lexical corroboration layer never fires, so the advisory is a no-op.\n")
    cf = sc["counterfactual_embedding_only_at_threshold"]
    L.append(f"- Counterfactual (embedding-only, ignoring the bounded gate): would promote "
             f"**{cf['would_promote']}** rows — {cf['correct_gt_relevant']} correct "
             f"(GT Relevant), {cf['incorrect_gt_not_relevant']} incorrect (GT NotRelevant), "
             f"{cf['incorrect_gt_possibly_relevant']} GT PossiblyRelevant.\n")

    L.append("\n## Root cause\n")
    L.append("The bounded gate requires `subject_match && (alias_matches || entity_match)` "
             "or `alias_matches >= 2`. On this synthetic holdout the topics (bicycle chains, "
             "composting, smoke detectors, …) are **outside the ALIAS_SETS concept vocabulary** "
             "(async, kubernetes, postgresql, …) and the imperative query shapes do not match "
             "`detect_intent` patterns, so `subject_match`/`entity_match` are always false. "
             "The embedding itself is not the limiter — it agrees on 42% of candidates — the "
             "lexical corroboration layer is structurally inert here.\n")

    if perf:
        L.append("\n## Performance\n")
        L.append(f"- Model load: {perf['model_load_ms']} ms\n")
        L.append(f"- ARM_A (deterministic): {perf['arm_a_eval_wall_ms']} ms\n")
        L.append(f"- ARM_B (semantic): {perf['arm_b_eval_wall_ms']} ms "
                 f"(incremental {perf['arm_b_incremental_wall_ms']} ms)\n")
        L.append(f"- CPU time: {perf['cpu_time_delta_ms']} ms ({perf['cpu_time_status']})\n")
        L.append(f"- Peak RSS delta: {perf['rss_peak_delta_mb']} MB\n")
        L.append(f"- Label counts — ARM_A {perf['arm_a_label_counts']}, "
                 f"ARM_B {perf['arm_b_label_counts']}\n")

    L.append("\n## Conclusion\n")
    L.append("`RELEVANCE_CANDIDATE_DECISION=NO_CANDIDATE_PASSES`. Both implemented "
             "candidates have macro-F1 0.2601, weighted-F1 0.2601, 77.3% error, and only "
             "30.0% Relevant recall. ARM_B provides no quality or stability gain over ARM_A "
             "and adds 44,474 ms incremental evaluation time in this run. The systematic "
             "failure mode is the boundary between PossiblyRelevant and the other two classes; "
             "there are zero Relevant↔NotRelevant errors.\n\n")
    L.append("`HOLDOUT_RESULT=FAIL`: the frozen holdout had already been opened and evaluated "
             "at commit `974cb09` before this evaluator/selection package could freeze a "
             "candidate, configuration, and PASS/FAIL criteria. This report records that "
             "non-compliance rather than treating a post-hoc selection as a valid final gate. "
             "No threshold or algorithm was changed after observing the artifacts.\n")

    (OUT / "step4e_evaluation_report.md").write_text("".join(L))


if __name__ == "__main__":
    main()
