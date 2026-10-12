//! Static, embeddable AMATL UI surface.
//!
//! This crate owns presentation assets only. Search, Deep, provider and Budget
//! behavior remain in `amatl-core`; an HTTP transport may serve these assets in
//! Phase 9 without copying product logic into the UI.

pub const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; object-src 'none'; base-uri 'self'; frame-ancestors 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self'; connect-src 'self'; form-action 'self'";
/// The same policy as [`CONTENT_SECURITY_POLICY`] minus `frame-ancestors`.
///
/// `frame-ancestors` is ignored (and warned about in the console) when
/// delivered through a `<meta>` element; only the response header can enforce
/// it. The header keeps the full policy, so frame-embedding stays blocked,
/// while the `<meta>` mirrors only the directives it can actually apply.
pub const META_CONTENT_SECURITY_POLICY: &str = "default-src 'self'; object-src 'none'; base-uri 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self'; connect-src 'self'; form-action 'self'";
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
        "/client.js" => Some(UiAsset {
            content_type: "text/javascript; charset=utf-8",
            cache_control: "public, max-age=3600",
            body: include_bytes!("../assets/client.js"),
        }),
        "/store.js" => Some(UiAsset {
            content_type: "text/javascript; charset=utf-8",
            cache_control: "public, max-age=3600",
            body: include_bytes!("../assets/store.js"),
        }),
        "/app.js" => Some(UiAsset {
            content_type: "text/javascript; charset=utf-8",
            cache_control: "public, max-age=3600",
            body: include_bytes!("../assets/app.js"),
        }),
        "/i18n.js" => Some(UiAsset {
            content_type: "text/javascript; charset=utf-8",
            cache_control: "public, max-age=3600",
            body: include_bytes!("../assets/i18n.js"),
        }),
        "/brand-icon.png" => Some(UiAsset {
            content_type: "image/png",
            cache_control: "public, max-age=3600",
            body: include_bytes!("../assets/brand-icon.png"),
        }),
        "/favicon.png" => Some(UiAsset {
            content_type: "image/png",
            cache_control: "public, max-age=3600",
            body: include_bytes!("../assets/favicon.png"),
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
        assert_eq!(asset("/brand-icon.png").unwrap().content_type, "image/png");
        assert_eq!(asset("/favicon.png").unwrap().content_type, "image/png");
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
        // The `<meta>` copy drops `frame-ancestors` (ignored there, warned
        // about); the header is the one that still carries it.
        assert!(!META_CONTENT_SECURITY_POLICY.contains("frame-ancestors"));
        assert!(!META_CONTENT_SECURITY_POLICY.contains("unsafe-inline"));
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
        assert!(html.contains(&format!("content=\"{META_CONTENT_SECURITY_POLICY}\"")));
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
            "#878f99",
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
        let client = text("/client.js");
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
        assert!(javascript.contains(
            "mode === \"deep\" ? \"/deep\" : mode === \"answer\" ? \"/answer\" : \"/search\""
        ));
        // Transport is centralized in client.js: requests go through the shared
        // client, which owns fetch + POST + JSON + authorization injection.
        assert!(javascript.contains("globalThis.api.post(endpoint"));
        assert!(client.contains("fetch(path"));
        assert!(client.contains("method: \"POST\""));
        assert!(client.contains("\"Content-Type\": \"application/json\""));
        assert!(client.contains("body: JSON.stringify(payload)"));
        assert!(javascript.contains("body.page = store.get().page"));
        assert!(javascript.contains("headers.Authorization"));
        assert!(!javascript.contains("searchParams"));
        // Search/deep/answer routing is POST-only; GET exists for the local
        // read panels (status/history/saved) and is never used for retrieval.
        assert!(!javascript.contains("globalThis.api.get(endpoint"));
        // Pagination is server-side only: no local windowing of the result set.
        assert!(!javascript.contains("serverPagination"));
        assert!(!javascript.contains("state.items.slice"));
        assert!(!client.contains("state.items.slice"));
        assert!(javascript.contains("body.page_size = PAGE_SIZE"));
        assert!(html.contains("id=\"deep-max-fetches\""));
        assert!(html.contains("deep-target-checkbox"));
        // The split runtime dependencies load in order: store before app.
        let store_index = html.find("src=\"/store.js\"").expect("store.js is loaded");
        let app_index = html.find("src=\"/app.js\"").expect("app.js is loaded");
        assert!(store_index < app_index, "store.js must load before app.js");
        assert!(javascript.contains("body.targets = selectedTargets"));
        assert!(javascript.contains("body.max_fetches = Number(deepMaxFetchesInput.value)"));
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
        let client = text("/client.js");
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
        // Local-domain reads go through the shared client; DELETE is defined
        // centrally and never issues a GET for these surfaces.
        assert!(javascript.contains("globalThis.api.get(\"/status\""));
        assert!(javascript.contains("`/history?limit=${LIST_LIMIT}`"));
        assert!(javascript.contains("`/saved?limit=${LIST_LIMIT}`"));
        assert!(!javascript.contains("`/saved?limit=\" +"));
        assert!(client.contains("method: \"DELETE\""));
        // Saved payloads stay bounded and never carry extracted content.
        assert!(javascript.contains("MAX_SAVED_PAYLOAD_BYTES"));
        assert!(!javascript.contains("documentValue.content,"));
        // Every request to a protected surface carries the in-memory token
        // injected by the shared client from the app-level auth helper.
        assert!(javascript.contains("globalThis.authHeaders ="));
        assert!(client.contains("globalThis.authHeaders()"));
    }

    #[test]
    fn marginalia_credential_ui_reflects_the_credential_source() {
        let javascript = text("/app.js");
        let catalog = text("/i18n.js");
        // The UI keys the "liquidate" affordance off the provenance the server
        // reports, so it never offers to erase a key it does not own (an
        // environment variable) and always explains where the key comes from.
        assert!(javascript.contains("credential_source"));
        assert!(javascript.contains("marginaliaCredentialSource"));
        assert!(javascript.contains("source !== \"ephemeral\""));
        assert!(catalog.contains("marginaliaStatusEnvironment"));
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

    #[test]
    fn marginalia_surface_copy_is_catalog_driven() {
        let javascript = text("/app.js");
        let catalog = text("/i18n.js");
        // The provider-credential button and its whole dialog must resolve
        // their copy through the catalog like every other surface, so a
        // non-Spanish locale never shows a half-translated window (this is the
        // one place an operator types a secret).
        for key in [
            "marginaliaKeyButton",
            "marginaliaClearButton",
            "marginaliaDialogHeading",
            "marginaliaDialogHelp",
            "marginaliaKeyPlaceholder",
            "marginaliaSave",
            "marginaliaCancel",
        ] {
            assert!(catalog.contains(key), "missing catalog key: {key}");
            assert!(
                javascript.contains(&format!("t(\"{key}\")")),
                "app.js does not resolve {key} through the catalog"
            );
        }
    }

    #[test]
    fn strategist_toggle_reports_an_enabled_but_unavailable_state() {
        let javascript = text("/app.js");
        let catalog = text("/i18n.js");
        // A configured strategist that is switched on without a usable
        // credential must say so, mirroring the AI-summary `answerHint`.
        assert!(catalog.contains("strategistHintMissingCredential"));
        assert!(javascript.contains("t(\"strategistHintMissingCredential\")"));
        assert!(javascript.contains("strategistToggleStatus.hidden"));
        assert!(javascript.contains("strategistStatus.configured && strategistStatus.enabled"));
    }

    #[test]
    fn storage_backed_panels_are_gated_on_storage_availability() {
        let javascript = text("/app.js");
        // `/history` and `/saved` answer 503 when persistence is off; the panel
        // refresh must not request them unless `/status` reports storage as on
        // and healthy, and must hide the panels otherwise.
        assert!(javascript.contains("async function refreshPanels"));
        assert!(javascript.contains("const status = await fetchServiceState()"));
        assert!(javascript.contains("if (storage.enabled && storage.available)"));
        assert!(javascript.contains("historyPanel.hidden = true"));
        assert!(javascript.contains("savedPanel.hidden = true"));
    }

    #[test]
    fn source_selector_maps_three_choices_onto_provider_enabled() {
        let html = text("/index.html");
        let javascript = text("/app.js");
        let catalog = text("/i18n.js");
        // A single-choice segmented control (Marginalia / SearXNG / Ambos)
        // must exist in the search panel and resolve its copy through the
        // catalog, not half-hardcoded.
        assert!(html.contains("id=\"provider-selector\""));
        assert!(html.contains("name=\"provider-source\""));
        assert!(html.contains("value=\"marginalia\""));
        assert!(html.contains("value=\"searxng\""));
        assert!(html.contains("value=\"both\""));
        assert!(catalog.contains("providerSelectorLabel"));
        assert!(catalog.contains("providerSourceBoth"));
        assert!(javascript.contains("t(\"providerSelectorLabel\")"));
        // The three choices reduce to exactly the two per-provider toggles the
        // backend speaks (`POST /providers/{name}/enabled`), keeping both the
        // selector and the detailed panel on the same `providers.enabled` list.
        assert!(javascript.contains("providerSelectorOptions.addEventListener(\"change\""));
        assert!(javascript.contains("applyProviderSource"));
        assert!(javascript.contains("PROVIDER_SOURCE_TARGET"));
        assert!(javascript.contains("marginalia: [\"marginalia\"]"));
        assert!(javascript.contains("searxng: [\"searxng\"]"));
        assert!(javascript.contains("both: [\"marginalia\", \"searxng\"]"));
        assert!(javascript.contains("syncProviderSelector"));
        assert!(javascript.contains("`/providers/${encodeURIComponent(enabled)}/enabled`"));
    }
}
