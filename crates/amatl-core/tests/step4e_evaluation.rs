//! STEP4E — frozen blind evaluation harness.
//!
//! Compares the production deterministic relevance path (**ARM_A**) against
//! production + the STEP 4D Candle semantic advisory (**ARM_B**) over the
//! frozen 150-row STEP4E hold-out (`bge-small-en-v1.5` embeddings).
//!
//! This file is a *prediction generator only* and is deliberately **blind**:
//! [`GroundTruthRow`] declares just the production-facing fields the frozen
//! corpus guarantees (`row_id`, `query`, `title`, `snippet`, `canonical_url`).
//! The label columns that exist in the corpus file (`final_label`, `label_a`,
//! `label_b`, `final_label_source`) are never declared, so serde never reads
//! them into the predictor — a label cannot leak into a prediction. Labels are
//! consumed later, by the offline metrics script, only after the predictions
//! have been frozen and committed.
//!
//! * **ARM_A** — the exact production deterministic relevance path:
//!   [`assess_result`] with [`RelevanceThresholds::default()`] (no embedding,
//!   no semantic promotion).
//! * **ARM_B** — the same production path **plus** the exact STEP 4D advisory:
//!   [`SemanticEvaluator::evaluate_search`] with
//!   [`ExperimentalSemanticConfig::default()`] (`K=8`, paraphrase threshold
//!   `0.72`, advisory-only). The advisory `suggested` promotion is applied when
//!   present; otherwise the production classification passes through unchanged.
//!   Rows that share a `query` are evaluated as one search unit (query embedded
//!   exactly once), matching the production seam's execution model.
//!
//! Outputs (immutable prediction artifacts, committed):
//! * `docs/evaluation/step4e/arm_a_predictions.json`
//! * `docs/evaluation/step4e/arm_b_predictions.json`
//!
//! Run-time performance snapshot (not committed): `/tmp/step4e_performance.json`.
//!
//! Compiled only under `--features experimental-local-embeddings` and gated on
//! `AMATL_TEST_MODEL_DIR` (must point at the hash-pinned `bge-small-en-v1.5`
//! package, e.g. `/home/panda/.local/share/amatl/models/bge-small-en-v1.5`).
//! Run:
//!
//! ```text
//! AMATL_TEST_MODEL_DIR=/path/to/bge-small-en-v1.5 cargo test -p amatl-core \
//!   --features experimental-local-embeddings \
//!   --test step4e_evaluation -- --ignored --nocapture --test-threads=1
//! ```

