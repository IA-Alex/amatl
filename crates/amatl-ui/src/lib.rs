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
        let surface = format!("{}{}", text("/index.html"), text("/app.js")).to_lowercase();
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
    fn search_flow_uses_the_public_contract_and_safe_dom_apis() {
        let html = text("/index.html");
        let javascript = text("/app.js");
        for field in [
            "name=\"q\"",
            "name=\"lang\"",
            "name=\"region\"",
            "name=\"filetype\"",
        ] {
            assert!(html.contains(field), "missing UI field: {field}");
        }
        assert!(javascript.contains("payload.schema_version !== \"1\""));
        assert!(javascript.contains("payload.status === \"partial_success\""));
        assert!(javascript.contains("result.canonical_url"));
        assert!(javascript.contains("parsed.protocol === \"http:\""));
        assert!(javascript.contains("parsed.protocol === \"https:\""));
        assert!(javascript.contains("textContent"));
        assert!(!javascript.contains("innerHTML"));
        assert!(!javascript.contains("document.write"));
    }
}
