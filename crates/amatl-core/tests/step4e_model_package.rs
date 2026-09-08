//! STEP4E infrastructure-only validation. It uses no relevance corpus rows.

#![cfg(feature = "experimental-local-embeddings")]

use amatl_core::experimental_embeddings::{
    model_config::{ModelPackage, PINNED_MODEL_VERSION},
    CandleBackend, EmbeddingBackend,
};

#[test]
#[ignore = "requires an explicitly supplied, verified optional model package"]
fn verified_package_loads_and_embeds_neutral_synthetic_text() {
    let dir = std::env::var_os("AMATL_TEST_MODEL_DIR")
        .map(std::path::PathBuf::from)
        .expect("AMATL_TEST_MODEL_DIR must identify the optional model package");
    let started = std::time::Instant::now();
    let package =
        ModelPackage::resolve(&dir, Some(PINNED_MODEL_VERSION)).expect("verified package");
    let backend = CandleBackend::load(&package).expect("Candle loads verified package");
    let vector = backend
        .embed_document("local embedding package verification")
        .expect("neutral synthetic text embeds");
    assert_eq!(backend.dimension(), 384);
    assert_eq!(vector.len(), 384);
    assert!(vector.iter().all(|value| value.is_finite()));
    println!("MODEL_LOAD_MS={}", started.elapsed().as_millis());
}