#![cfg(feature = "experimental-local-embeddings")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use amatl_core::experimental_embeddings::model_config::PINNED_MODEL_VERSION;
use amatl_core::experimental_embeddings::{
    EmbeddingBackend, ExperimentalSemanticConfig, SemanticEvaluator,
};
use amatl_core::{
    assess_result, parse_query, CanonicalUrl, DeduplicatedResult, DuplicateStatus, OriginalUrl,
    Rank, RelevanceClassification, RelevanceThresholds, ResultType, SCHEMA_VERSION,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Frozen sha256 of `docs/evaluation/step4e/step4e_ground_truth.json`.
const GROUND_TRUTH_SHA256: &str =
    "db7eea3026f05adc0b59c6868e78c710928163912e009456cef43785169dc198";

const STEP4E_DIR: &str = "../../docs/evaluation/step4e/";
const GROUND_TRUTH_FILE: &str = "step4e_ground_truth.json";
const ARM_A_ARTIFACT: &str = "arm_a_predictions.json";
const ARM_B_ARTIFACT: &str = "arm_b_predictions.json";
const PERF_OUT: &str = "/tmp/step4e_performance.json";

const MODEL_NAME: &str = "BAAI/bge-small-en-v1.5";
const EXPECTED_ROWS: usize = 150;
const EXPECTED_PER_STRATUM: usize = 50;
/// The production relevance path consumes exactly these frozen fields; the
/// corpus' `domain` / `provider_or_source` / `source_capture_id_or_provenance`
/// columns are provenance metadata and are deliberately not prediction inputs.
const PREDICTOR_INPUTS: &[&str] = &["query", "title", "snippet", "canonical_url"];

/// A hold-out row as read by the predictor. **Only production-facing fields are
/// declared** — see the module docs for the blind-evaluation contract. Serde
/// ignores the corpus' extra label columns, which is exactly the property this
/// harness relies on to stay label-free.
#[derive(Debug, Deserialize)]
struct GroundTruthRow {
    row_id: String,
    query: String,
    title: String,
    snippet: String,
    canonical_url: String,
}

#[derive(Debug, Deserialize)]
struct GroundTruth {
    rows: Vec<GroundTruthRow>,
}

#[derive(Debug, Serialize)]
struct PredictionRow {
    row_id: String,
    prediction: String,
}

#[derive(Debug, Serialize)]
struct PredictionArtifact {
    schema: String,
    model_version: String,
    prediction_key: String,
    predictor_inputs: Vec<String>,
    rows: Vec<PredictionRow>,
}

#[derive(Debug, Serialize)]
struct SemanticRowDiagnostic {
    row_id: String,
    production: String,
    cosine: f32,
    bounded_strong_rescue: bool,
    suggested: Option<String>,
}

#[derive(Debug, Serialize)]
struct PerformanceSnapshot {
    model_name: String,
    model_version: String,
    model_load_ms: u64,
    arm_a_eval_wall_ms: u64,
    arm_b_eval_wall_ms: u64,
    arm_b_incremental_wall_ms: u64,
    total_wall_ms: u64,
    rss_before_mb: f64,
    rss_after_model_load_mb: f64,
    rss_peak_delta_mb: f64,
    cpu_time_delta_ms: f64,
    cpu_time_status: String,
    rows_total: usize,
    rows_with_semantic_candidate: usize,
    rows_with_suggestion: usize,
    rows_changed_by_advisory: usize,
    arm_a_label_counts: BTreeMap<String, usize>,
    arm_b_label_counts: BTreeMap<String, usize>,
    candidates_cosine_ge_threshold: usize,
    candidates_bounded_strong_rescue: usize,
    semantic_diagnostics: Vec<SemanticRowDiagnostic>,
}

fn step4e_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(STEP4E_DIR)
}

fn model_dir() -> Option<PathBuf> {
    std::env::var_os("AMATL_TEST_MODEL_DIR").map(PathBuf::from)
}

fn build_result(url: &str, title: &str, snippet: &str) -> DeduplicatedResult {
    let parsed = url::Url::parse(url).expect("valid canonical URL");
    let mut provider_ranks = BTreeMap::new();
    // The frozen hold-out carries no provider rank; we must not fabricate one,
    // so the rank signal is explicitly absent (None) rather than invented.
    provider_ranks.insert("corpus".to_string(), None::<Rank>);
    DeduplicatedResult {
        schema_version: SCHEMA_VERSION.into(),
        title: Some(title.to_string()),
        original_url: OriginalUrl(parsed.clone()),
        canonical_url: CanonicalUrl(parsed.clone()),
        original_urls: vec![OriginalUrl(parsed)],
        providers: vec!["corpus".to_string()],
        representative_provider: "corpus".into(),
        provider_ranks,
        snippet: Some(snippet.to_string()),
        alternate_snippets: vec![],
        result_type: ResultType::Organic,
        published_at: None,
        author: None,
        language: None,
        file_type: None,
        thumbnail: None,
        metadata: BTreeMap::new(),
        observed_dates: vec![],
        duplicate_status: DuplicateStatus::Distinct,
        merge_reason: None,
        possible_duplicate_with: vec![],
    }
}

fn label_string(c: RelevanceClassification) -> &'static str {
    match c {
        RelevanceClassification::Relevant => "Relevant",
        RelevanceClassification::PossiblyRelevant => "PossiblyRelevant",
        RelevanceClassification::NotRelevant => "NotRelevant",
        RelevanceClassification::Unknown => "Unknown",
    }
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

fn rss_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            return rest.split_whitespace().next()?.parse().ok();
        }
    }
    None
}

fn peak_rss_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmHWM:") {
            return rest.split_whitespace().next()?.parse().ok();
        }
    }
    None
}

