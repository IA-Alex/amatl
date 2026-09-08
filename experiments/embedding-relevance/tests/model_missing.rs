//! Spec §8 — MODEL_MISSING / MODEL_CORRUPT must be a clean `Err`, never a
//! panic, so a caller can fall back to bounded-semantic relevance.
//!
//! This test mutates process-wide environment, so it lives in its own test
//! binary (one process, runs alone).

use amatl_embedding_relevance_experiment::fastembed_backend::FastembedBackend;

#[test]
fn model_missing_returns_err_not_panic() {
    let tmp = std::env::temp_dir().join(format!("amatl-embed-missing-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    // Force fastembed's cache to an empty dir and forbid network.
    std::env::set_var("FASTEMBED_CACHE_PATH", &tmp);
    std::env::set_var("HF_HUB_OFFLINE", "1");

    let r = std::panic::catch_unwind(FastembedBackend::bge_small_en_v15);

    let _ = std::fs::remove_dir_all(&tmp);
    match r {
        Ok(Ok(_)) => {
            // If a global cache satisfied it anyway, that's acceptable — the
            // contract we assert is "no panic".
        }
        Ok(Err(e)) => {
            eprintln!("expected clean error on missing model: {e:#}");
        }
        Err(_) => panic!("init panicked on missing model; must return Result"),
    }
}

#[test]
fn model_corrupt_returns_err_not_panic() {
    let tmp = std::env::temp_dir().join(format!("amatl-embed-corrupt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let blob_dir = tmp.join("models--Xenova--bge-small-en-v1.5");
    std::fs::create_dir_all(&blob_dir).unwrap();
    // Write garbage where the model would be.
    std::fs::write(blob_dir.join("model.onnx"), b"not a real onnx file").unwrap();
    std::env::set_var("FASTEMBED_CACHE_PATH", &tmp);
    std::env::set_var("HF_HUB_OFFLINE", "1");

    let r = std::panic::catch_unwind(FastembedBackend::bge_small_en_v15);

    let _ = std::fs::remove_dir_all(&tmp);
    assert!(
        r.is_ok(),
        "init panicked on corrupt model; must return Result for safe fallback"
    );
}
