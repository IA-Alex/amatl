//! STEP 4C — real end-to-end search-path latency with vs without the
//! experimental local-embedding seam.
//!
//! Compiled only under `--features experimental-local-embeddings`. Ignored by
//! default (perf-sensitive); run explicitly:
//!
//! ```text
//! cargo test -p amatl-core --features experimental-local-embeddings \
//!   --test experimental_embedding_latency -- --ignored --nocapture --test-threads=1
//! ```
//!
//! ## What is measured
//!
//! * **A — baseline**: the *production* relevance path (`assess_result`, bounded
//!   -semantic layer on) over a post-dedupe result set of size 5 / 10 / 20,
//!   driven from the frozen `corpus.json` fixture. No provider network.
//! * **B — with Candle seam**: the same path, plus `evaluate_with_embeddings`
//!   for every result, using [`FixtureBackend`] calibrated at runtime so each
//!   embed call costs the **STEP 4B measured Candle CPU time**
//!   (`QUERY_EMBED_P50 = 15.0 ms`, `DOCUMENT_EMBED_P50 = 22.6 ms`,
//!   `docs/experiments/candle-musl-feasibility.md` §6). The fixture is a hash
//!   embedder, not a real model — only its *timing* stands in for Candle, so
//!   the delta this test reports is the seam's operational cost, machine
//!   -normalised.
//!
//! Every number printed comes from this executed run.

#![cfg(feature = "experimental-local-embeddings")]

use std::time::Instant;

use amatl_core::experimental_embeddings::{
    evaluate_with_embeddings, EmbeddingBackend, FixtureBackend,
};
use amatl_core::{
    assess_result, assess_semantics, parse_query, CanonicalUrl, DeduplicatedResult,
    DuplicateStatus, OriginalUrl, Rank, RelevanceThresholds, ResultType, SCHEMA_VERSION,
};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Deserialize)]
struct Corpus {
    samples: Vec<Sample>,
}

#[derive(Debug, Clone, Deserialize)]
struct Sample {
    query: String,
    title: Option<String>,
    snippet: Option<String>,
    url: String,
    provider_rank: Option<u32>,
}

const CORPUS_JSON: &str = include_str!("fixtures/relevance/corpus.json");

// STEP 4B measured Candle CPU cost per embed, glibc host, --release (§6).
const CANDLE_QUERY_EMBED_MS: f64 = 15.0;
const CANDLE_DOC_EMBED_MS: f64 = 22.6;