/// Process CPU time in milliseconds from `/proc/self/stat` (fields 14/15,
/// `utime`/`stime` in clock ticks). `comm` is the only parenthesised field, so
/// everything after the last `)` starts at field 3; utime is then index 11 and
/// stime index 12.
fn cpu_time_ms() -> f64 {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let rest = &stat[stat.rfind(')').map(|i| i + 1).unwrap_or(0)..];
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let utime: f64 = fields.get(11).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let stime: f64 = fields.get(12).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    const TICKS_PER_SEC: f64 = 100.0;
    (utime + stime) / TICKS_PER_SEC * 1000.0
}

fn kb_to_mb(kb: u64) -> f64 {
    kb as f64 / 1024.0
}

fn count_labels(rows: &[PredictionRow]) -> BTreeMap<String, usize> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for row in rows {
        *counts.entry(row.prediction.clone()).or_insert(0) += 1;
    }
    counts
}

/// Load the frozen ground-truth corpus, verifying its identity (sha256) and
/// shape (150 rows, unique ids, 50 per stratum). Only production fields are
/// deserialised; labels are never touched.
fn load_ground_truth() -> GroundTruth {
    let path = step4e_dir().join(GROUND_TRUTH_FILE);
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read frozen ground truth {}: {e}", path.display()));
    let digest = sha256_hex(raw.as_bytes());
    assert_eq!(
        digest, GROUND_TRUTH_SHA256,
        "frozen ground truth changed — sha256 {digest} != {GROUND_TRUTH_SHA256}"
    );
    let ground: GroundTruth = serde_json::from_str(&raw).expect("ground truth parses");
    assert_eq!(ground.rows.len(), EXPECTED_ROWS, "row count");
    let mut seen = std::collections::HashSet::new();
    for row in &ground.rows {
        assert!(
            seen.insert(row.row_id.clone()),
            "duplicate row_id {}",
            row.row_id
        );
    }
    let mut strata: BTreeMap<&str, usize> = BTreeMap::new();
    for row in &ground.rows {
        let suffix = row
            .row_id
            .rsplit('-')
            .next()
            .unwrap_or_default()
            .to_string();
        *strata
            .entry(Box::leak(suffix.into_boxed_str()))
            .or_insert(0) += 1;
    }
    for (stratum, count) in &strata {
        assert_eq!(
            *count, EXPECTED_PER_STRATUM,
            "stratum '{stratum}' must have {EXPECTED_PER_STRATUM} rows"
        );
    }
    assert_eq!(strata.len(), 3, "exactly direct/collision/limited strata");
    ground
}

/// Write `bytes` to `path` if the artifact does not yet exist; if it already
/// exists, verify it is byte-for-byte identical (frozen immutability). Returns
/// the sha256 of the (written or verified) content.
fn write_or_verify(path: &Path, bytes: &str) -> String {
    let digest = sha256_hex(bytes.as_bytes());
    if path.exists() {
        let existing = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("read existing artifact {}: {e}", path.display()));
        assert_eq!(
            existing,
            bytes,
            "artifact {} changed — immutability violation",
            path.display()
        );
        eprintln!("ARTIFACT_VERIFIED {} ({digest})", path.display());
    } else {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .unwrap_or_else(|e| panic!("create {}: {e}", parent.display()));
        }
        std::fs::write(path, bytes)
            .unwrap_or_else(|e| panic!("write artifact {}: {e}", path.display()));
        eprintln!("ARTIFACT_WRITTEN {} ({digest})", path.display());
    }
    digest
}

