use amatl_core::{Extractor, TrafilaturaExtractor};

#[tokio::test]
async fn installed_trafilatura_extracts_real_html_and_metadata() {
    if std::env::var_os("AMATL_TRAFILATURA_INTEGRATION").is_none() {
        eprintln!("skipped: set AMATL_TRAFILATURA_INTEGRATION=1 to require real Trafilatura");
        return;
    }
    let executable =
        std::env::var("AMATL_TRAFILATURA_BIN").unwrap_or_else(|_| "trafilatura".to_owned());
    let extractor = TrafilaturaExtractor::new(
        executable,
        "trafilatura-2.2.0-cli-json-v1".into(),
        8_000,
        4 * 1024 * 1024,
    );
    let result = extractor
        .extract(include_bytes!("fixtures/trafilatura_article.html"))
        .await
        .expect("the pinned Trafilatura CLI must satisfy AMATL's extraction contract");

    assert!(result.content.contains("AMATL procesa HTML"));
    assert!(!result.content.contains("ruido de navegación"));
    assert_eq!(
        result.title.as_deref(),
        Some("Contrato real de Trafilatura")
    );
    assert_eq!(result.author.as_deref(), Some("Equipo AMATL"));
    assert_eq!(result.published_at.as_deref(), Some("2026-08-13"));
    assert_eq!(result.extractor_used, "trafilatura-2.2.0-cli-json-v1");
}