fn sample_to_result(s: &Sample) -> DeduplicatedResult {
    let parsed = url::Url::parse(&s.url).unwrap();
    let rank = s.provider_rank.and_then(|v| Rank::new(v).ok());
    let mut provider_ranks = BTreeMap::new();
    provider_ranks.insert("corpus".to_string(), rank);
    DeduplicatedResult {
        schema_version: SCHEMA_VERSION.into(),
        title: s.title.clone(),
        original_url: OriginalUrl(parsed.clone()),
        canonical_url: CanonicalUrl(parsed.clone()),
        original_urls: vec![OriginalUrl(parsed)],
        providers: vec!["corpus".to_string()],
        representative_provider: "corpus".into(),
        provider_ranks,
        snippet: s.snippet.clone(),
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

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn summarize(mut samples: Vec<f64>) -> (f64, f64) {
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (percentile(&samples, 50.0), percentile(&samples, 95.0))
}

/// Calibrate `FixtureBackend` `work_rounds` so one embed call takes ~`target_ms`
/// on *this* machine.
fn calibrate_rounds(target_ms: f64) -> u32 {
    // Measure cost of a known round count, scale linearly.
    let probe = FixtureBackend::default().with_work_rounds(2_000_000);
    let warmup = probe.embed_query("calibration warmup text");
    std::hint::black_box(&warmup);
    let start = Instant::now();
    for _ in 0..20 {
        let v = probe.embed_query("calibration probe text sample");
        std::hint::black_box(&v);
    }
    let per_call_ms = start.elapsed().as_secs_f64() * 1000.0 / 20.0;
    // per_call_ms corresponds to 2_000_000 rounds (+ fixed hashing overhead).
    let rounds = (2_000_000.0 * (target_ms / per_call_ms.max(0.001))).round();
    rounds.clamp(0.0, 500_000_000.0) as u32
}

fn rss_kb() -> u64 {
    let statm = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
    let pages: u64 = statm
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    pages * 4 // page size 4 KiB
}

struct Scenario {
    results: Vec<(amatl_core::model::Query, Vec<DeduplicatedResult>)>,
}

fn build_scenario(n_results: usize) -> Scenario {
    let corpus: Corpus = serde_json::from_str(CORPUS_JSON).unwrap();
    // Group samples by query, keep queries that have >= n_results rows.
    let mut by_query: BTreeMap<String, Vec<Sample>> = BTreeMap::new();
    for s in corpus.samples {
        by_query.entry(s.query.clone()).or_default().push(s);
    }
    let mut results = Vec::new();
    for (q, mut samples) in by_query {
        let query = match parse_query(q) {
            Ok(q) => q,
            Err(_) => continue,
        };
        // Skip queries that normalise to empty (e.g. fully-quoted phrases):
        // the seam short-circuits them to fallback with zero embed cost, which
        // would make the latency distribution bimodal and hide the real cost.
        if query.normalized_query.trim().is_empty() {
            continue;
        }
        // Repeat samples to reach n_results deterministically if a query is short.
        while samples.len() < n_results {
            let extra = samples[samples.len() % samples.len().max(1)].clone();
            samples.push(extra);
        }
        samples.truncate(n_results);
        let deduped: Vec<DeduplicatedResult> = samples.iter().map(sample_to_result).collect();
        results.push((query, deduped));
    }
    // Cap the number of distinct queries so the harness (real Candle-speed
    // embed cost per result) stays under ~15 s per scenario. Deterministic:
    // BTreeMap iteration order.
    results.truncate(8);
    Scenario { results }
}

fn run_baseline(scn: &Scenario, thresholds: &RelevanceThresholds) -> Vec<f64> {
    let mut per_search = Vec::new();
    for (query, deduped) in &scn.results {
        let start = Instant::now();
        for r in deduped {
            let a = assess_result(query, r, thresholds);
            std::hint::black_box(&a);
        }
        per_search.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    per_search
}

fn run_with_candle(
    scn: &Scenario,
    thresholds: &RelevanceThresholds,
    q_backend: &FixtureBackend,
    d_backend: &FixtureBackend,
) -> Vec<f64> {
    let mut per_search = Vec::new();
    for (query, deduped) in &scn.results {
        let start = Instant::now();
        // Query embedded once per search (low-risk optimisation from spec §5).
        std::hint::black_box(q_backend.embed_query(&query.normalized_query).ok());
        for r in deduped {
            let a = assess_result(query, r, thresholds);
            let bounded = assess_semantics(
                query,
                r.title.as_deref(),
                r.snippet.as_deref(),
                "",
                &Default::default(),
            );
            let text = format!(
                "{} {}",
                r.title.as_deref().unwrap_or(""),
                r.snippet.as_deref().unwrap_or("")
            );
            let rescore = evaluate_with_embeddings(query, &text, &bounded, d_backend);
            std::hint::black_box((&a, &rescore));
        }
        per_search.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    per_search
}

#[test]
#[ignore = "perf-sensitive; run explicitly"]
fn e2e_latency_candle_seam_vs_production() {
    let thresholds = RelevanceThresholds::default();

    let q_rounds = calibrate_rounds(CANDLE_QUERY_EMBED_MS);
    let d_rounds = calibrate_rounds(CANDLE_DOC_EMBED_MS);
    let q_backend = FixtureBackend::default().with_work_rounds(q_rounds);
    let d_backend = FixtureBackend::default().with_work_rounds(d_rounds);
    eprintln!("CALIBRATION: query_rounds={q_rounds} doc_rounds={d_rounds}");

    // Verify calibration hit the target.
    for (label, backend, target) in [
        ("query", &q_backend, CANDLE_QUERY_EMBED_MS),
        ("doc", &d_backend, CANDLE_DOC_EMBED_MS),
    ] {
        let start = Instant::now();
        for _ in 0..15 {
            let _ = std::hint::black_box(backend.embed_query("verify calibration text"));
        }
        let ms = start.elapsed().as_secs_f64() * 1000.0 / 15.0;
        eprintln!("CALIBRATION_CHECK {label}: measured {ms:.1} ms (target {target:.1} ms)");
    }

    let rss_before = rss_kb();

    println!("\n=== STEP 4C E2E LATENCY (per-search wall time, ms) ===");
    println!(
        "(baseline = production assess_result; candle = + evaluate_with_embeddings per result)\n"
    );

    let mut deltas = BTreeMap::new();
    // Few iterations: each "candle" iteration pays real Candle-speed embed cost
    // (~22.6 ms) for every result of every query. p50/p95 are taken over
    // iterations * queries per-search samples.
    let iterations = 6usize;

    for &n in &[5usize, 10, 20] {
        let scn = build_scenario(n);
        assert!(!scn.results.is_empty(), "scenario {n} has queries");

        // Warm.
        let _ = run_baseline(&scn, &thresholds);
        let _ = run_with_candle(&scn, &thresholds, &q_backend, &d_backend);

        let mut base = Vec::new();
        let mut cand = Vec::new();
        let cpu_base_start = cpu_time_ms();
        for _ in 0..(iterations * 30) {
            base.extend(run_baseline(&scn, &thresholds));
        }
        let cpu_base = cpu_time_ms() - cpu_base_start;

        let cpu_cand_start = cpu_time_ms();
        for _ in 0..iterations {
            cand.extend(run_with_candle(&scn, &thresholds, &q_backend, &d_backend));
        }
        let cpu_cand = cpu_time_ms() - cpu_cand_start;

        let n_searches = base.len() as f64;
        let (b50, b95) = summarize(base);
        let (c50, c95) = summarize(cand);
        deltas.insert(n, (c50 - b50, c95 - b95));

        println!("--- {n} results/search ---");
        println!("  BASELINE   p50={b50:.3}  p95={b95:.3}");
        println!("  CANDLE     p50={c50:.3}  p95={c95:.3}");
        println!("  INCREMENTAL p50={:.3}  p95={:.3}", c50 - b50, c95 - b95);
        println!(
            "  CPU_TOTAL   baseline={:.1} ms  candle={:.1} ms  over {} searches",
            cpu_base, cpu_cand, n_searches
        );
    }

    let rss_after = rss_kb();

    println!("\n=== DELTAS (incremental cost of the Candle seam) ===");
    for (n, (d50, d95)) in &deltas {
        println!("  E2E_{n}_RESULTS_DELTA: p50 +{d50:.3} ms   p95 +{d95:.3} ms");
    }
    println!("\n=== MEMORY ===");
    println!(
        "  RSS_BEFORE_KB={rss_before}  RSS_AFTER_KB={rss_after}  DELTA_KB={}",
        rss_after.saturating_sub(rss_before)
    );
    println!(
        "  (fixture backend holds no model; a real Candle backend adds ~137 MB RSS — STEP 4B §6)"
    );
}

fn cpu_time_ms() -> f64 {
    // Sum of user + system CPU time for this process, via getrusage-free
    // /proc/self/stat (fields 14 utime, 15 stime, in clock ticks).
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    // field 2 is "(comm)" which may contain spaces/parens; split after the last ')'.
    let after = stat.rsplit_once(')').map(|(_, r)| r).unwrap_or(&stat);
    let fields: Vec<&str> = after.split_whitespace().collect();
    // after ')' , index 0 == field 3 (state). utime = field 14 => index 11, stime => index 12.
    let utime: f64 = fields.get(11).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let stime: f64 = fields.get(12).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let hz = 100.0; // USER_HZ on Linux x86_64
    (utime + stime) / hz * 1000.0
}

/// Fallback tests for the seam, run in the integrated (feature-on) build.
#[test]
fn seam_failure_modes_fall_back_to_bounded_semantic() {
    use amatl_core::experimental_embeddings::EmbeddingUnavailable;

    let query = parse_query("kubernetes ingress controller".to_string()).unwrap();
    let bounded = assess_semantics(
        &query,
        Some("k8s ingress"),
        Some("how ingress controllers route traffic"),
        "",
        &Default::default(),
    );

    for reason in [
        EmbeddingUnavailable::ModelMissing,
        EmbeddingUnavailable::ModelCorrupt,
        EmbeddingUnavailable::HashMismatch,
        EmbeddingUnavailable::TokenizerMissing,
        EmbeddingUnavailable::ConfigMissing,
        EmbeddingUnavailable::BackendError("simulated".into()),
    ] {
        let backend = FixtureBackend::failing(reason.clone());
        let r = evaluate_with_embeddings(&query, "some result text", &bounded, &backend);
        assert!(r.used_fallback, "{reason:?} must fall back");
        assert_eq!(r.fallback_reason.as_ref(), Some(&reason));
        assert_eq!(r.cosine, 0.0);
        // Bounded-semantic assessment is preserved and unchanged.
        assert_eq!(r.bounded_strong_rescue, bounded.is_strong_rescue());
    }

    // Empty query / empty result set.
    let mut empty_q = query.clone();
    empty_q.normalized_query.clear();
    let ok = FixtureBackend::default();
    assert!(evaluate_with_embeddings(&empty_q, "text", &bounded, &ok).used_fallback);
    assert!(evaluate_with_embeddings(&query, "", &bounded, &ok).used_fallback);

    // Non-ascii / long input: no panic, finite cosine, no fallback.
    let long = "términos ".repeat(20_000);
    let r = evaluate_with_embeddings(&query, &long, &bounded, &ok);
    assert!(!r.used_fallback);
    assert!(r.cosine.is_finite());
}