#[test]
#[ignore = "requires AMATL_TEST_MODEL_DIR to point at the verified bge-small-en-v1.5 package"]
fn step4e_evaluation() {
    let started = Instant::now();

    // 1. Frozen inputs — identity, shape, strata. Labels are never read.
    let ground = load_ground_truth();
    let rows = &ground.rows;

    // 2. Model package (hash-pinned) through the seam's convenience loader.
    let model_dir = model_dir()
        .expect("AMATL_TEST_MODEL_DIR must point at the verified bge-small-en-v1.5 package");
    let rss_before = rss_kb();
    let load_started = Instant::now();
    let backend = SemanticEvaluator::load_backend(&model_dir, Some(PINNED_MODEL_VERSION))
        .expect("hash-pinned model package must load — no fallback allowed for STEP4E");
    let model_load_ms = load_started.elapsed().as_millis() as u64;
    assert_eq!(
        backend.dimension(),
        384,
        "bge-small-en-v1.5 embedding dimension"
    );
    let rss_after_load = peak_rss_kb();

    // 3. ARM_A — the exact production deterministic relevance path, timed.
    let cpu_before = cpu_time_ms();
    let arm_a_started = Instant::now();
    let mut arm_a: Vec<RelevanceClassification> = Vec::with_capacity(rows.len());
    for row in rows {
        let q = parse_query(row.query.clone()).expect("frozen query parses");
        let result = build_result(&row.canonical_url, &row.title, &row.snippet);
        let assessment = assess_result(&q, &result, &RelevanceThresholds::default());
        arm_a.push(assessment.classification);
    }
    let arm_a_eval_ms = arm_a_started.elapsed().as_millis() as u64;

    // 4. ARM_B — production + STEP 4D advisory, timed. Rows sharing a query
    //    form one search unit (the seam's execution model: query embedded
    //    exactly once, documents in one batch).
    let config = ExperimentalSemanticConfig::default();
    let paraphrase_threshold = config.paraphrase_threshold;
    let evaluator = SemanticEvaluator::new(&backend, config);
    let mut groups: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, row) in rows.iter().enumerate() {
        groups.entry(row.query.as_str()).or_default().push(i);
    }
    let arm_b_started = Instant::now();
    let mut arm_b: Vec<RelevanceClassification> = arm_a.clone();
    let mut rows_candidate = 0usize;
    let mut rows_suggestion = 0usize;
    let mut rows_changed = 0usize;
    let mut cosine_ge_threshold = 0usize;
    let mut bounded_rescue_count = 0usize;
    let mut diagnostics: Vec<SemanticRowDiagnostic> = Vec::new();
    for (query_text, indices) in &groups {
        let q = parse_query(query_text.to_string()).expect("frozen query parses");
        let results: Vec<DeduplicatedResult> = indices
            .iter()
            .map(|&i| build_result(&rows[i].canonical_url, &rows[i].title, &rows[i].snippet))
            .collect();
        let outcome = evaluator.evaluate_search(&q, &results);
        assert!(
            outcome.backend_fallback.is_none(),
            "ARM_B must run the real embedding pass, never a bounded-semantic fallback"
        );
        assert_eq!(
            outcome.query_embed_count,
            if outcome.semantic_candidates > 0 {
                1
            } else {
                0
            },
            "query embedded exactly once per search unit"
        );
        rows_candidate += outcome.semantic_candidates;
        for evaluation in outcome.evaluations {
            let row_index = indices[evaluation.result_index];
            diagnostics.push(SemanticRowDiagnostic {
                row_id: rows[row_index].row_id.clone(),
                production: label_string(evaluation.production.classification).to_string(),
                cosine: evaluation.rescore.cosine,
                bounded_strong_rescue: evaluation.rescore.bounded_strong_rescue,
                suggested: evaluation.suggested.map(|l| label_string(l).to_string()),
            });
            if evaluation.rescore.cosine >= paraphrase_threshold {
                cosine_ge_threshold += 1;
            }
            if evaluation.rescore.bounded_strong_rescue {
                bounded_rescue_count += 1;
            }
            if let Some(suggested) = evaluation.suggested {
                rows_suggestion += 1;
                let previous = arm_b[row_index];
                if suggested != previous {
                    rows_changed += 1;
                }
                arm_b[row_index] = suggested;
            }
        }
    }
    let arm_b_eval_ms = arm_b_started.elapsed().as_millis() as u64;
    let arm_b_incremental_ms = arm_b_eval_ms.saturating_sub(arm_a_eval_ms);
    let cpu_after = cpu_time_ms();
    let cpu_delta_ms = cpu_after - cpu_before;
    let peak_rss_after = peak_rss_kb();

    // 5. Build frozen prediction artifacts (canonical object form, row order
    //    preserved from the frozen corpus).
    let make_rows = |labels: &[RelevanceClassification]| -> Vec<PredictionRow> {
        rows.iter()
            .zip(labels)
            .map(|(r, c)| PredictionRow {
                row_id: r.row_id.clone(),
                prediction: label_string(*c).to_string(),
            })
            .collect()
    };
    let a_rows = make_rows(&arm_a);
    let b_rows = make_rows(&arm_b);
    let a_artifact = PredictionArtifact {
        schema: "step4e/predictions/v1".into(),
        model_version: PINNED_MODEL_VERSION.to_string(),
        prediction_key: "classification".into(),
        predictor_inputs: PREDICTOR_INPUTS.iter().map(|s| s.to_string()).collect(),
        rows: a_rows,
    };
    let b_artifact = PredictionArtifact {
        schema: "step4e/predictions/v1".into(),
        model_version: PINNED_MODEL_VERSION.to_string(),
        prediction_key: "classification".into(),
        predictor_inputs: PREDICTOR_INPUTS.iter().map(|s| s.to_string()).collect(),
        rows: b_rows,
    };
    let a_json = serde_json::to_string_pretty(&a_artifact).expect("serialize ARM_A") + "\n";
    let b_json = serde_json::to_string_pretty(&b_artifact).expect("serialize ARM_B") + "\n";
    let _a_digest = write_or_verify(&step4e_dir().join(ARM_A_ARTIFACT), &a_json);
    let _b_digest = write_or_verify(&step4e_dir().join(ARM_B_ARTIFACT), &b_json);

    // 6. Performance snapshot (not committed — path lives in /tmp).
    let cpu_time_status = if cpu_delta_ms > 0.0 {
        "CPU_TIME_READABLE"
    } else {
        "CPU_TIME_UNAVAILABLE"
    };
    let perf = PerformanceSnapshot {
        model_name: MODEL_NAME.to_string(),
        model_version: PINNED_MODEL_VERSION.to_string(),
        model_load_ms,
        arm_a_eval_wall_ms: arm_a_eval_ms,
        arm_b_eval_wall_ms: arm_b_eval_ms,
        arm_b_incremental_wall_ms: arm_b_incremental_ms,
        total_wall_ms: started.elapsed().as_millis() as u64,
        rss_before_mb: rss_before.map(kb_to_mb).unwrap_or(-1.0),
        rss_after_model_load_mb: rss_after_load.map(kb_to_mb).unwrap_or(-1.0),
        rss_peak_delta_mb: peak_rss_after
            .zip(rss_before)
            .map(|(peak, base)| kb_to_mb(peak.saturating_sub(base)))
            .unwrap_or(-1.0),
        cpu_time_delta_ms: cpu_delta_ms,
        cpu_time_status: cpu_time_status.to_string(),
        rows_total: rows.len(),
        rows_with_semantic_candidate: rows_candidate,
        rows_with_suggestion: rows_suggestion,
        rows_changed_by_advisory: rows_changed,
        arm_a_label_counts: count_labels(&make_rows(&arm_a)),
        arm_b_label_counts: count_labels(&make_rows(&arm_b)),
        candidates_cosine_ge_threshold: cosine_ge_threshold,
        candidates_bounded_strong_rescue: bounded_rescue_count,
        semantic_diagnostics: diagnostics,
    };
    let perf_json = serde_json::to_string_pretty(&perf).expect("serialize perf") + "\n";
    std::fs::write(PERF_OUT, perf_json).expect("write /tmp perf snapshot");

    // 7. Summary line.
    eprintln!(
        "STEP4E summary: rows={} model_load_ms={model_load_ms} arm_a_ms={arm_a_eval_ms} \
         arm_b_ms={arm_b_eval_ms} incremental_ms={arm_b_incremental_ms} \
         cpu_delta_ms={cpu_delta_ms:.2} ({cpu_time_status}) candidates={rows_candidate} \
         suggestions={rows_suggestion} changed={rows_changed} \
         cosine_ge_threshold={cosine_ge_threshold} bounded_strong_rescue={bounded_rescue_count}",
        rows.len(),
    );
    eprintln!("STEP4E_ARM_A_LABELS={:?}", count_labels(&make_rows(&arm_a)));
    eprintln!("STEP4E_ARM_B_LABELS={:?}", count_labels(&make_rows(&arm_b)));
    assert_eq!(
        rows_changed,
        arm_b
            .iter()
            .zip(arm_a.iter())
            .filter(|(b, a)| b != a)
            .count()
    );
}
