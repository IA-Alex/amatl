//! STEP 4B §5 — failure / fallback behavior for the Candle backend.
//! Mirrors `tests/failure_modes.rs` (the ONNX suite). `#[ignore]` by default
//! so `cargo test` stays offline and fast; run once the model is cached:
//!   cargo test --release -- --ignored
//! The model is fetched into `./.candle_cache` by `cargo run --bin candle_bench`.

use amatl_embedding_relevance_experiment::candle_backend::CandleBackend;
use amatl_embedding_relevance_experiment::{cosine_similarity, EmbeddingBackend};

fn backend() -> CandleBackend {
    CandleBackend::bge_small_en_v15()
        .expect("model must be cached; run `cargo run --release --bin candle_bench`")
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
fn empty_document_zero_vector_cosine_zero() {
    let b = backend();
    let v = b.embed_document("  \n ").expect("empty doc must not error");
    assert!(v.iter().all(|x| *x == 0.0));
    let q = b.embed_query("anything").unwrap();
    assert_eq!(cosine_similarity(&q, &v), 0.0);
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
    assert!(v.iter().any(|x| *x != 0.0));
}

#[test]
#[ignore = "requires cached model"]
fn repeated_input_is_deterministic() {
    let b = backend();
    let a = b.embed_query("how to prevent burnout at work").unwrap();
    let c = b.embed_query("how to prevent burnout at work").unwrap();
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

#[test]
#[ignore = "requires cached model"]
fn batch_matches_single() {
    let b = backend();
    let texts = vec![
        "rust borrow checker".to_string(),
        "".to_string(),
        "kubernetes pod scheduling".to_string(),
    ];
    let batch = b.embed_documents(&texts).unwrap();
    assert_eq!(batch.len(), 3);
    assert!(batch[1].iter().all(|x| *x == 0.0)); // empty slot
    let single0 = b.embed_document(&texts[0]).unwrap();
    let d: f32 = batch[0]
        .iter()
        .zip(&single0)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f32::max);
    assert!(d < 1e-3, "batch vs single drift {d}");
}

#[test]
#[ignore = "requires cached model"]
fn missing_model_is_clean_err_not_panic() {
    let tmp = std::env::temp_dir().join("amatl_candle_missing_xyz");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    // Put a config but no weights so no network fetch is attempted for it and
    // the safetensors load fails cleanly.
    std::fs::write(tmp.join("config.json"), "{}").unwrap();
    std::fs::write(tmp.join("tokenizer.json"), "{}").unwrap();
    std::fs::write(tmp.join("model.safetensors"), b"not a real safetensors").unwrap();
    let r = CandleBackend::load(&tmp);
    assert!(r.is_err(), "corrupt model must return Err, got Ok");
    let _ = std::fs::remove_dir_all(&tmp);
}
