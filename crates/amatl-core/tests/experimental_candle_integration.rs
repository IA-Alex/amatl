//! STEP 4D — real Candle backend integration + fallback matrix.
//!
//! Compiled only under `--features experimental-local-embeddings`.
//!
//! Tests that need the actual ~128 MB model are `#[ignore]` by default and
//! keyed on `AMATL_TEST_MODEL_DIR` pointing at a valid package directory
//! (`model.safetensors` / `tokenizer.json` / `config.json`). Run:
//!
//! ```text
//! AMATL_TEST_MODEL_DIR=/path/to/pkg cargo test -p amatl-core \
//!   --features experimental-local-embeddings \
//!   --test experimental_candle_integration -- --ignored --nocapture --test-threads=1
//! ```
//!
//! The fallback-matrix tests need no model and run in the ordinary
//! feature-on `cargo test`.

#![cfg(feature = "experimental-local-embeddings")]

use std::collections::BTreeMap;

use amatl_core::experimental_embeddings::{
    model_config::{ModelPackage, ModelPackageError, PINNED_MODEL_VERSION},
    CandleBackend, EmbeddingBackend, ExperimentalSemanticConfig, SemanticEvaluator,
    DEFAULT_EXPERIMENTAL_K,
};
use amatl_core::{
    parse_query, CanonicalUrl, DeduplicatedResult, DuplicateStatus, OriginalUrl, Rank,
    RelevanceClassification, ResultType, SCHEMA_VERSION,
};

fn model_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("AMATL_TEST_MODEL_DIR").map(std::path::PathBuf::from)
}

