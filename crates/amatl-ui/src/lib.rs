//! Static, embeddable AMATL UI surface.
//!
//! This crate owns presentation assets only. Search, Deep, provider and Budget
//! behavior remain in `amatl-core`; an HTTP transport may serve these assets in
//! Phase 9 without copying product logic into the UI.

pub const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; object-src 'none'; base-uri 'self'; frame-ancestors 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self'; connect-src 'self'; form-action 'self'";
pub const REFERRER_POLICY: &str = "no-referrer";
pub const PERMISSIONS_POLICY: &str = "camera=(), microphone=(), geolocation=()";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UiAsset {
    pub content_type: &'static str,
    pub cache_control: &'static str,
    pub body: &'static [u8],
}

pub fn asset(path: &str) -> Option<UiAsset> {
    match path {
        "/" | "/index.html" => Some(UiAsset {
            content_type: "text/html; charset=utf-8",
            cache_control: "no-store",
            body: include_bytes!("../assets/index.html"),
        }),
        "/styles.css" => Some(UiAsset {
            content_type: "text/css; charset=utf-8",
            cache_control: "public, max-age=3600",
            body: include_bytes!("../assets/styles.css"),
        }),
        "/app.js" => Some(UiAsset {
            content_type: "text/javascript; charset=utf-8",
            cache_control: "public, max-age=3600",
            body: include_bytes!("../assets/app.js"),
        }),
        // Message catalog, kept out of app.js so translations are one file.
        "/i18n.js" => Some(UiAsset {
            content_type: "text/javascript; charset=utf-8",
            cache_control: "public, max-age=3600",
            body: include_bytes!("../assets/i18n.js"),
        }),
        _ => None,
    }
}

