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
            cache_control: "no-cache",
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
            cache_control: "no-cache",
            body: include_bytes!("../assets/app.js"),
        }),
        "/i18n.js" => Some(UiAsset {
            content_type: "text/javascript; charset=utf-8",
            cache_control: "no-cache",
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

    /// Every asset is provided by `include_bytes!`, so the bytes the tests read
    /// here are, by construction, the bytes the binary serves: a rebuild cannot
    /// leave one behind the other. What it *can* leave behind is the binary
    /// itself — `target/release/amatl` older than `assets/` serves the markup and
    /// scripts from before a UI change and makes the change look missing (the
    /// H-R01 staleness of the 2026-11-14 re-audit). This pins the property that
    /// makes the mistake diagnosable: the asset the binary embeds and the file on
    /// disk are byte-identical, and the UI surface a reader of `audit/` checks by
    /// id is really in there.
    #[test]
    fn embedded_assets_are_the_files_on_disk_and_carry_the_current_surface() {
        for (name, bytes) in [
            (
                "index.html",
                include_bytes!("../assets/index.html") as &[u8],
            ),
            ("styles.css", include_bytes!("../assets/styles.css")),
            ("client.js", include_bytes!("../assets/client.js")),
            ("store.js", include_bytes!("../assets/store.js")),
            ("app.js", include_bytes!("../assets/app.js")),
            ("i18n.js", include_bytes!("../assets/i18n.js")),
        ] {
            assert_eq!(
                bytes,
                std::fs::read(format!("{}/assets/{name}", env!("CARGO_MANIFEST_DIR")))
                    .unwrap_or_else(|error| panic!("assets/{name} is readable: {error}")),
                "assets/{name} changed since the embed was compiled: rebuild before \
                 judging UI behaviour, a stale binary serves the previous markup"
            );
        }
        // The ids the UI audits grep for, so a rename cannot pass silently.
        let index = text("/");
        for id in [
            "id=\"model-options\"",
            "id=\"local-token\"",
            "id=\"access-summary\"",
        ] {
            assert!(
                index.contains(id),
                "index.html no longer carries {id}: UI audit citations in `audit/` \
                 would point at markup that does not exist"
            );
        }
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
        assert!(css.contains("@media (orientation: landscape)"));
        assert!(css.contains("#search-form > .model-options { grid-column: 1 / -1; }"));
        assert!(
            css.contains(".filters summary { width: fit-content; min-height: var(--control-h);")
        );
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
    fn control_geometry_uses_the_shared_sizing_scale() {
        // Regression guard for the drift this fixed: buttons and inputs used to
        // pick their own radius/height (radii 8/6/4px; heights 46/44/40/38px),
        // so controls doing the same job did not line up. Geometry must now
        // come from the tokens, not magic numbers.
        let css = text("/styles.css");
        for token in [
            "--radius-sm:",
            "--radius-md:",
            "--radius-pill:",
            "--control-h:",
            "--control-h-sm:",
        ] {
            assert!(css.contains(token), "missing sizing token: {token}");
        }
        // The base controls resolve to the scale, not to bare values.
        assert!(css.contains("min-height: var(--control-h);"));
        assert!(css.contains("border-radius: var(--radius-md);"));
        // No stray literal radii survive outside the token declarations — the
        // only allowed literal is the loading dot's circle.
        let stray_radii = css
            .lines()
            .filter(|line| {
                line.contains("border-radius:")
                    && !line.contains("var(--radius")
                    && !line.contains("--radius-")
                    && !line.contains("50%")
            })
            .count();
        assert_eq!(stray_radii, 0, "a literal border-radius escaped the scale");
        // Ghost/compact buttons declare their own (smaller) tier explicitly,
        // instead of inheriting the base height and looking padded by accident.
        assert!(css.contains(".ghost-action {"));
        assert!(css.contains("min-height: var(--control-h-sm);"));
    }

    #[test]
    fn hidden_attribute_is_authoritative_over_component_layout() {
        // Regression guard: the JS shows/hides panels by flipping the `hidden`
        // attribute, but a component that sets its own `display:` (e.g.
        // `.provider-selector { display: flex }`) would beat the UA
        // `[hidden] { display: none }` and stay visible while logically hidden.
        // The stylesheet must carry a global author rule that makes `hidden`
        // win, and the display-carrying components the JS toggles must be
        // exactly the ones it covers.
        let css = text("/styles.css");
        assert!(
            css.contains("[hidden] { display: none !important; }"),
            "the global [hidden] override is missing from styles.css"
        );
        for component in [
            ".credential-actions {",
            ".credential-group {",
            ".fragment-signals {",
            ".result-index {",
        ] {
            assert!(
                css.contains(component),
                "component expected to rely on [hidden] is gone: {component}"
            );
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
            "uiLanguageNameEn",
            "uiLanguageNameEs",
            "strategistToggleLabel",
            "strategistHelp",
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
    fn declarative_i18n_markup_has_catalog_coverage() {
        let html = text("/index.html");
        let catalog = text("/i18n.js");
        for attribute in [
            "data-i18n=\"",
            "data-i18n-placeholder=\"",
            "data-i18n-aria-label=\"",
        ] {
            let mut remaining = html.as_str();
            while let Some(start) = remaining.find(attribute) {
                let value = &remaining[start + attribute.len()..];
                let end = value.find('"').expect("closed i18n attribute");
                let key = &value[..end];
                assert!(
                    catalog.contains(&format!("{key}:")),
                    "missing catalog key: {key}"
                );
                remaining = &value[end + 1..];
            }
        }
        assert!(html.contains("id=\"search-button\" type=\"submit\" data-mode=\"search\" disabled"));
        assert!(html.contains("aria-describedby=\"strategist-help strategist-toggle-status\""));
        assert!(html
            .contains("id=\"strategist-help\" class=\"field-help\" data-i18n=\"strategistHelp\""));
    }

    #[test]
    fn default_markup_matches_default_locale_and_explains_search_modes() {
        let html = text("/index.html");
        assert!(html.contains("<html lang=\"en\">"));
        assert!(html.contains("AMATL — Search and evidence"));
        assert!(html.contains("id=\"search-mode-help\""));
        assert!(html.contains("aria-describedby=\"search-mode-help\""));
        assert!(!html.contains("AMATL — Búsqueda y evidencia"));
        assert!(!html.contains(">Busquemos</h1>"));
    }

    /// The language picker names each locale in its own language (`English`,
    /// `Español`) and the names live in the catalog, not the markup. Before
    /// this the options were hardcoded `English`/`Spanish`, so a visitor whose
    /// interface had resolved to Spanish still read "Spanish" in the picker --
    /// an English name under a Spanish UI.
    #[test]
    fn language_options_name_each_locale_in_its_own_language_from_the_catalog() {
        let html = text("/index.html");
        let catalog = text("/i18n.js");
        // The options carry a catalog key instead of a literal label.
        assert!(html.contains("id=\"ui-language\""));
        assert!(html.contains("data-i18n=\"uiLanguageNameEn\""));
        assert!(html.contains("data-i18n=\"uiLanguageNameEs\""));
        // The old hardcoded English name for the Spanish locale is gone.
        assert!(!html.contains(">Spanish</option>"));
        // Both catalog blocks carry the names, and Spanish's own name is
        // Spanish, not the English word for it.
        assert!(catalog.contains("uiLanguageNameEn: \"English\""));
        assert!(catalog.contains("uiLanguageNameEs: \"Español\""));
    }

    /// The empty-source note must point somewhere. It used to end at
    /// "Ninguna fuente disponible." -- a dead end that named the problem and
    /// offered no next step.
    #[test]
    fn empty_source_note_says_what_to_do_next() {
        let catalog = text("/i18n.js");
        assert!(catalog.contains("Habilita una para poder buscar."));
        assert!(catalog.contains("Enable one to be able to search."));
    }

    #[test]
    fn unavailable_backend_states_are_fail_closed_in_the_ui() {
        let html = text("/index.html");
        let javascript = text("/app.js");
        assert!(html.contains("id=\"search-button\""));
        assert!(javascript.contains("searchButton.disabled = !searchAvailable"));
        assert!(javascript.contains("answerButton.hidden = !answerRelevant"));
        assert!(javascript.contains("answer-mode-hidden"));
        assert!(javascript.contains("checkbox.disabled = provider.status !== \"available\""));
        assert!(
            javascript.contains("answerEnabledToggle.disabled = answerStatus.available === false")
        );
        assert!(javascript.contains("strategistStatus.available !== false"));
        assert!(javascript.contains("strategistEnabledToggle.checked = false"));
        assert!(javascript.contains("error.code"));
        assert!(text("/client.js").contains("payload?.error?.code"));
    }

    /// `reportFailure` must never read a bare `message` identifier.
    ///
    /// Commit `ac11624a` replaced `const message = error.message;` with direct
    /// `error.code`/`error.status` handling but left the `504` branch reading
    /// the removed local, so any failure reaching that branch threw
    /// `ReferenceError: message is not defined` instead of reporting a timeout
    /// or a generic error: the operator saw no status at all. Nothing else
    /// catches it, because the asset tests assert string presence rather than
    /// JS semantics and the CI pipeline ships no JS linter.
    #[test]
    fn report_failure_reads_only_error_properties_never_a_bare_message() {
        let javascript = text("/app.js");
        let body = javascript
            .split_once("function reportFailure(error) {")
            .expect("reportFailure is defined in app.js")
            .1
            .split_once("\n}\n")
            .expect("reportFailure closes at column zero")
            .0;
        let mut rest = body;
        let mut references = 0;
        while let Some(index) = rest.find("message") {
            assert!(
                rest[..index].ends_with('.'),
                "bare `message` identifier in reportFailure near: {}",
                &rest[index.saturating_sub(24)..]
            );
            references += 1;
            rest = &rest[index + "message".len()..];
        }
        assert!(
            references >= 3,
            "reportFailure must inspect error.message on every branch"
        );
        assert!(javascript.contains("error.message.includes(\"504\")"));
    }

    /// BH-01/BH-02 (2026-11-18 audit). Two field defects, both born of the
    /// same mistake — keying a decision off something other than the structured
    /// fields the shared client actually exposes:
    ///
    /// - BH-01: `createServerClient` always sent `tools: []`, and
    ///   `ClientIdentity::allows_tool` denies on an empty allowlist, so an
    ///   "MCP"-scoped client created from the UI could never call a single MCP
    ///   tool — the checkbox was a dead control.
    /// - BH-02: the error branches classified 401/403 by scanning the *message
    ///   text* for status digits. Real error bodies carry a catalog message
    ///   with no digits (`{"code":"unauthorized",...}`), so the regex never
    ///   matched and an operator without admin scope was shown "the key was
    ///   rejected" in the one dialog where a secret is typed.
    ///
    /// The asset tests assert string presence, so nothing here executes the JS:
    /// this locks the two properties the fix introduced, exactly as the audit's
    /// live probe observed them.
    #[test]
    fn mcp_clients_carry_the_tool_vocabulary_and_errors_classify_on_status() {
        let javascript = text("/app.js");
        // BH-01: the MCP tool vocabulary is declared once and sent in full for
        // an MCP-scoped client, so the allowlist is never empty.
        assert!(
            javascript.contains(
                "const MCP_TOOL_NAMES = [\"search\", \"deep_search\", \"fetch\", \"providers\", \"status\", \"answer\"]"
            ),
            "the MCP tool vocabulary must mirror the server's MCP_TOOLS constant"
        );
        assert!(
            javascript.contains("scopes.includes(\"mcp\") ? MCP_TOOL_NAMES.slice() : []"),
            "an MCP-scoped client must be created with the full tool vocabulary"
        );
        assert!(
            !javascript.contains("const body = { id, scopes, tools: [] };"),
            "the old always-empty allowlist must not come back"
        );
        // BH-02: the remaining error branches read `error.status`/`error.code`
        // (set by client.js) and keep the message check only as a fallback. The
        // `applyProviderSource` branch this originally covered is gone with the
        // retired source selector.
        assert!(
            javascript.contains("const status = Number(error && error.status) || 0;"),
            "saveMarginaliaKey must read the HTTP status, not parse the message"
        );
        assert!(
            javascript.contains("code === \"scope_denied\""),
            "saveMarginaliaKey must recognise the catalog's 403 code"
        );
        assert!(
            javascript.contains("status === 409 || message.includes(\"409\")"),
            "createServerClient must report the duplicate-id catalog case"
        );
        assert!(
            !javascript.contains("/API Error: (\\d+)/"),
            "no branch may parse the HTTP status out of the message text"
        );
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
    fn marginalia_key_dialog_submits_as_a_keyboard_operable_form() {
        let html = text("/index.html");
        let javascript = text("/app.js");
        assert!(html.contains("<form id=\"marginalia-form\""));
        assert!(html.contains("<button type=\"submit\" id=\"marginalia-save\">"));
        assert!(javascript
            .contains("const marginaliaForm = document.querySelector(\"#marginalia-form\")"));
        assert!(javascript.contains("event.preventDefault();"));
        assert!(
            javascript.contains("marginaliaForm.addEventListener(\"submit\", saveMarginaliaKey)")
        );
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
    fn retired_source_selector_is_gone_and_cannot_return() {
        let html = text("/index.html");
        let javascript = text("/app.js");
        let catalog = text("/i18n.js");
        // Marginalia is the only shipped source, so a source selector would be a
        // single-choice control with nothing to choose: it is removed entirely,
        // not left as a dead three-way radio group.
        assert!(!html.contains("id=\"provider-selector\""));
        assert!(!html.contains("name=\"provider-source\""));
        assert!(!html.contains("value=\"searxng\""));
        assert!(!html.contains("value=\"all\""));
        assert!(!javascript.contains("PROVIDER_SOURCE_TARGET"));
        assert!(!javascript.contains("applyProviderSource"));
        assert!(!javascript.contains("syncProviderSelector"));
        assert!(!javascript.contains("PROVIDER_SOURCE_NAME"));
        // No JS may query an element that no longer renders: a `querySelector`
        // for the removed class returns null and the chained `.closest(...)`
        // throws, aborting the rest of the handler. This is the defect the
        // 2026-11-18 connector audit caught (H-01): the HTML was removed but a
        // translation line still queried `.provider-selector-option`.
        assert!(!javascript.contains("provider-selector"));
        assert!(!javascript.contains("providerSelector"));
        assert!(!javascript.contains("provider-source"));
        // The catalog carries no copy for a control that no longer exists.
        assert!(!catalog.contains("providerSelectorLabel"));
        assert!(!catalog.contains("providerSourceSearxng"));
        assert!(!catalog.contains("providerSourceAll"));
    }

    #[test]
    fn only_the_shipped_sources_are_selectable_in_the_ui() {
        let html = text("/index.html");
        let javascript = text("/app.js");
        let catalog = text("/i18n.js");
        // The only shipped source is Marginalia, and the UI exposes a control
        // for it through the detailed source panel (driven by `/providers`),
        // never a selector or a key surface for a retired source.
        assert!(!html.contains("value=\"searxng\""));
        assert!(!javascript.contains("searxng"));
        assert!(!catalog.contains("SearXNG"));
        // Marginalia is the only credential-gated source, so its key surface is
        // the only one in the catalog.
        assert!(catalog.contains("marginaliaKeyButton"));
    }

    /// The history rows render a result count, and a single result must read
    /// "1 resultado."/"1 result.", never the bare plural.
    ///
    /// Regression guard for a real defect: `historyLabel` called
    /// `t("resultsCount")` directly while the results header went through
    /// `countLabel`/`pluralKey`, so one query showed as "1 of 1 result" above
    /// and "1 resultados." in the history panel below. The singular keys exist
    /// in both locales; the fix routes history through the same helper.
    #[test]
    fn history_count_uses_the_plural_aware_helper() {
        let javascript = text("/app.js");
        let catalog = text("/i18n.js");
        // The catalog ships both forms in every locale.
        assert!(catalog.contains("resultsCount:"));
        assert!(catalog.contains("resultsCountOne:"));
        // The history label selects between them instead of hardcoding the
        // plural, so the two views of one number cannot disagree. It skips the
        // `-1` "total never known" sentinel so an unreported count is not
        // rendered as "-1 results".
        assert!(
            javascript.contains("countLabel(entry.total_results, \"resultsCount\")"),
            "historyLabel must pluralize through countLabel"
        );
        assert!(
            javascript.contains("entry.total_results >= 0"),
            "historyLabel must skip the unknown-total sentinel"
        );
        assert!(
            !javascript.contains("${entry.total_results} ${t(\"resultsCount\")}"),
            "historyLabel still bypasses pluralization"
        );
    }

    /// A native provider page omits `total_results`, so the page length is not
    /// a whole-set size. The UI must keep paging forward (a full page implies a
    /// further page may exist) instead of collapsing to a single page and
    /// hiding the paginator (BH-001).
    #[test]
    fn unknown_total_keeps_pagination_reachable() {
        let javascript = text("/app.js");
        let store = text("/store.js");
        // The store tracks the two facts the render needs.
        assert!(store.contains("knownTotal"));
        assert!(store.contains("hasMore"));
        // The payload handler distinguishes a known total from a page length.
        assert!(
            javascript.contains("const knownTotal = typeof payload.total_results === \"number\"")
        );
        assert!(javascript.contains("items.length >= pageSize"));
        // The render shows the paginator and enables Next when more may exist.
        assert!(javascript.contains("state.knownTotal"));
        assert!(javascript.contains("state.hasMore"));
        // The Next handler advances when the total is unknown but more may exist.
        assert!(javascript.contains("const canAdvance = state.knownTotal"));
    }

    /// The token field is a credential, not a query filter: it lives in its own
    /// disclosure and no longer pollutes the filter summary.
    #[test]
    fn token_field_is_grouped_as_access_not_as_a_filter() {
        let html = text("/index.html");
        let javascript = text("/app.js");
        let catalog = text("/i18n.js");
        assert!(html.contains("class=\"filters search-filters\""));
        assert!(html.contains("class=\"filters evidence-options\""));
        assert!(html.contains("class=\"filters source-credentials\""));
        assert!(html.contains("class=\"filters access-filters\""));
        assert!(html.contains("id=\"answer-hint\" class=\"field-help\" hidden"));
        assert!(html.contains("<div class=\"search-guidance\">"));
        assert!(html.contains("<details class=\"clients-panel\" hidden>"));
        assert!(html.contains("<details class=\"operator-tools\">"));
        assert!(html.contains("operatorToolsLabel"));
        assert!(
            html.find("id=\"query\"").unwrap() < html.find("class=\"operator-tools\"").unwrap()
        );
        assert!(text("/styles.css").contains(".search-guidance { grid-column: 1 / -1;"));
        assert!(text("/i18n.js").contains("mockSourcesLabel: \"Fuentes de prueba\""));
        assert!(!html.contains("id=\"answer-button\" class=\"secondary-action is-unavailable\" type=\"submit\" data-mode=\"answer\" disabled"));
        assert!(html.contains("id=\"access-summary\" data-i18n=\"accessLabel\""));
        assert!(html.contains("id=\"local-token\""));
        assert!(catalog.contains("accessLabel:"));
        assert!(catalog.contains("AMATL_SERVER_TOKEN"));
        assert!(catalog.contains("openssl rand -hex 32"));
        // The access panel must be a *sibling* of the filter panel, not nested
        // inside it: nesting would leave the token reachable only after opening
        // the filters, which is exactly the grouping confusion this fixed.
        // Between the token panel and the form close there is exactly one
        // `</details>` (the access panel's own); if it were nested inside the
        // filters panel there would be two.
        let after_access = html
            .split("class=\"filters access-filters\"")
            .nth(1)
            .expect("access panel exists");
        let closers_before_form_end = after_access
            .split("</form>")
            .next()
            .expect("form closes")
            .matches("</details>")
            .count();
        assert_eq!(
            closers_before_form_end, 1,
            "the access panel is nested inside the filters panel instead of beside it"
        );
        // The token is absent from the filter summary and drives its own.
        assert!(!javascript.contains("if (tokenInput.value) active.push"));
        let search_filters = html
            .split("class=\"filters search-filters\"")
            .nth(1)
            .unwrap()
            .split("</details>")
            .next()
            .unwrap();
        assert!(search_filters.contains("id=\"language\""));
        assert!(!search_filters.contains("deep-max-fetches"));
        assert!(!search_filters.contains("marginalia-key-button"));
        assert!(javascript.contains("authentication_required !== false"));
        assert!(javascript.contains("providerStatus.hidden = mockOnly"));
        assert!(javascript.contains("function updateAccessSummary"));
        assert!(javascript.contains("updateAccessSummary();"));
        // A token validation error opens the panel that actually holds it.
        assert!(
            javascript.contains("if (accessDetails.contains(field)) accessDetails.open = true;")
        );
        assert!(javascript
            .contains("if (evidenceDetails.contains(field)) evidenceDetails.open = true;"));
    }

    /// The side panels keep a shared, drift-proof control scale: the checkbox
    /// box comes from one token so the toggle/scope rows cannot diverge, and
    /// the history panel's height is decoupled from its entry count.
    #[test]
    fn side_panel_sizing_uses_shared_tokens_and_bounds_the_history() {
        let css = text("/styles.css");
        assert!(css.contains("--checkbox-size:"));
        assert!(css.contains("width: var(--checkbox-size);"));
        assert!(css.contains("height: var(--checkbox-size);"));
        assert!(!css.contains("width: 1rem;\n  height: 1rem;"));
        // The history scroll region bounds the panel height.
        assert!(css.contains(".panel-scroll {"));
        assert!(css.contains("max-height: 22rem;"));
        // The provider panel aligns with its grid siblings instead of carrying
        // the legacy standalone `margin-top: 2rem`.
        assert!(
            !css.contains(".provider-status {\n  margin-top: 2rem;"),
            "provider-status still carries the legacy standalone margin"
        );
    }
}