fn result(
    url: &str,
    title: Option<&str>,
    snippet: Option<&str>,
    rank: Option<u32>,
) -> DeduplicatedResult {
    let parsed = url::Url::parse(url).unwrap();
    let mut provider_ranks = BTreeMap::new();
    provider_ranks.insert("corpus".to_string(), rank.and_then(|v| Rank::new(v).ok()));
    DeduplicatedResult {
        schema_version: SCHEMA_VERSION.into(),
        title: title.map(str::to_string),
        original_url: OriginalUrl(parsed.clone()),
        canonical_url: CanonicalUrl(parsed.clone()),
        original_urls: vec![OriginalUrl(parsed)],
        providers: vec!["corpus".to_string()],
        representative_provider: "corpus".into(),
        provider_ranks,
        snippet: snippet.map(str::to_string),
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

// ---------------------------------------------------------------------------
// Fallback matrix — no model required.
// ---------------------------------------------------------------------------

#[test]
fn model_missing_yields_model_missing_error() {
    let tmp = std::env::temp_dir().join("amatl_4d_missing");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    assert_eq!(
        ModelPackage::resolve(&tmp, None).unwrap_err(),
        ModelPackageError::ModelMissing
    );
    // The evaluator's convenience loader turns that into `None` (→ fallback).
    assert!(SemanticEvaluator::load_backend(&tmp, None).is_none());
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn model_corrupt_and_hash_mismatch_fall_back() {
    let tmp = std::env::temp_dir().join("amatl_4d_corrupt");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("model.safetensors"), b"garbage").unwrap();
    std::fs::write(tmp.join("tokenizer.json"), b"{}").unwrap();
    std::fs::write(tmp.join("config.json"), b"{}").unwrap();
    // Real pinned hashes will not match these files.
    let err = ModelPackage::resolve(&tmp, None).unwrap_err();
    assert!(matches!(err, ModelPackageError::HashMismatch { .. }));
    assert!(SemanticEvaluator::load_backend(&tmp, None).is_none());
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn tokenizer_and_config_missing_detected() {
    let tmp = std::env::temp_dir().join("amatl_4d_partial");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("model.safetensors"), b"w").unwrap();
    assert_eq!(
        ModelPackage::resolve(&tmp, None).unwrap_err(),
        ModelPackageError::TokenizerMissing
    );
    std::fs::write(tmp.join("tokenizer.json"), b"t").unwrap();
    assert_eq!(
        ModelPackage::resolve(&tmp, None).unwrap_err(),
        ModelPackageError::ConfigMissing
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn default_experimental_k_is_small() {
    assert!((5..=10).contains(&DEFAULT_EXPERIMENTAL_K));
}

// ---------------------------------------------------------------------------
// Real Candle backend — needs AMATL_TEST_MODEL_DIR.
// ---------------------------------------------------------------------------

#[test]
#[ignore = "needs AMATL_TEST_MODEL_DIR"]
fn real_backend_loads_and_embeds_offline() {
    let Some(dir) = model_dir() else { return };
    let pkg = ModelPackage::resolve(&dir, Some(PINNED_MODEL_VERSION)).expect("valid package");
    let backend = CandleBackend::load(&pkg).expect("load");
    assert_eq!(backend.dimension(), 384);

    let q = backend
        .embed_query("kubernetes ingress controller")
        .unwrap();
    let d = backend
        .embed_document("How ingress controllers route external traffic into a cluster")
        .unwrap();
    assert_eq!(q.len(), 384);
    assert_eq!(d.len(), 384);

    // Empty input is a typed error, not a panic.
    assert!(backend.embed_query("").is_err());
    assert!(backend.embed_document("   ").is_err());

    // Non-ascii / long input: no panic.
    let _ = backend.embed_document("café — 日本語 — Ελληνικά 🚀");
    let long = "distributed systems consensus ".repeat(4000);
    assert_eq!(backend.embed_document(&long).unwrap().len(), 384);

    // Determinism.
    let q2 = backend
        .embed_query("kubernetes ingress controller")
        .unwrap();
    let drift = q
        .iter()
        .zip(&q2)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(drift < 1e-6, "drift {drift}");
}

#[test]
#[ignore = "needs AMATL_TEST_MODEL_DIR"]
fn real_pipeline_query_embedded_once_and_bounded() {
    let Some(dir) = model_dir() else { return };
    let backend = SemanticEvaluator::load_backend(&dir, Some(PINNED_MODEL_VERSION)).expect("load");
    let evaluator = SemanticEvaluator::new(&backend, ExperimentalSemanticConfig::default());

    let query = parse_query("how to prevent burnout at work".to_string()).unwrap();
    // Row A: strong lexical -> Relevant (never a candidate).
    // Rows B..: partial overlap ("burnout"/"work" present, not full) -> the
    // PossiblyRelevant ambiguous band, which is what the pass evaluates.
    let mut results = vec![result(
        "https://a.test/burnout",
        Some("How to prevent burnout at work"),
        Some("practical steps to prevent workplace burnout at work"),
        Some(1),
    )];
    for i in 0..25 {
        results.push(result(
            &format!("https://amb.test/{i}"),
            Some("Notes on burnout"),
            Some("some thoughts about burnout and staying healthy in a demanding job"),
            Some(2 + i),
        ));
    }

    let outcome = evaluator.evaluate_search(&query, &results);
    assert_eq!(outcome.results_total, results.len());
    assert!(
        outcome.semantic_candidates <= DEFAULT_EXPERIMENTAL_K,
        "candidate set bounded by K: {} > {}",
        outcome.semantic_candidates,
        DEFAULT_EXPERIMENTAL_K
    );
    assert!(
        outcome.semantic_candidates >= 1,
        "some ambiguous candidates"
    );
    // Invariant: query embedded exactly once whenever the pass runs.
    assert_eq!(outcome.query_embed_count, 1, "query embedded exactly once");
    assert_eq!(
        outcome.document_embed_batches, 1,
        "documents embedded in one batch"
    );

    // Production classifications are passed through unchanged.
    for e in &outcome.evaluations {
        let recomputed = amatl_core::assess_result(
            &query,
            &results[e.result_index],
            &amatl_core::RelevanceThresholds::default(),
        );
        assert_eq!(e.production.classification, recomputed.classification);
        // Suggestions, if any, are only PossiblyRelevant -> Relevant.
        if let Some(s) = e.suggested {
            assert_eq!(s, RelevanceClassification::Relevant);
            assert_eq!(
                e.production.classification,
                RelevanceClassification::PossiblyRelevant
            );
        }
    }
}

#[test]
#[ignore = "needs AMATL_TEST_MODEL_DIR"]
fn contradiction_case_never_promoted_by_embedding() {
    let Some(dir) = model_dir() else { return };
    let backend = SemanticEvaluator::load_backend(&dir, None).expect("load");
    let evaluator = SemanticEvaluator::new(&backend, ExperimentalSemanticConfig::default());

    // Query about one entity, result about a different, semantically-near one.
    let query = parse_query("python language garbage collection".to_string()).unwrap();
    let results = vec![result(
        "https://snake.test/",
        Some("Python (Monty) — the ball python care guide"),
        Some("humidity, feeding and habitat for pet ball pythons; nothing about programming"),
        Some(1),
    )];

    let outcome = evaluator.evaluate_search(&query, &results);
    for e in &outcome.evaluations {
        assert!(
            e.suggested.is_none(),
            "embedding must not promote a contradiction/entity-mismatch case"
        );
    }
}

#[test]
#[ignore = "needs AMATL_TEST_MODEL_DIR"]
fn no_ambiguous_results_means_no_embedding_work() {
    let Some(dir) = model_dir() else { return };
    let backend = SemanticEvaluator::load_backend(&dir, None).expect("load");
    let evaluator = SemanticEvaluator::new(&backend, ExperimentalSemanticConfig::default());

    let query = parse_query("rust ownership".to_string()).unwrap();
    // Strongly relevant only — no PossiblyRelevant band.
    let results = vec![result(
        "https://doc.rust-lang.org/ownership",
        Some("Understanding Ownership - The Rust Programming Language"),
        Some("Ownership is Rust's most unique feature. Rust ownership rules and borrowing."),
        Some(1),
    )];
    let outcome = evaluator.evaluate_search(&query, &results);
    assert_eq!(outcome.query_embed_count, 0);
    assert_eq!(outcome.semantic_candidates, 0);
    assert!(outcome.evaluations.is_empty());
}

#[test]
#[ignore = "needs AMATL_TEST_MODEL_DIR"]
fn empty_query_and_empty_results_fall_back() {
    let Some(dir) = model_dir() else { return };
    let backend = SemanticEvaluator::load_backend(&dir, None).expect("load");
    let evaluator = SemanticEvaluator::new(&backend, ExperimentalSemanticConfig::default());

    // Empty result set.
    let query = parse_query("anything".to_string()).unwrap();
    let outcome = evaluator.evaluate_search(&query, &[]);
    assert_eq!(outcome.semantic_candidates, 0);
    assert_eq!(outcome.query_embed_count, 0);

    // Query that normalises to empty, with an ambiguous result present.
    let mut empty_q = parse_query("placeholder".to_string()).unwrap();
    empty_q.normalized_query.clear();
    empty_q.raw_query.clear();
    let results = vec![result(
        "https://x.test/",
        Some("Some tangentially related page"),
        Some("a snippet with partial overlap to trigger the possibly-relevant band maybe"),
        Some(1),
    )];
    let outcome = evaluator.evaluate_search(&empty_q, &results);
    // Either no candidate, or fell back cleanly with no suggestion.
    for e in &outcome.evaluations {
        assert!(e.rescore.used_fallback);
        assert!(e.suggested.is_none());
    }
}