pub fn security_headers(https: bool) -> Vec<(&'static str, &'static str)> {
    let mut headers = vec![
        ("Content-Security-Policy", CONTENT_SECURITY_POLICY),
        ("X-Content-Type-Options", "nosniff"),
        ("Referrer-Policy", REFERRER_POLICY),
        ("Permissions-Policy", PERMISSIONS_POLICY),
        ("X-Frame-Options", "DENY"),
    ];
    if https {
        headers.push((
            "Strict-Transport-Security",
            "max-age=31536000; includeSubDomains",
        ));
    }
    headers
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(path: &str) -> String {
        String::from_utf8(asset(path).unwrap().body.to_vec()).unwrap()
    }

    #[test]
    fn serves_only_known_assets_with_explicit_types() {
        assert_eq!(asset("/").unwrap(), asset("/index.html").unwrap());
        assert_eq!(
            asset("/app.js").unwrap().content_type,
            "text/javascript; charset=utf-8"
        );
        assert_eq!(
            asset("/styles.css").unwrap().content_type,
            "text/css; charset=utf-8"
        );
        assert!(asset("/../Cargo.toml").is_none());
        assert!(asset("/unknown").is_none());
    }

    #[test]
    fn csp_is_self_only_and_has_no_unsafe_escape_hatch() {
        assert!(CONTENT_SECURITY_POLICY.contains("default-src 'self'"));
        assert!(CONTENT_SECURITY_POLICY.contains("connect-src 'self'"));
        assert!(CONTENT_SECURITY_POLICY.contains("object-src 'none'"));
        assert!(CONTENT_SECURITY_POLICY.contains("frame-ancestors 'none'"));
        assert!(!CONTENT_SECURITY_POLICY.contains("unsafe-inline"));
        assert!(!CONTENT_SECURITY_POLICY.contains("unsafe-eval"));
        assert!(!CONTENT_SECURITY_POLICY.contains('*'));
        assert!(security_headers(false)
            .iter()
            .all(|(name, _)| *name != "Strict-Transport-Security"));
        assert!(security_headers(true)
            .iter()
            .any(|(name, _)| *name == "Strict-Transport-Security"));
    }

    #[test]
    fn markup_has_no_inline_executable_content_and_is_accessible() {
        let html = text("/index.html");
        assert!(!html.contains("<style"));
        assert!(!html.contains("<script>"));
        assert!(!html.contains(" onclick="));
        assert!(html.contains("aria-live=\"polite\""));
        assert!(html.contains("<label"));
        assert!(html.contains("class=\"skip-link\""));
        assert!(html.contains(&format!("content=\"{CONTENT_SECURITY_POLICY}\"")));
    }

    #[test]
    fn presentation_keeps_technical_internals_hidden() {
        let surface = format!(
            "{}{}{}",
            text("/index.html"),
            text("/app.js"),
            text("/i18n.js")
        )
        .to_lowercase();
        for internal in [
            "rrf",
            "combined_score",
            "provider health",
            "telemetry",
            "retry",
            "ranking_v2",
            "evidence_score",
        ] {
            assert!(
                !surface.contains(internal),
                "leaked UI internal: {internal}"
            );
        }
    }

    #[test]
    fn layout_contract_covers_mobile_and_respects_reduced_motion() {
        let css = text("/styles.css");
        assert!(css.contains("@media (max-width: 44rem)"));
        assert!(css.contains("prefers-reduced-motion: reduce"));
        assert!(css.contains("min-width: 0"));
        assert!(css.contains("overflow-wrap: anywhere"));
        assert!(css.contains(".pagination[hidden] { display: none; }"));
        for token in [
            "#111315",
            "#181b1f",
            "#2a2f35",
            "#e7e9ec",
            "#9da5ae",
            "#6f7780",
            "#4f8cff",
            "#48b8c7",
            "#4fae72",
            "#d6a84b",
            "#d95c5c",
            "Inter",
            "JetBrains Mono",
        ] {
            assert!(css.contains(token), "missing visual token: {token}");
        }
    }

    #[test]
    fn search_and_deep_use_post_contracts_and_safe_dom_apis() {
        let html = text("/index.html");
        let javascript = text("/app.js");
        assert!(html.contains("action=\"/search\" method=\"post\""));
        assert!(!html.contains("method=\"get\""));
        assert!(!html.contains("name=\"token\""));
        assert!(html.contains("type=\"password\""));
        assert!(html.contains("autocomplete=\"off\""));
        assert!(html.contains("minlength=\"32\""));
        assert!(html.contains("aria-describedby=\"token-help\""));
        for field in [
            "name=\"q\"",
            "name=\"lang\"",
            "name=\"region\"",
            "name=\"filetype\"",
        ] {
            assert!(html.contains(field), "missing UI field: {field}");
        }
        assert!(javascript.contains("payload.schema_version !== \"1\""));
        assert!(javascript.contains("mode === \"deep\" ? \"/deep\" : \"/search\""));
        assert!(javascript.contains("fetch(endpoint"));
        assert!(javascript.contains("method: \"POST\""));
        assert!(javascript.contains("\"Content-Type\": \"application/json\""));
        assert!(javascript.contains("body: JSON.stringify(body)"));
        assert!(javascript.contains("body.page = state.page"));
        assert!(javascript.contains("headers.Authorization"));
        assert!(!javascript.contains("searchParams"));
        assert!(!javascript.contains("method: \"GET\""));
        // Pagination is server-side only: no local windowing of the result set.
        assert!(!javascript.contains("serverPagination"));
        assert!(!javascript.contains("state.items.slice"));
        assert!(javascript.contains("body.page_size = PAGE_SIZE"));
        assert!(javascript.contains("payload.total_results"));
        assert!(javascript.contains("payload.status === \"partial_success\""));
        assert!(javascript.contains("result.canonical_url"));
        assert!(javascript.contains("parsed.protocol === \"http:\""));
        assert!(javascript.contains("parsed.protocol === \"https:\""));
        assert!(javascript.contains("textContent"));
        assert!(!javascript.contains("innerHTML"));
        assert!(!javascript.contains("document.write"));
    }

    #[test]
    fn deep_evidence_is_bounded_traceable_and_has_no_local_file_surface() {
        let html = text("/index.html");
        let javascript = text("/app.js");
        assert!(html.contains("id=\"deep-button\""));
        assert!(html.contains("id=\"deep-template\""));
        assert!(html.contains("id=\"fragment-template\""));
        assert!(html.contains("Procedencia verificable"));
        assert!(html.contains("<blockquote>"));
        assert!(javascript.contains("Array.isArray(payload.evidence_v2)"));
        assert!(javascript.contains("const MAX_DEEP_DOCUMENTS = 20"));
        assert!(javascript.contains("const MAX_FRAGMENTS = 8"));
        assert!(javascript.contains("const MAX_FRAGMENT_BYTES = 512"));
        assert!(javascript.contains("fragment.provenance_id !== provenanceId"));
        assert!(javascript.contains("provenance.document_id !== evidence.document_id"));
        assert!(javascript.contains("provenance.final_url === documentValue.final_url"));
        assert!(javascript.contains("new TextEncoder()"));
        assert!(javascript.contains("new TextDecoder(\"utf-8\", { fatal: true })"));
        assert!(javascript.contains("crypto.subtle.digest(\"SHA-256\""));
        assert!(!html.contains("type=\"file\""));
        assert!(!javascript.contains("FileReader"));
        assert!(!javascript.contains("\"/ingest\""));
        assert!(!javascript.contains("innerHTML"));
        assert!(!javascript.contains("document.write"));
    }

    #[test]
    fn local_domain_panels_use_the_bounded_service_endpoints() {
        let html = text("/index.html");
        let javascript = text("/app.js");
        for id in [
            "id=\"service-indicators\"",
            "id=\"history-list\"",
            "id=\"history-purge\"",
            "id=\"saved-list\"",
            "id=\"history-template\"",
            "id=\"saved-template\"",
        ] {
            assert!(html.contains(id), "missing panel element: {id}");
        }
        assert!(javascript.contains("fetch(\"/status\""));
        assert!(javascript.contains("`/history?limit=${LIST_LIMIT}`"));
        assert!(javascript.contains("`/saved?limit=${LIST_LIMIT}`"));
        assert!(javascript.contains("method: \"DELETE\""));
        // Saved payloads stay bounded and never carry extracted content.
        assert!(javascript.contains("MAX_SAVED_PAYLOAD_BYTES"));
        assert!(!javascript.contains("documentValue.content,"));
        // Every request to a protected surface carries the in-memory token.
        assert!(javascript.contains("headers: authHeaders()"));
    }

    #[test]
    fn user_visible_copy_lives_only_in_the_message_catalog() {
        let catalog = text("/i18n.js");
        let javascript = text("/app.js");
        assert!(catalog.contains("globalThis.AMATL_LOCALES"));
        assert!(catalog.contains("defaultLocale: \"en\""));
        // app.js resolves copy through the catalog instead of embedding it.
        assert!(javascript.contains("globalThis.AMATL_LOCALES"));
        assert!(!javascript.contains("const MSG = {"));
        for key in [
            "historyHeading",
            "savedHeading",
            "serviceHeading",
            "storageLabel",
            "cacheLabel",
        ] {
            assert!(catalog.contains(key), "missing catalog key: {key}");
        }
        // Both shipped locales expose the same keys; `en` is the contract.
        let english = catalog
            .split("    en: {")
            .nth(1)
            .expect("english catalog block");
        let spanish = catalog
            .split("    es: {")
            .nth(1)
            .and_then(|value| value.split("    en: {").next())
            .expect("spanish catalog block");
        let keys = |block: &str| {
            block
                .lines()
                .filter_map(|line| line.trim().split_once(':').map(|(key, _)| key.to_owned()))
                .filter(|key| !key.starts_with("//") && !key.contains(' '))
                .collect::<std::collections::BTreeSet<_>>()
        };
        assert_eq!(keys(english), keys(spanish));
    }
}
