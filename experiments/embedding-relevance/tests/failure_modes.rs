//! Spec §7 — failure / fallback behavior for the embedding backend.
//!
//! These tests require the model to be present in the HF cache (run `bench`
//! once first). They are `#[ignore]` by default so `cargo test` stays offline
//! and fast; run with `cargo test -- --ignored`.

use amatl_embedding_relevance_experiment::fastembed_backend::FastembedBackend;
use amatl_embedding_relevance_experiment::{cosine_similarity, EmbeddingBackend};

fn backend() -> FastembedBackend {
    // Use the crate-local cache populated by `cargo run --bin bench`.
    let cache = concat!(env!("CARGO_MANIFEST_DIR"), "/.fastembed_cache");
    if std::path::Path::new(cache).exists() {
        std::env::set_var("FASTEMBED_CACHE_PATH", cache);
    }
    FastembedBackend::bge_small_en_v15().expect("model must be cached; run `cargo run --bin bench`")
}

#[test]
#[ignore = "requires cached model"]
fn empty_query_returns_zero_vector_not_error() {
    let b = backend();
    let v = b.embed_query("").expect("empty query must not error");
    assert_eq!(v.len(), b.dim());
    assert!(v.iter().all(|x| *x == 0.0));
}

#[test]
#[ignore = "requires cached model"]
fn empty_document_returns_zero_vector() {
    let b = backend();
    let v = b
        .embed_document("   \n  ")
        .expect("empty doc must not error");
    assert!(v.iter().all(|x| *x == 0.0));
    // cosine with a zero vector is defined as 0.0, never NaN
    let q = b.embed_query("anything").unwrap();
    let s = cosine_similarity(&q, &v);
    assert_eq!(s, 0.0);
}

#[test]
#[ignore = "requires cached model"]
fn empty_title_or_snippet_only() {
    let b = backend();
    let title_only =
        amatl_embedding_relevance_experiment::document_text(Some("Rust ownership"), None);
    let snippet_only =
        amatl_embedding_relevance_experiment::document_text(None, Some("Borrow checker rules"));
    assert!(!b
        .embed_document(&title_only)
        .unwrap()
        .iter()
        .all(|x| *x == 0.0));
    assert!(!b
        .embed_document(&snippet_only)
        .unwrap()
        .iter()
        .all(|x| *x == 0.0));
}

#[test]
#[ignore = "requires cached model"]
fn non_ascii_text_ok() {
    let b = backend();
    let v = b
        .embed_document("Café — naïve façade — 日本語 — Ελληνικά — emoji 🚀")
        .expect("non-ascii must not error");
    assert_eq!(v.len(), b.dim());
    assert!(v.iter().any(|x| *x != 0.0));
}

#[test]
#[ignore = "requires cached model"]
fn very_long_text_truncates_not_panics() {
    let b = backend();
    let long = "distributed systems consensus ".repeat(4000); // ~120k chars
    let v = b.embed_document(&long).expect("long text must not panic");
    assert_eq!(v.len(), b.dim());
}

#[test]
#[ignore = "requires cached model"]
fn repeated_input_is_deterministic() {
    let b = backend();
    let a = b.embed_query("how to prevent burnout at work").unwrap();
    let c = b.embed_query("how to prevent burnout at work").unwrap();
    assert_eq!(a.len(), c.len());
    let max_abs_diff = a
        .iter()
        .zip(&c)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0f32, f32::max);
    assert!(
        max_abs_diff < 1e-6,
        "expected deterministic embeddings, max abs diff {max_abs_diff}"
    );
}

// model-missing / model-corrupt behavior is exercised in
// `tests/model_missing.rs` (own process — it mutates process-wide env).

/// Pure-function guards that need no model.
#[test]
fn cosine_never_nan_on_degenerate_input() {
    assert_eq!(cosine_similarity(&[], &[]), 0.0);
    assert_eq!(cosine_similarity(&[0.0, 0.0], &[0.0, 0.0]), 0.0);
    assert_eq!(cosine_similarity(&[1.0, 2.0], &[1.0]), 0.0);
    let s = cosine_similarity(&[1.0, 0.0], &[1.0, 0.0]);
    assert!((s - 1.0).abs() < 1e-6);
}

#[test]
fn document_text_skips_missing_parts() {
    use amatl_embedding_relevance_experiment::document_text;
    assert_eq!(document_text(None, None), "");
    assert_eq!(document_text(Some("  T  "), None), "T");
    assert_eq!(document_text(Some("T"), Some("S")), "T\nS");
    assert_eq!(document_text(Some(""), Some("S")), "S");
}
