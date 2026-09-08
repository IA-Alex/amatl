//! STEP 4D §6 — REAL integrated-execution latency: default bounded-semantic
//! path vs the optimized Candle pipeline with the model actually loaded.
//!
//! No FixtureBackend, no calibrated/simulated cost — every number here comes
//! from a real `bge-small-en-v1.5` forward pass on this machine.
//!
//! ```text
//! AMATL_TEST_MODEL_DIR=/path/to/pkg cargo test -p amatl-core \
//!   --features experimental-local-embeddings --release \
//!   --test experimental_candle_latency -- --ignored --nocapture --test-threads=1
//! ```

#![cfg(feature = "experimental-local-embeddings")]

use std::collections::BTreeMap;
use std::time::Instant;

use amatl_core::experimental_embeddings::{
    model_config::PINNED_MODEL_VERSION, ExperimentalSemanticConfig, SemanticEvaluator,
};
use amatl_core::{
    assess_result, parse_query, CanonicalUrl, DeduplicatedResult, DuplicateStatus, OriginalUrl,
    Rank, RelevanceThresholds, ResultType, SCHEMA_VERSION,
};

fn model_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("AMATL_TEST_MODEL_DIR").map(std::path::PathBuf::from)
}

fn result(i: usize, title: &str, snippet: &str) -> DeduplicatedResult {
    let parsed = url::Url::parse(&format!("https://r{i}.test/p")).unwrap();
    let mut provider_ranks = BTreeMap::new();
    provider_ranks.insert("corpus".to_string(), Rank::new((i as u32) + 1).ok());
    DeduplicatedResult {
        schema_version: SCHEMA_VERSION.into(),
        title: Some(title.into()),
        original_url: OriginalUrl(parsed.clone()),
        canonical_url: CanonicalUrl(parsed.clone()),
        original_urls: vec![OriginalUrl(parsed)],
        providers: vec!["corpus".to_string()],
        representative_provider: "corpus".into(),
        provider_ranks,
        snippet: Some(snippet.into()),
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

/// A result set of size `n`: a couple of strong ones, the rest in the
/// PossiblyRelevant ambiguous band so the pipeline actually does embedding work.
fn result_set(n: usize) -> Vec<DeduplicatedResult> {
    let mut v = vec![result(
        0,
        "How to prevent burnout at work",
        "practical steps to prevent workplace burnout at work",
    )];
    for i in 1..n {
        v.push(result(
            i,
            "Notes on burnout",
            "some thoughts about burnout and staying healthy in a demanding job",
        ));
    }
    v
}

fn pctl(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn p50_p95(mut xs: Vec<f64>) -> (f64, f64) {
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (pctl(&xs, 50.0), pctl(&xs, 95.0))
}

fn rss_mb() -> f64 {
    let statm = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
    let pages: f64 = statm
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);
    pages * 4096.0 / (1024.0 * 1024.0)
}

fn cpu_ms() -> f64 {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let after = stat.rsplit_once(')').map(|(_, r)| r).unwrap_or(&stat);
    let f: Vec<&str> = after.split_whitespace().collect();
    let u: f64 = f.get(11).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let s: f64 = f.get(12).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    (u + s) / 100.0 * 1000.0
}

#[test]
#[ignore = "needs AMATL_TEST_MODEL_DIR; perf-sensitive"]
fn real_e2e_latency_baseline_vs_candle() {
    let Some(dir) = model_dir() else {
        eprintln!("AMATL_TEST_MODEL_DIR unset — skipping");
        return;
    };
    let thresholds = RelevanceThresholds::default();
    let query = parse_query("how to prevent burnout at work".to_string()).unwrap();

    let rss_before = rss_mb();
    let load_start = Instant::now();
    let backend =
        SemanticEvaluator::load_backend(&dir, Some(PINNED_MODEL_VERSION)).expect("model loads");
    let model_load_ms = load_start.elapsed().as_secs_f64() * 1000.0;
    let rss_after_load = rss_mb();
    let evaluator = SemanticEvaluator::new(&backend, ExperimentalSemanticConfig::default());

    println!("\n=== STEP 4D REAL E2E LATENCY (per-search wall time, ms) ===");
    println!("MODEL_LOAD_TIME_MS={model_load_ms:.1}");
    println!(
        "RSS_DELTA_MB={:.1}  (before={:.1} after_load={:.1})",
        rss_after_load - rss_before,
        rss_before,
        rss_after_load
    );

    for &n in &[5usize, 10, 20] {
        let set = result_set(n);

        // A — default bounded-semantic path (production assess_result over all n).
        let mut base = Vec::new();
        for _ in 0..200 {
            let t = Instant::now();
            for r in &set {
                std::hint::black_box(assess_result(&query, r, &thresholds));
            }
            base.push(t.elapsed().as_secs_f64() * 1000.0);
        }

        // B — Candle pipeline (query embed once + batched doc embed of the
        // bounded candidate set + suggestions).
        // Warm.
        std::hint::black_box(evaluator.evaluate_search(&query, &set));
        let mut cand = Vec::new();
        let cpu0 = cpu_ms();
        let mut sem_candidates = 0usize;
        let mut q_embed = 0usize;
        let mut d_batches = 0usize;
        for _ in 0..12 {
            let t = Instant::now();
            let o = evaluator.evaluate_search(&query, &set);
            cand.push(t.elapsed().as_secs_f64() * 1000.0);
            sem_candidates = o.semantic_candidates;
            q_embed = o.query_embed_count;
            d_batches = o.document_embed_batches;
        }
        let cpu_delta = cpu_ms() - cpu0;

        let (b50, b95) = p50_p95(base);
        let (c50, c95) = p50_p95(cand);
        println!("\n--- {n} results/search ---");
        println!("RESULTS_TOTAL={n}");
        println!("SEMANTIC_CANDIDATES={sem_candidates}");
        println!("QUERY_EMBED_COUNT={q_embed}");
        println!("DOCUMENT_EMBED_BATCHES={d_batches}");
        assert_eq!(q_embed, 1, "invariant: query embedded exactly once");
        println!("BASELINE_P50_MS={b50:.3}  BASELINE_P95_MS={b95:.3}");
        println!("CANDLE_P50_MS={c50:.3}  CANDLE_P95_MS={c95:.3}");
        println!(
            "INCREMENTAL_P50_MS={:.3}  INCREMENTAL_P95_MS={:.3}",
            c50 - b50,
            c95 - b95
        );
        println!(
            "CPU_TIME_DELTA_MS={:.1}  over 12 candle searches",
            cpu_delta
        );
    }
    println!("\nRSS_AFTER_ALL_MB={:.1}", rss_mb());
}
