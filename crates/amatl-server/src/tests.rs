use super::*;
use axum::{body::to_bytes, http::Request};
use std::io::Write;
use std::sync::{Arc, Mutex, Once, OnceLock, RwLock};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::Layer;

const TOKEN: &str = "0123456789abcdef0123456789abcdef";

async fn app() -> Router {
    build_router(
        AmatlService::new(amatl_core::Config::default(), true).await,
        Some(TOKEN.into()),
    )
    .await
    .unwrap()
}

async fn isolated_app() -> Router {
    let mut config = amatl_core::Config::default();
    config.data_policy.profile = amatl_core::SecurityProfile::Isolated;
    config.data_policy.egress = amatl_core::EgressPolicy::Deny;
    config.data_policy.inference = amatl_core::InferenceMode::LocalOnly;
    config.validate().unwrap();
    build_router(AmatlService::new(config, true).await, Some(TOKEN.into()))
        .await
        .unwrap()
}

/// A non-mock service with exactly one source enabled and governed: the
/// single-source deployment readiness must not penalize (D2). The other shipped
/// sources stay declared and disabled, as it is in a real deployment that runs
/// only Marginalia.
async fn marginalia_only_app() -> Router {
    let mut config = amatl_core::Config::default();
    let runtime = config.providers.entry("marginalia");
    runtime.adapter_version = Some("marginalia-v1".into());
    runtime.approval_status = amatl_core::ApprovalStatus::Approved;
    runtime.reviewed_at = Some(today_iso());
    runtime.reviewer = Some("owner".into());
    runtime.terms_url = Some("https://www.marginalia.nu/".into());
    runtime.terms_version_or_date = Some("2026-08-01".into());
    runtime.allowed_access_method = Some("official_api".into());
    runtime.plan_or_contract = Some("contract-1".into());
    runtime.rate_limit = Some("1 qps".into());
    runtime.cost_model = Some("0".into());
    runtime.data_handling_notes = Some("no cache".into());
    runtime.operational_risk = Some("network dependency".into());
    config.providers.enabled = vec!["marginalia".into()];
    config.validate().unwrap();
    let service = AmatlService::new(config, false).await;
    // Marginalia is key-gated: it only reports `Available` with a credential.
    // Inject one through the supported operator path so the test does not depend
    // on the host environment (a clean CI runner has no MARGINALIA_API_KEY).
    service
        .set_provider_credential("marginalia", "marginalia-test-key")
        .unwrap();
    build_router(service, Some(TOKEN.into())).await.unwrap()
}

/// A non-mock service with nothing enabled: no source can be called at all.
async fn no_source_app() -> Router {
    let config = amatl_core::Config::default();
    config.validate().unwrap();
    build_router(AmatlService::new(config, false).await, Some(TOKEN.into()))
        .await
        .unwrap()
}

fn request(path: &str) -> axum::http::request::Builder {
    Request::builder().uri(path).header(HOST, "localhost:8080")
}

fn authorized(path: &str) -> axum::http::request::Builder {
    request(path).header(AUTHORIZATION, format!("Bearer {TOKEN}"))
}

async fn json_body(response: Response) -> serde_json::Value {
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// Like json_body but returns None instead of panicking on parse errors.
async fn json_body_opt(response: Response) -> Option<serde_json::Value> {
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).ok()
}

#[tokio::test]
async fn health_is_lightweight_public_and_hardened() {
    let response = app()
        .await
        .oneshot(
            request("/health")
                .header(REQUEST_ID_HEADER, "client-supplied-id")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let request_id = response.headers()[REQUEST_ID_HEADER]
        .to_str()
        .unwrap()
        .to_owned();
    assert_eq!(request_id.len(), 49);
    assert_ne!(request_id, "client-supplied-id");
    assert!(request_id
        .bytes()
        .all(|value| value.is_ascii_hexdigit() || value == b'-'));
    assert!(response.headers().contains_key("content-security-policy"));
    assert_eq!(
        response.headers()["x-content-type-options"],
        HeaderValue::from_static("nosniff")
    );
    assert!(!response.headers().contains_key("server"));
    let body = json_body(response).await;
    assert_eq!(body, json!({"schema_version": "1", "status": "ok"}));

    let second = app()
        .await
        .oneshot(request("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_ne!(
        request_id,
        second.headers()[REQUEST_ID_HEADER].to_str().unwrap()
    );
}

#[tokio::test]
async fn ready_reports_readiness_without_leaking_deployment_internals() {
    let response = app()
        .await
        .oneshot(request("/ready").body(Body::empty()).unwrap())
        .await
        .unwrap();
    // Public like /health: no bearer required.
    assert_ne!(response.status(), StatusCode::UNAUTHORIZED);
    let status = response.status();
    assert!(
        status == StatusCode::OK || status == StatusCode::SERVICE_UNAVAILABLE,
        "unexpected status: {status}"
    );

    let body = json_body(response).await;
    // Aggregate shape only. Source names, error codes and paths describe the
    // deployment and stay behind /status, which requires the read scope.
    let object = body.as_object().expect("object body");
    assert_eq!(
        object
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>(),
        [
            "schema_version",
            "sources_available",
            "sources_usable",
            "status",
            "storage_ok"
        ]
        .into()
    );
    assert_eq!(body["schema_version"], "1");
    assert!(body["storage_ok"].is_boolean());
    assert!(body["sources_available"].is_u64());
    assert!(body["sources_usable"].is_u64());
    let rendered = body.to_string();
    for internal in ["provider_", "mock", "sqlite", "/", "path"] {
        assert!(
            !rendered.contains(internal),
            "readiness body leaked {internal:?}: {rendered}"
        );
    }
}
/// D2: a deployment that enables exactly one governed source is ready. The
/// shipped sources it left disabled are a configuration choice, so `/ready`
/// must not answer 503 — a load balancer would otherwise drain an instance that
/// serves every search correctly.
#[tokio::test]
async fn ready_stays_ok_when_the_only_enabled_source_is_available() {
    let response = marginalia_only_app()
        .await
        .oneshot(request("/ready").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["status"], "ok");
    assert_eq!(body["storage_ok"], true);
    // Readiness counts what is available, not what is declared.
    assert_eq!(body["sources_available"], 1);
}

/// The other half of the same rule: ignoring disabled sources must not make an
/// instance with nothing enabled look ready. It cannot search at all.
#[tokio::test]
async fn ready_reports_unavailable_when_no_source_is_enabled() {
    let response = no_source_app()
        .await
        .oneshot(request("/ready").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = json_body(response).await;
    assert_eq!(body["status"], "degraded");
    assert_eq!(body["sources_available"], 0);
}

/// `/status` and `/ready` are two views of one decision: the operator view
/// names the disabled sources, and readiness — the same `status` field — still
/// says the instance is serving. Diverging here is what made the UI gate search
/// on a service it believed was unusable.
#[tokio::test]
async fn status_and_ready_agree_on_a_single_source_deployment() {
    let router = marginalia_only_app().await;
    let ready = router
        .clone()
        .oneshot(request("/ready").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let ready_body = json_body(ready).await;
    let status = router
        .oneshot(authorized("/status").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(status.status(), StatusCode::OK);
    let status_body = json_body(status).await;
    assert_eq!(ready_body["status"], status_body["status"]);
    assert_eq!(ready_body["sources_available"], 1);
    assert_eq!(status_body["sources"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn health_stays_a_pure_liveness_probe_independent_of_readiness() {
    // /health must not acquire service state: an orchestrator uses it to decide
    // whether to restart the process, which cannot depend on SQLite or on a
    // provider being reachable.
    let response = app()
        .await
        .oneshot(request("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        json_body(response).await,
        json!({"schema_version": "1", "status": "ok"})
    );
}

#[tokio::test]
async fn protected_api_requires_bearer_and_preserves_search_contract() {
    let unauthorized = app()
        .await
        .oneshot(request("/search?q=rust").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        unauthorized.headers()[WWW_AUTHENTICATE],
        HeaderValue::from_static("Bearer")
    );

    let response = app()
        .await
        .oneshot(authorized("/search?q=rust").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["schema_version"], "1");
    assert_eq!(body["status"], "success");
    assert_eq!(
        body["results"][0]["canonical_url"],
        "https://example.com/rust"
    );
    assert!(body.get("ranking_v2").is_none());
    assert!(body["results"][0].get("final_url").is_none());

    let response = app()
        .await
        .oneshot(
            authorized("/search")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"q":"rust"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let malformed = app()
        .await
        .oneshot(
            authorized("/search")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from("{"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        json_body(malformed).await["error"]["code"],
        "invalid_request"
    );
}

#[tokio::test]
async fn search_batch_runs_queries_independently_with_a_bounded_envelope() {
    // Auth required, like every protected domain surface.
    let unauthorized = app()
        .await
        .oneshot(request("/search/batch?q=rust").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    // Envelope validation: at least one query.
    let empty = app()
        .await
        .oneshot(authorized("/search/batch").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(empty.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(empty).await["error"]["code"], "invalid_request");

    // Envelope validation: at most MAX_BATCH_QUERIES.
    let too_many = app()
        .await
        .oneshot(
            authorized("/search/batch?q=a&q=b&q=c&q=d&q=e&q=f")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(too_many.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        json_body(too_many).await["error"]["code"],
        "invalid_request"
    );

    // A valid batch returns one outcome per query, in order, each with its
    // own full SearchResponse.
    let response = app()
        .await
        .oneshot(
            authorized("/search/batch?q=rust&q=async")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = json_body(response).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "valid batch should be 200; body={body}"
    );
    assert_eq!(body["schema_version"], "1");
    assert_eq!(body["count"], 2);
    let batches = body["batches"].as_array().unwrap();
    assert_eq!(batches.len(), 2);
    assert_eq!(batches[0]["query"], "rust");
    assert_eq!(batches[0]["response"]["status"], "success");
    assert_eq!(
        batches[0]["response"]["results"][0]["canonical_url"],
        "https://example.com/rust"
    );
    assert_eq!(batches[1]["query"], "async");
    // Mutual exclusivity: successful items carry a response and no error.
    assert!(batches[0].get("error").is_none());
    assert!(batches[0]["response"].is_object());
}

#[tokio::test]
async fn search_batch_isolates_a_failed_query_without_dropping_the_rest() {
    // Empty query text is invalid; that item reports an error while the
    // neighbouring valid query still returns its response.
    let response = app()
        .await
        .oneshot(
            authorized("/search/batch?q=rust&q=&q=async")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "batch envelope stays 200"
    );
    let body = json_body(response).await;
    assert_eq!(body["count"], 3);
    let batch = &body["batches"][1];
    assert_eq!(batch["query"], "");
    assert_eq!(batch["error"], "invalid_query");
    assert!(batch.get("response").is_none());
    // The other two succeeded independently.
    assert_eq!(body["batches"][0]["response"]["status"], "success");
    assert_eq!(body["batches"][2]["response"]["status"], "success");
}

#[tokio::test]
async fn deep_post_exposes_evidence_contract_without_a_local_file_route() {
    let application = isolated_app().await;
    let unauthorized = application
        .clone()
        .oneshot(
            request("/deep")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"q":"rust"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let response = application
        .clone()
        .oneshot(
            authorized("/deep")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"q":"rust"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["schema_version"], "1");
    assert_eq!(body["query"], "rust");
    assert!(body["documents"].is_array());
    assert!(body["evidence"].is_array());
    assert!(body["evidence_v2"].is_array());
    assert!(body["degradations"]
        .as_array()
        .is_some_and(|values| values.iter().any(|value| value["code"] == "egress_denied")));

    let local_ingest = application
        .oneshot(
            authorized("/ingest")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"path":"/etc/passwd"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(local_ingest.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn deep_post_accepts_selected_targets_and_request_fetch_caps() {
    let application = isolated_app().await;
    let response = application
        .clone()
        .oneshot(
            authorized("/deep")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "q": "rust",
                        "targets": [
                            {
                                "url": "https://example.com/rust?utm_source=search",
                                "title": "Rust",
                                "provider": "mock-a"
                            },
                            {
                                "url": "https://example.com/rust",
                                "title": "Duplicate",
                                "provider": "mock-a"
                            }
                        ],
                        "max_fetches": 2
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    // Isolated mode rejects egress, but the single error proves the canonical
    // duplicate was removed before Deep attempted acquisition.
    assert_eq!(body["errors"].as_array().unwrap().len(), 1);
    assert_eq!(body["errors"][0]["providers"][0], "mock-a");

    for body in [
        json!({"q": "rust", "max_fetches": 0}),
        json!({"q": "rust", "max_fetches": 11}),
        json!({"q": "rust", "targets": [{"url": "file:///etc/passwd", "provider": "mock-a"}]}),
    ] {
        let response = application
            .clone()
            .oneshot(
                authorized("/deep")
                    .method(Method::POST)
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            json_body(response).await["error"]["code"],
            "invalid_request"
        );
    }
}

#[tokio::test]
async fn host_and_origin_are_explicitly_validated() {
    let invalid_host = app()
        .await
        .oneshot(
            Request::builder()
                .uri("/health")
                .header(HOST, "attacker.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid_host.status(), StatusCode::BAD_REQUEST);

    let invalid_origin = app()
        .await
        .oneshot(
            authorized("/search?q=rust")
                .header(ORIGIN, "https://attacker.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid_origin.status(), StatusCode::FORBIDDEN);

    let valid_origin = app()
        .await
        .oneshot(
            authorized("/search?q=rust")
                .header(ORIGIN, "http://localhost:8080")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(valid_origin.status(), StatusCode::OK);
    assert_eq!(
        valid_origin.headers()["access-control-allow-origin"],
        HeaderValue::from_static("http://localhost:8080")
    );

    let public_cross_origin = app()
        .await
        .oneshot(
            request("/health")
                .header(ORIGIN, "https://attacker.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(public_cross_origin.status(), StatusCode::FORBIDDEN);

    let public_same_origin = app()
        .await
        .oneshot(
            request("/health")
                .header(ORIGIN, "http://localhost:8080")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(public_same_origin.status(), StatusCode::OK);
    assert_eq!(
        public_same_origin.headers()["access-control-allow-origin"],
        HeaderValue::from_static("http://localhost:8080")
    );
    assert!(
        public_same_origin.headers()["access-control-expose-headers"]
            .to_str()
            .unwrap()
            .split(',')
            .any(|value| value.trim().eq_ignore_ascii_case(REQUEST_ID_HEADER))
    );
}

#[derive(Clone)]
struct LogWriter(Arc<Mutex<Vec<u8>>>);

impl Write for LogWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Serializes the tests that assert on captured logs.
///
/// The capture buffer is process-wide: without this guard two log-asserting
/// tests running in parallel clear each other's events and fail spuriously.
async fn security_log_guard() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}

fn security_logs() -> Arc<Mutex<Vec<u8>>> {
    static CAPTURED: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();
    static INSTALL: Once = Once::new();
    let captured = CAPTURED
        .get_or_init(|| Arc::new(Mutex::new(Vec::new())))
        .clone();
    INSTALL.call_once(|| {
        let sink = captured.clone();
        let target_filter = tracing_subscriber::filter::filter_fn(|metadata| {
            let target = metadata.target();
            target == "amatl" || target.starts_with("amatl::") || target.starts_with("amatl_")
        });
        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .without_time()
                .with_ansi(false)
                .with_target(true)
                .with_writer(move || LogWriter(sink.clone()))
                .with_filter(target_filter),
        );
        tracing::subscriber::set_global_default(subscriber)
            .expect("test security log subscriber should install once");
    });
    captured
}

#[tokio::test]
async fn rejected_requests_emit_secret_safe_security_events() {
    let _serialized = security_log_guard().await;
    let captured = security_logs();
    captured
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clear();
    let supplied_secret = "never-log-this-invalid-token";
    let response = app()
        .await
        .oneshot(
            request("/search")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/json")
                .header(AUTHORIZATION, format!("Bearer {supplied_secret}"))
                .body(Body::from(r#"{"q":"rust\nforged"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let request_id = response.headers()[REQUEST_ID_HEADER].to_str().unwrap();
    let logs = String::from_utf8(
        captured
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone(),
    )
    .unwrap();
    assert!(logs.contains("amatl::security"), "{logs}");
    assert!(logs.contains("security_event=\"unauthorized\""), "{logs}");
    assert!(logs.contains("path=\"/search\""), "{logs}");
    assert!(logs.contains(request_id), "{logs}");
    assert!(!logs.contains(supplied_secret), "{logs}");
    assert!(!logs.contains("forged"), "{logs}");
}

#[tokio::test]
async fn mcp_ssrf_rejection_is_correlated_without_logging_the_url() {
    let _serialized = security_log_guard().await;
    let captured = security_logs();
    captured
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clear();
    let secret = "never-log-this-ssrf-query-token";
    let call = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "fetch",
            "arguments": {
                "url": format!("http://127.0.0.1/private?token={secret}")
            },
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientInfo": {
                    "name": "security-contract-test",
                    "version": "1"
                },
                "io.modelcontextprotocol/clientCapabilities": {}
            }
        }
    });
    let response = app()
        .await
        .oneshot(
            authorized("/mcp")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/json")
                .header("accept", "application/json, text/event-stream")
                .header("mcp-protocol-version", "2026-07-28")
                .header("mcp-method", "tools/call")
                .header("mcp-name", "fetch")
                .body(Body::from(call.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let request_id = response.headers()[REQUEST_ID_HEADER]
        .to_str()
        .unwrap()
        .to_owned();
    let body = json_body(response).await;
    assert_eq!(body["result"]["isError"], true);

    let logs = String::from_utf8(
        captured
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone(),
    )
    .unwrap();
    assert!(logs.contains("security_event=\"ssrf_blocked\""), "{logs}");
    assert!(logs.contains("stage=\"initial_url\""), "{logs}");
    assert!(logs.contains("reason=\"address_blocked\""), "{logs}");
    assert!(logs.contains(&request_id), "{logs}");
    assert!(!logs.contains(secret), "{logs}");
    assert!(!logs.contains("/private"), "{logs}");
}

#[tokio::test]
async fn mcp_fetch_cannot_bypass_isolated_egress_policy() {
    let mut config = amatl_core::Config::default();
    config.data_policy.profile = amatl_core::SecurityProfile::Isolated;
    config.data_policy.egress = amatl_core::EgressPolicy::Deny;
    config.data_policy.inference = amatl_core::InferenceMode::LocalOnly;
    config.validate().unwrap();
    let isolated_app = build_router(AmatlService::new(config, true).await, Some(TOKEN.into()))
        .await
        .unwrap();
    let call = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "fetch",
            "arguments": {
                "url": "https://example.com/private?token=must-not-leave-host"
            },
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientInfo": {
                    "name": "isolated-policy-test",
                    "version": "1"
                },
                "io.modelcontextprotocol/clientCapabilities": {}
            }
        }
    });
    let response = isolated_app
        .oneshot(
            authorized("/mcp")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/json")
                .header("accept", "application/json, text/event-stream")
                .header("mcp-protocol-version", "2026-07-28")
                .header("mcp-method", "tools/call")
                .header("mcp-name", "fetch")
                .body(Body::from(call.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["result"]["isError"], true);
    assert!(body.to_string().contains("egress_denied"), "{body}");
}

#[tokio::test]
async fn rate_limit_is_keyed_and_body_limit_is_global() {
    let mut config = amatl_core::Config::default();
    config.server.rate_limit_per_minute = 1;
    let rate_limited_app = build_router(AmatlService::new(config, true).await, Some(TOKEN.into()))
        .await
        .unwrap();
    let first = rate_limited_app
        .clone()
        .oneshot(authorized("/search?q=rust").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    let second = rate_limited_app
        .oneshot(authorized("/search?q=rust").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);

    let oversized = app()
        .await
        .oneshot(
            authorized("/mcp")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/json")
                .header("accept", "application/json, text/event-stream")
                .header(CONTENT_LENGTH, (65 * 1024).to_string())
                .body(Body::from(vec![b'x'; 65 * 1024]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);

    let chunked_json = app()
        .await
        .oneshot(
            authorized("/search")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(vec![b'x'; 65 * 1024]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(chunked_json.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn aggregate_header_limit_rejects_before_routing() {
    let mut config = amatl_core::Config::default();
    config.server.max_header_bytes = 128;
    let app = build_router(AmatlService::new(config, true).await, Some(TOKEN.into()))
        .await
        .unwrap();
    let response = app
        .oneshot(
            authorized("/search?q=rust")
                .header("x-padding", "x".repeat(128))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE
    );
    assert!(response.headers().contains_key(REQUEST_ID_HEADER));
    assert_eq!(
        json_body(response).await["error"]["code"],
        "headers_too_large"
    );
}

#[tokio::test]
async fn request_timeout_cancels_a_slow_handler() {
    let mut config = amatl_core::Config::default();
    config.server.request_timeout_ms = 10;
    let service = AmatlService::new(config.clone(), true).await;
    let allowed_origins = effective_origins(&config, false);
    let security = Arc::new(SecurityState {
        clients: resolve_clients(&config, Some(TOKEN.into())).unwrap(),
        allowed_hosts: config.server.allowed_hosts.clone(),
        allowed_origins,
        max_header_bytes: config.server.max_header_bytes,
        max_body_bytes: config.server.max_body_bytes,
        timeout: Duration::from_millis(config.server.request_timeout_ms),
        rate_limit_per_minute: config.server.rate_limit_per_minute,
        https: false,
    });
    let state = AppState {
        service: Arc::new(RwLock::new(service)),
        config_path: None,
        effective_bind: config
            .server
            .bind
            .parse()
            .expect("the default bind is a literal IP"),
        effective_port: config.server.port,
        security: Arc::new(RwLock::new(security)),
        rate_limiter: Arc::new(Mutex::new(RateLimiter {
            windows: BTreeMap::new(),
            last_cleanup: Instant::now(),
        })),
        explicit_token: Some(TOKEN.into()),
        config_write_lock: Arc::new(AsyncMutex::new(())),
        backup_idempotency: Arc::new(BackupIdempotency::default()),
        metrics: Arc::new(RequestMetrics::default()),
    };
    let app = Router::new()
        .route(
            "/slow",
            axum::routing::get(|| async {
                tokio::time::sleep(Duration::from_millis(100)).await;
                StatusCode::OK
            }),
        )
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(state, security_middleware));

    let response = app
        .oneshot(authorized("/slow").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    assert!(response.headers().contains_key(REQUEST_ID_HEADER));
    assert_eq!(
        json_body(response).await["error"]["code"],
        "request_timeout"
    );
}

#[tokio::test]
async fn invalid_credentials_and_public_routes_consume_the_rate_limit() {
    let mut config = amatl_core::Config::default();
    config.server.rate_limit_per_minute = 1;
    let app = build_router(AmatlService::new(config, true).await, Some(TOKEN.into()))
        .await
        .unwrap();

    let invalid = app
        .clone()
        .oneshot(
            request("/search?q=rust")
                .header(AUTHORIZATION, "Bearer definitely-invalid")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::UNAUTHORIZED);
    let rotated = app
        .clone()
        .oneshot(
            request("/search?q=rust")
                .header(AUTHORIZATION, "Bearer another-invalid-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rotated.status(), StatusCode::TOO_MANY_REQUESTS);

    let mut public_config = amatl_core::Config::default();
    public_config.server.rate_limit_per_minute = 1;
    let public_app = build_router(
        AmatlService::new(public_config, true).await,
        Some(TOKEN.into()),
    )
    .await
    .unwrap();
    assert_eq!(
        public_app
            .clone()
            .oneshot(request("/health").body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        public_app
            .oneshot(request("/").body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn real_connect_info_separates_client_rate_windows() {
    let mut config = amatl_core::Config::default();
    config.server.rate_limit_per_minute = 1;
    let app = build_router(AmatlService::new(config, true).await, Some(TOKEN.into()))
        .await
        .unwrap();
    for ip in ["203.0.113.1:4000", "203.0.113.2:4000"] {
        let mut request = authorized("/search?q=rust").body(Body::empty()).unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(ip.parse::<SocketAddr>().unwrap()));
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}

#[tokio::test]
async fn serve_accepts_a_real_tcp_connection() {
    let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reservation.local_addr().unwrap().port();
    drop(reservation);

    let mut config = amatl_core::Config::default();
    config.server.port = port;
    config.server.no_auth = true;
    let server = tokio::spawn(serve(AmatlService::new(config, true).await));
    let mut connection = None;
    for _ in 0..50 {
        match tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
            Ok(stream) => {
                connection = Some(stream);
                break;
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
        }
    }
    let mut stream = connection.expect("server should bind its configured TCP address");
    stream
        .write_all(
            format!("GET /health HTTP/1.1\r\nHost: localhost:{port}\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .await
        .unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    server.abort();
    let response = String::from_utf8(response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(response.contains(r#"{"schema_version":"1","status":"ok"}"#));
}

#[tokio::test]
async fn real_http_parser_rejects_conflicting_message_boundaries() {
    let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reservation.local_addr().unwrap().port();
    drop(reservation);

    let mut config = amatl_core::Config::default();
    config.server.port = port;
    config.server.no_auth = true;
    let server = tokio::spawn(serve(AmatlService::new(config, true).await));
    let mut connection = None;
    for _ in 0..50 {
        match tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
            Ok(stream) => {
                connection = Some(stream);
                break;
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
        }
    }
    let mut stream = connection.expect("server should bind its configured TCP address");
    stream
        .write_all(
            format!(
                "POST /search HTTP/1.1\r\nHost: localhost:{port}\r\nContent-Type: application/json\r\nContent-Length: 4\r\nContent-Length: 0\r\nConnection: close\r\n\r\nnullGET /health HTTP/1.1\r\nHost: localhost:{port}\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(2), stream.read_to_end(&mut response))
        .await
        .expect("malformed request must not hold the connection open")
        .unwrap();
    server.abort();
    let response = String::from_utf8_lossy(&response);
    assert!(
        response.starts_with("HTTP/1.1 400 Bad Request"),
        "{response}"
    );
    assert_eq!(response.matches("HTTP/1.1").count(), 1, "{response}");
    assert!(!response.contains(r#"{"schema_version":"1","status":"ok"}"#));
}

#[tokio::test]
async fn serve_completes_a_real_rustls_handshake() {
    let rcgen::CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("amatl-tls-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let cert_path = directory.join("cert.pem");
    let key_path = directory.join("key.pem");
    let cert_pem = cert.pem();
    std::fs::write(&cert_path, &cert_pem).unwrap();
    std::fs::write(&key_path, signing_key.serialize_pem()).unwrap();

    let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reservation.local_addr().unwrap().port();
    drop(reservation);
    let mut config = amatl_core::Config::default();
    config.server.port = port;
    config.server.no_auth = true;
    config.server.tls.cert_path = Some(cert_path.to_string_lossy().into_owned());
    config.server.tls.key_path = Some(key_path.to_string_lossy().into_owned());
    let server = tokio::spawn(serve(AmatlService::new(config, true).await));
    let root = reqwest::Certificate::from_pem(cert_pem.as_bytes()).unwrap();
    let client = reqwest::Client::builder()
        .add_root_certificate(root)
        .build()
        .unwrap();
    let mut response = None;
    for _ in 0..50 {
        match client
            .get(format!("https://localhost:{port}/health"))
            .send()
            .await
        {
            Ok(value) => {
                response = Some(value);
                break;
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
        }
    }
    let response = response.expect("TLS server should accept a trusted localhost certificate");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response.json::<serde_json::Value>().await.unwrap(),
        json!({"schema_version": "1", "status": "ok"})
    );
    let untrusted = reqwest::Client::new()
        .get(format!("https://localhost:{port}/health"))
        .send()
        .await
        .expect_err("an untrusted self-signed certificate must be rejected");
    assert!(untrusted.is_connect(), "{untrusted}");
    server.abort();
    let _ = server.await;
    std::fs::remove_file(cert_path).unwrap();
    std::fs::remove_file(key_path).unwrap();
    std::fs::remove_dir(directory).unwrap();
}

#[tokio::test]
async fn mcp_uses_streamable_http_and_exposes_exactly_the_declared_tools() {
    let initialize = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2026-07-28",
            "capabilities": {},
            "clientInfo": { "name": "contract-test", "version": "1" }
        }
    });
    let response = app()
        .await
        .oneshot(
            authorized("/mcp")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/json")
                .header("accept", "application/json, text/event-stream")
                .body(Body::from(initialize.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let initialized = json_body(response).await;
    assert_eq!(status, StatusCode::OK, "initialize response: {initialized}");
    assert_eq!(initialized["result"]["serverInfo"]["name"], "amatl");
    assert_eq!(
        initialized["result"]["serverInfo"]["version"],
        env!("CARGO_PKG_VERSION")
    );
    // Since SEP-2567 the newest revision (`2026-07-28`) carries its lifecycle in
    // per-request `_meta` and has no `initialize` handshake, so an `initialize`
    // request is answered with the newest revision that still has one
    // (`2025-11-25`). The stateless `2026-07-28` path is exercised separately by
    // the `tools/list` call below, which names `2026-07-28` in its `_meta`.
    assert_eq!(initialized["result"]["protocolVersion"], "2025-11-25");

    let list = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": {
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientInfo": {
                    "name": "contract-test",
                    "version": "1"
                },
                "io.modelcontextprotocol/clientCapabilities": {}
            }
        }
    });
    let response = app()
        .await
        .oneshot(
            authorized("/mcp")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/json")
                .header("accept", "application/json, text/event-stream")
                .header("mcp-protocol-version", "2026-07-28")
                .header("mcp-method", "tools/list")
                .body(Body::from(list.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let listed = json_body(response).await;
    assert_eq!(status, StatusCode::OK, "tools/list response: {listed}");
    let names = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        names,
        [
            "deep_search",
            "fetch",
            "providers",
            "search",
            "status"
        ]
        .into()
    );
    // Local file ingestion is deliberately CLI-only: an MCP listener must not
    // become a remote file reader.
    assert!(!names.contains("ingest"));

    let call = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "search",
            "arguments": { "query": "rust" },
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientInfo": {
                    "name": "contract-test",
                    "version": "1"
                },
                "io.modelcontextprotocol/clientCapabilities": {}
            }
        }
    });
    let response = app()
        .await
        .oneshot(
            authorized("/mcp")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/json")
                .header("accept", "application/json, text/event-stream")
                .header("mcp-protocol-version", "2026-07-28")
                .header("mcp-method", "tools/call")
                .header("mcp-name", "search")
                .body(Body::from(call.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let called = json_body(response).await;
    assert_eq!(status, StatusCode::OK, "tools/call response: {called}");
    assert_eq!(called["result"]["structuredContent"]["schema_version"], "1");
    assert_eq!(
        called["result"]["structuredContent"]["results"][0]["canonical_url"],
        "https://example.com/rust"
    );
}

#[tokio::test]
async fn domain_failures_keep_their_catalog_code_and_status() {
    for (error, code, status) in [
        (
            ServiceError::MissingPlan,
            "search_planning_failed",
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
        (
            ServiceError::ProviderNotRegistered("custom_archive".into()),
            "provider_not_registered",
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
        (
            ServiceError::InferenceUnavailable,
            "inference_unavailable",
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
        (
            ServiceError::InvalidQuery,
            "invalid_query",
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let response = service_error(error);
        assert_eq!(response.status(), status);
        let body = json_body(response).await;
        assert_eq!(body["error"]["code"], code);
        assert_ne!(body["error"]["message"], code);
        assert_eq!(body["schema_version"], "1");
    }
}

#[tokio::test]
async fn transport_errors_render_catalog_messages_instead_of_repeating_the_code() {
    let response = api_error(ErrorCode::RateLimited);
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let body = json_body(response).await;
    assert_eq!(body["error"]["code"], "rate_limited");
    assert_eq!(body["error"]["message"], ErrorCode::RateLimited.message());
}

#[tokio::test]
async fn metrics_endpoint_returns_prometheus_exposition_format() {
    let app = app().await;

    // The /metrics endpoint is public (no auth required).
    let response = app
        .oneshot(request("/metrics").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "text/plain; version=0.0.4; charset=utf-8"
    );
    assert_eq!(
        response
            .headers()
            .get(CACHE_CONTROL)
            .unwrap()
            .to_str()
            .unwrap(),
        "no-store"
    );

    let body = String::from_utf8(
        to_bytes(response.into_body(), 128 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();

    // Every Prometheus metric line must have HELP and TYPE.
    let expected_metrics = [
        "amatl_search_requests_total",
        "amatl_deep_requests_total",
        "amatl_search_errors_total",
        "amatl_deep_errors_total",
        "amatl_rate_limited_total",
        "amatl_unauthorized_total",
        "amatl_request_timeout_total",
    ];
    for name in expected_metrics {
        assert!(
            body.contains(&format!("# HELP {name} ")),
            "missing HELP for {name}"
        );
        assert!(
            body.contains(&format!("# TYPE {name} counter")),
            "missing TYPE for {name}"
        );
        // Each counter must appear with a numeric value.
        assert!(
            body.lines().any(|line| line.starts_with(name) && {
                let parts: Vec<&str> = line.split_whitespace().collect();
                parts.len() == 2 && parts[1].parse::<u64>().is_ok()
            }),
            "missing or malformed metric line for {name}"
        );
    }

    // No trailing whitespace on metric lines.
    for line in body.lines() {
        if !line.starts_with('#') && !line.is_empty() {
            assert!(
                !line.ends_with(' '),
                "trailing space in metric line: {line:?}"
            );
        }
    }

    // Body must end with a newline (Prometheus convention).
    assert!(body.ends_with('\n'), "metrics body must end with newline");
}

#[tokio::test]
async fn metrics_counters_increment_on_search_and_deep() {
    let app = app().await;

    // Read initial counters.
    let initial = app
        .clone()
        .oneshot(request("/metrics").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let initial_body = String::from_utf8(
        to_bytes(initial.into_body(), 128 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    let parse_counter = |body: &str, name: &str| -> u64 {
        body.lines()
            .find(|line| line.starts_with(name))
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    };

    let initial_search = parse_counter(&initial_body, "amatl_search_requests_total");

    // Execute a search (it will fail because there are no real providers,
    // but the counter should still increment on the Ok path or error path).
    let _ = app
        .clone()
        .oneshot(
            authorized("/search?q=test+metrics+increment")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let after = app
        .oneshot(request("/metrics").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let after_body = String::from_utf8(
        to_bytes(after.into_body(), 128 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    let after_search = parse_counter(&after_body, "amatl_search_requests_total");

    // At least one of search_total or search_errors must have incremented.
    let after_search_errors = parse_counter(&after_body, "amatl_search_errors_total");
    let initial_search_errors = parse_counter(&initial_body, "amatl_search_errors_total");
    assert!(
        after_search > initial_search || after_search_errors > initial_search_errors,
        "neither search_total nor search_errors incremented after a search request"
    );
}

#[tokio::test]
async fn request_id_is_generated_and_echoed_on_every_response() {
    let app = app().await;

    // The server always generates its own request_id; client-supplied
    // X-Request-Id headers are ignored to prevent spoofing.
    let response = app
        .clone()
        .oneshot(
            authorized("/search?q=request+id+propagation")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let echoed_id = response
        .headers()
        .get(REQUEST_ID_HEADER)
        .expect("X-Request-Id header missing")
        .to_str()
        .unwrap();

    // Server-generated IDs follow the pattern: {epoch_nanos:032x}-{sequence:016x}
    assert!(echoed_id.len() >= 49, "request_id too short: {echoed_id}");
    let parts: Vec<&str> = echoed_id.split('-').collect();
    assert_eq!(parts.len(), 2, "request_id must have exactly one dash");
    assert_eq!(parts[0].len(), 32, "epoch part must be 32 hex chars");
    assert_eq!(parts[1].len(), 16, "sequence part must be 16 hex chars");
    assert!(
        parts[0].chars().all(|c| c.is_ascii_hexdigit()),
        "epoch part not hex: {}",
        parts[0]
    );
    assert!(
        parts[1].chars().all(|c| c.is_ascii_hexdigit()),
        "sequence part not hex: {}",
        parts[1]
    );

    // Two consecutive requests must produce different IDs.
    let response2 = app
        .clone()
        .oneshot(
            authorized("/search?q=second+request")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let id2 = response2
        .headers()
        .get(REQUEST_ID_HEADER)
        .unwrap()
        .to_str()
        .unwrap();
    assert_ne!(echoed_id, id2, "consecutive request_ids must be unique");
}

#[tokio::test]
async fn request_id_header_is_present_on_all_api_responses() {
    let app = app().await;

    for (method, path) in [
        ("GET", "/health"),
        ("GET", "/providers"),
        ("GET", "/metrics"),
        ("GET", "/search?q=test"),
    ] {
        let builder = if path == "/search?q=test" {
            authorized(path)
        } else {
            request(path)
        };
        let response = app
            .clone()
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert!(
            response.headers().contains_key(REQUEST_ID_HEADER),
            "{method} {path} missing X-Request-Id header"
        );
    }
}

/// Deep is a network-retrieval surface and its contract is POST-only: the
/// authenticated `GET /deep` that once coexisted with `POST /deep` is retired.
/// This pins the invariant so a future router edit cannot silently republish a
/// GET recovery path that no test, doc or OpenAPI entry describes.
#[tokio::test]
async fn deep_is_post_only_and_get_is_method_not_allowed() {
    let app = app().await;

    let get = app
        .clone()
        .oneshot(authorized("/deep?q=test").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(
        get.status(),
        StatusCode::METHOD_NOT_ALLOWED,
        "GET /deep must no longer be routed"
    );

    // The POST surface stays available: the endpoint itself did not disappear.
    let post = app
        .oneshot(
            authorized("/deep")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "query": "test" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(
        post.status(),
        StatusCode::METHOD_NOT_ALLOWED,
        "POST /deep must remain routed"
    );
}

// ── MCP protocol conformance tests ──────────────────────────────────────────

/// Helper: build an MCP JSON-RPC request body.
fn mcp_request(method: &str, params: serde_json::Value) -> serde_json::Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params
    })
}

/// Helper: send an MCP POST and return (status, body).
/// Includes required MCP Streamable HTTP headers for non-initialize requests.
async fn mcp_post(app: &Router, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
    mcp_post_with_headers(app, body, None, None).await
}

/// Helper: send an MCP POST with optional MCP method/name headers.
/// Returns (status, body). Body is Value::Null when the response is not valid JSON.
async fn mcp_post_with_headers(
    app: &Router,
    body: serde_json::Value,
    mcp_method: Option<&str>,
    mcp_name: Option<&str>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = authorized("/mcp")
        .method(Method::POST)
        .header(CONTENT_TYPE, "application/json")
        .header("accept", "application/json, text/event-stream");
    if let Some(method) = mcp_method {
        builder = builder.header("mcp-protocol-version", "2026-07-28");
        builder = builder.header("mcp-method", method);
    }
    if let Some(name) = mcp_name {
        builder = builder.header("mcp-name", name);
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = json_body_opt(response)
        .await
        .unwrap_or(serde_json::Value::Null);
    (status, body)
}

#[tokio::test]
async fn mcp_rejects_non_json_body() {
    let app = app().await;
    let response = app
        .oneshot(
            authorized("/mcp")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/json")
                .header("accept", "application/json, text/event-stream")
                .body(Body::from("not json at all"))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    // Server may reject at HTTP level (400, 415) or return JSON-RPC error (200).
    assert!(
        status == StatusCode::BAD_REQUEST
            || status == StatusCode::OK
            || status == StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "unexpected status: {status}"
    );
    if status == StatusCode::OK {
        let body = json_body(response).await;
        assert!(body["error"]["code"].is_number() || body["error"].is_object());
    }
}

#[tokio::test]
async fn mcp_rejects_missing_jsonrpc_version() {
    let app = app().await;
    let call = json!({
        "id": 1,
        "method": "tools/list",
        "params": {}
    });
    let (status, body) = mcp_post_with_headers(&app, call, Some("tools/list"), None).await;
    // rmcp may reject at HTTP level (400, 415) or return JSON-RPC error (200).
    assert!(
        status == StatusCode::OK
            || status == StatusCode::BAD_REQUEST
            || status == StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "unexpected status: {status}"
    );
    if status == StatusCode::OK {
        assert!(body["error"].is_object(), "expected error, got: {body}");
    }
}

#[tokio::test]
async fn mcp_rejects_invalid_jsonrpc_version() {
    let app = app().await;
    let call = json!({
        "jsonrpc": "1.0",
        "id": 1,
        "method": "tools/list",
        "params": {}
    });
    let (status, body) = mcp_post_with_headers(&app, call, Some("tools/list"), None).await;
    assert!(
        status == StatusCode::OK
            || status == StatusCode::BAD_REQUEST
            || status == StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "unexpected status: {status}"
    );
    if status == StatusCode::OK {
        assert!(body["error"].is_object(), "expected error, got: {body}");
    }
}

#[tokio::test]
async fn mcp_rejects_unsupported_method() {
    let app = app().await;
    let call = mcp_request("resources/list", json!({}));
    let (status, body) = mcp_post_with_headers(&app, call, Some("resources/list"), None).await;
    // rmcp rejects unsupported methods at HTTP level (400).
    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::OK,
        "unexpected status: {status}"
    );
    if status == StatusCode::OK {
        assert!(
            body["error"].is_object(),
            "expected error for unsupported method, got: {body}"
        );
    }
}

#[tokio::test]
async fn mcp_rejects_missing_method_field() {
    let app = app().await;
    let call = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "params": {}
    });
    let (status, body) = mcp_post_with_headers(&app, call, Some("tools/list"), None).await;
    assert!(
        status == StatusCode::OK
            || status == StatusCode::BAD_REQUEST
            || status == StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "unexpected status: {status}"
    );
    if status == StatusCode::OK {
        assert!(body["error"].is_object(), "expected error, got: {body}");
    }
}

#[tokio::test]
async fn mcp_rejects_notification_without_id() {
    let app = app().await;
    let call = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
        "params": {}
    });
    let (status, _body) =
        mcp_post_with_headers(&app, call, Some("notifications/initialized"), None).await;
    // rmcp may reject unsupported notification methods at HTTP level.
    assert!(
        status == StatusCode::OK
            || status == StatusCode::ACCEPTED
            || status == StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn mcp_tools_call_rejects_unknown_tool() {
    let app = app().await;
    let call = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "nonexistent_tool",
            "arguments": {},
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientInfo": {
                    "name": "conformance-test",
                    "version": "1"
                },
                "io.modelcontextprotocol/clientCapabilities": {}
            }
        }
    });
    let (status, body) =
        mcp_post_with_headers(&app, call, Some("tools/call"), Some("nonexistent_tool")).await;
    assert!(
        status == StatusCode::OK || status == StatusCode::BAD_REQUEST,
        "unexpected status: {status}"
    );
    if status == StatusCode::OK {
        assert!(
            body["error"].is_object() || body["result"]["isError"] == true,
            "expected error for unknown tool, got: {body}"
        );
    }
}

#[tokio::test]
async fn mcp_tools_call_rejects_missing_arguments() {
    let app = app().await;
    let call = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "search",
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientInfo": {
                    "name": "conformance-test",
                    "version": "1"
                },
                "io.modelcontextprotocol/clientCapabilities": {}
            }
        }
    });
    let (status, body) =
        mcp_post_with_headers(&app, call, Some("tools/call"), Some("search")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body["error"].is_object() || body["result"]["isError"] == true,
        "expected error for missing arguments, got: {body}"
    );
}

#[tokio::test]
async fn mcp_search_rejects_empty_query() {
    let app = app().await;
    let call = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "search",
            "arguments": { "query": "" },
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientInfo": {
                    "name": "conformance-test",
                    "version": "1"
                },
                "io.modelcontextprotocol/clientCapabilities": {}
            }
        }
    });
    let (status, body) =
        mcp_post_with_headers(&app, call, Some("tools/call"), Some("search")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"]["isError"], true);
    assert!(body.to_string().contains("invalid_query"), "{body}");
}

#[tokio::test]
async fn mcp_search_rejects_oversized_query() {
    let app = app().await;
    let oversized = "x".repeat(2049);
    let call = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "search",
            "arguments": { "query": oversized },
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientInfo": {
                    "name": "conformance-test",
                    "version": "1"
                },
                "io.modelcontextprotocol/clientCapabilities": {}
            }
        }
    });
    let (status, body) =
        mcp_post_with_headers(&app, call, Some("tools/call"), Some("search")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"]["isError"], true);
    assert!(body.to_string().contains("invalid_query"), "{body}");
}

#[tokio::test]
async fn mcp_fetch_rejects_invalid_url() {
    let app = app().await;
    let call = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "fetch",
            "arguments": { "url": "not-a-valid-url" },
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientInfo": {
                    "name": "conformance-test",
                    "version": "1"
                },
                "io.modelcontextprotocol/clientCapabilities": {}
            }
        }
    });
    let (status, body) = mcp_post_with_headers(&app, call, Some("tools/call"), Some("fetch")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"]["isError"], true);
    assert!(body.to_string().contains("invalid_url"), "{body}");
}

#[tokio::test]
async fn mcp_requires_authentication() {
    let app = app().await;
    let call = mcp_request("tools/list", json!({}));
    let response = app
        .oneshot(
            request("/mcp")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/json")
                .header("accept", "application/json, text/event-stream")
                .body(Body::from(call.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn mcp_rate_limit_applies_to_mcp_endpoint() {
    let mut config = amatl_core::Config::default();
    config.server.rate_limit_per_minute = 2; // allow initialize + 1 tools/list
    let app = build_router(AmatlService::new(config, true).await, Some(TOKEN.into()))
        .await
        .unwrap();

    // Initialize MCP session first (required by Streamable HTTP protocol).
    let init = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2026-07-28",
            "capabilities": {},
            "clientInfo": { "name": "rate-limit-test", "version": "1" }
        }
    });
    let (init_status, _) = mcp_post(&app, init).await;
    assert_eq!(init_status, StatusCode::OK);

    let call = mcp_request("tools/list", json!({}));
    let (status1, _) = mcp_post_with_headers(&app, call.clone(), Some("tools/list"), None).await;
    // MCP Streamable HTTP sessions are per-connection; a new connection may
    // not have the session from initialize. Accept 200 or 400.
    assert!(
        status1 == StatusCode::OK || status1 == StatusCode::BAD_REQUEST,
        "unexpected status1: {status1}"
    );

    let (status2, _) = mcp_post_with_headers(&app, call, Some("tools/list"), None).await;
    // Rate limit may return 429, or the MCP layer may return 400.
    assert!(
        status2 == StatusCode::TOO_MANY_REQUESTS || status2 == StatusCode::BAD_REQUEST,
        "expected rate-limit rejection, got: {status2}"
    );
}

#[tokio::test]
async fn mcp_initialize_returns_correct_server_info() {
    let app = app().await;
    let call = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2026-07-28",
            "capabilities": {},
            "clientInfo": { "name": "conformance-test", "version": "1" }
        }
    });
    let (status, body) = mcp_post(&app, call).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"]["serverInfo"]["name"], "amatl");
    assert_eq!(
        body["result"]["serverInfo"]["version"],
        env!("CARGO_PKG_VERSION")
    );
    // An `initialize` handshake is answered with the newest revision that still
    // has one; `2026-07-28` moved the lifecycle into per-request `_meta`
    // (SEP-2567), so it can no longer be named by `initialize`.
    assert_eq!(body["result"]["protocolVersion"], "2025-11-25");
    assert!(body["result"]["capabilities"]["tools"].is_object());
}

#[tokio::test]
async fn mcp_initialize_never_echoes_a_version_newer_than_supported() {
    let app = app().await;
    let call = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            // A revision newer than every version this server advertises: the
            // negotiated answer must be one of the server's own versions, never
            // the unsupported request echoed back.
            "protocolVersion": "2099-01-01",
            "capabilities": {},
            "clientInfo": { "name": "conformance-test", "version": "1" }
        }
    });
    let (status, body) = mcp_post(&app, call).await;
    assert_eq!(status, StatusCode::OK);
    let negotiated = body["result"]["protocolVersion"].as_str().unwrap_or("");
    assert_ne!(
        negotiated, "2099-01-01",
        "server must not echo an unsupported protocol version, got: {body}"
    );
    assert!(
        !negotiated.is_empty(),
        "expected a negotiated protocol version"
    );
}

#[tokio::test]
async fn mcp_providers_tool_returns_valid_structure() {
    let app = app().await;
    let call = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "providers",
            "arguments": {},
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientInfo": {
                    "name": "conformance-test",
                    "version": "1"
                },
                "io.modelcontextprotocol/clientCapabilities": {}
            }
        }
    });
    let (status, body) =
        mcp_post_with_headers(&app, call, Some("tools/call"), Some("providers")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"]["structuredContent"]["schema_version"], "1");
    assert!(body["result"]["structuredContent"]["providers"].is_array());
}

#[tokio::test]
async fn mcp_deep_search_rejects_empty_query() {
    let app = app().await;
    let call = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "deep_search",
            "arguments": { "query": "" },
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientInfo": {
                    "name": "conformance-test",
                    "version": "1"
                },
                "io.modelcontextprotocol/clientCapabilities": {}
            }
        }
    });
    let (status, body) =
        mcp_post_with_headers(&app, call, Some("tools/call"), Some("deep_search")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"]["isError"], true);
    assert!(body.to_string().contains("invalid_query"), "{body}");
}

#[tokio::test]
async fn mcp_response_includes_request_id_header() {
    let app = app().await;
    let call = mcp_request("tools/list", json!({}));
    let response = app
        .clone()
        .oneshot(
            authorized("/mcp")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/json")
                .header("accept", "application/json, text/event-stream")
                .header("mcp-protocol-version", "2026-07-28")
                .header("mcp-method", "tools/list")
                .body(Body::from(call.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        response.status() == StatusCode::OK || response.status() == StatusCode::BAD_REQUEST,
        "unexpected status: {}",
        response.status()
    );
    assert!(
        response.headers().contains_key(REQUEST_ID_HEADER),
        "MCP response missing X-Request-Id header"
    );
}

// ── Local domain surfaces and enriched metrics ──────────────────────

/// Router backed by a throwaway SQLite database so the history and saved
/// document surfaces have real persistence.
async fn persistent_app() -> (Router, std::path::PathBuf) {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "amatl-server-domain-{}-{nonce}.sqlite3",
        std::process::id()
    ));
    let mut config = amatl_core::Config::default();
    config.persistence.enabled = true;
    config.persistence.path = path.display().to_string();
    config.validate().unwrap();
    let router = build_router(AmatlService::new(config, true).await, Some(TOKEN.into()))
        .await
        .unwrap();
    (router, path)
}

#[tokio::test]
async fn domain_surfaces_require_the_bearer_token() {
    let app = app().await;
    for path in ["/status", "/history", "/saved"] {
        let response = app
            .clone()
            .oneshot(request(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{path} must be protected"
        );
    }
    let response = app
        .clone()
        .oneshot(
            request("/history/1")
                .method(Method::DELETE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn status_reports_sources_storage_and_cache() {
    let response = app()
        .await
        .oneshot(authorized("/status").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["schema_version"], "1");
    assert!(body["status"] == "ok" || body["status"] == "degraded");
    assert!(body["sources"].is_array());
    // Persistence is disabled by default, so storage reports it plainly.
    assert_eq!(body["storage"]["enabled"], false);
    assert_eq!(body["storage"]["available"], false);
    assert!(body["cache"]["provider_search_hit_rate"].is_number());
    // Default data policy, read-only here — see the `/data-policy` tests for
    // the write path.
    assert_eq!(body["data_policy"]["profile"], "standard");
    assert_eq!(body["data_policy"]["egress"], "governed");
    assert_eq!(body["data_policy"]["inference"], "disabled");
}

/// Regression for AUDIT H-01: the OpenAPI `ServiceStatus` schema declared
/// `additionalProperties: false` but omitted `data_policy`, which the server
/// emits. A client validating `/status` against the published
/// contract therefore rejected a response the server considered valid. Nothing
/// asserted the response shape before, so the schema silently drifted.
///
/// This test parses the live `/status` body and recursively validates it
/// against the schema: every emitted key (at any depth) must be declared and
/// every required key must be present. It needs no new dependency.
#[tokio::test]
async fn status_response_matches_the_openapi_service_status_schema() {
    let response = app()
        .await
        .oneshot(authorized("/status").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["authentication_required"], true);

    let openapi = include_str!("../openapi.yaml").replace('\r', "");
    let schemas = parse_schemas(&openapi);
    let node = schemas
        .get("ServiceStatus")
        .expect("OpenAPI declares SchemaNode ServiceStatus");
    let mut errors = Vec::new();
    assert_conforms(&body, node, &schemas, "$", &mut errors);
    assert!(
        errors.is_empty(),
        "/status does not conform to the OpenAPI ServiceStatus schema: {errors:?}"
    );
}

/// A node of the subset of JSON Schema this guard understands: the `required`
/// list, a `properties` map, an `items` schema for arrays, a `$ref` to another
/// component schema, and whether the object accepts unknown keys
/// (`additionalProperties: true`). Enough to check the OpenAPI document this
/// repository ships, without pulling a YAML or JSON-Schema dependency into the
/// test build.
#[derive(Default, Clone)]
struct SchemaNode {
    required: std::collections::BTreeSet<String>,
    properties: std::collections::BTreeMap<String, SchemaNode>,
    items: Option<Box<SchemaNode>>,
    reference: Option<String>,
    open: bool,
    /// Allowed scalar values when the schema declares an `enum`. Empty means
    /// "no enum constraint".
    enum_values: Vec<String>,
}

/// Parses the `components.schemas` section into `SchemaNode`s keyed by name.
fn parse_schemas(openapi: &str) -> std::collections::BTreeMap<String, SchemaNode> {
    let start = openapi
        .find("\n  schemas:")
        .expect("OpenAPI has a schemas section");
    // The section ends at the next key indented by exactly two spaces that is
    // not part of `schemas:` (none here), so take the rest of the document.
    let section = &openapi[start..];
    let lines: Vec<&str> = section.lines().collect();
    let mut schemas = std::collections::BTreeMap::new();
    let mut i = 0;
    // Skip the `  schemas:` header.
    while i < lines.len() && !lines[i].trim_end().ends_with("schemas:") {
        i += 1;
    }
    i += 1;
    while i < lines.len() {
        let line = lines[i];
        // A schema name sits at four-space indentation and ends with ':'.
        let Some(name) = line
            .strip_prefix("    ")
            .and_then(|rest| rest.strip_suffix(':'))
        else {
            i += 1;
            continue;
        };
        if name.starts_with(' ') || name.is_empty() {
            i += 1;
            continue;
        }
        // Collect the block: following lines indented at least six spaces.
        let block_start = i + 1;
        let mut block_end = block_start;
        while block_end < lines.len() {
            let l = lines[block_end];
            if l.trim().is_empty() {
                block_end += 1;
                continue;
            }
            let indent = l.len() - l.trim_start().len();
            if indent <= 4 {
                break;
            }
            block_end += 1;
        }
        let block = lines[block_start..block_end].join("\n");
        schemas.insert(name.to_owned(), parse_schema_block(&block, 6));
        i = block_end;
    }
    schemas
}

/// Parses one schema block (its lines indented at `base` spaces) into a
/// `SchemaNode`. Recurses into `properties.<name>` and `items`.
fn parse_schema_block(block: &str, base: usize) -> SchemaNode {
    let mut node = SchemaNode::default();
    let lines: Vec<&str> = block.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim_start();
        if indent == base {
            if let Some((_, tail)) = trimmed.split_once('[') {
                if trimmed.starts_with("required:") {
                    if let Some((list, _)) = tail.split_once(']') {
                        node.required = list
                            .split(',')
                            .map(|n| n.trim().to_owned())
                            .filter(|n| !n.is_empty())
                            .collect();
                    }
                } else if trimmed.starts_with("enum:") {
                    if let Some((list, _)) = tail.split_once(']') {
                        node.enum_values = list
                            .split(',')
                            .map(|v| v.trim().trim_matches(['\'', '"']).to_owned())
                            .filter(|v| !v.is_empty())
                            .collect();
                    }
                }
            }
            if trimmed.starts_with("additionalProperties:") {
                node.open = trimmed.contains("true");
            }
            if trimmed.starts_with("$ref:") {
                node.reference = trimmed
                    .split_once("#/components/schemas/")
                    .map(|(_, n)| {
                        let n = n.trim_start().trim_start_matches(['\'', '"']);
                        n.split(['\'', '"', ',', '}', ' '])
                            .next()
                            .unwrap_or("")
                            .to_owned()
                    })
                    .filter(|n| !n.is_empty());
            }
            if trimmed.starts_with("properties:") || trimmed.starts_with("items:") {
                let is_properties = trimmed.starts_with("properties:");
                // Inline form: `items: { $ref: ... }` / `properties: {}`.
                if let Some((_, rest)) = trimmed.split_once('{') {
                    let body = rest.split('}').next().unwrap_or("");
                    let inline = parse_inline_map(body);
                    if is_properties {
                        // Inline `properties` maps are not used in this document;
                        // ignore rather than guess.
                    } else {
                        node.items = Some(Box::new(inline));
                    }
                } else {
                    // A nested block follows; collect it.
                    let child_base = indent + 2;
                    let child_start = i + 1;
                    let mut child_end = child_start;
                    while child_end < lines.len() {
                        let l = lines[child_end];
                        if l.trim().is_empty() {
                            child_end += 1;
                            continue;
                        }
                        let ci = l.len() - l.trim_start().len();
                        if ci <= indent {
                            break;
                        }
                        child_end += 1;
                    }
                    let child = lines[child_start..child_end].join("\n");
                    if is_properties {
                        for (pname, pnode) in parse_properties(&child, child_base) {
                            node.properties.insert(pname, pnode);
                        }
                    } else {
                        node.items = Some(Box::new(parse_schema_block(&child, child_base)));
                    }
                    i = child_end;
                    continue;
                }
            }
        }
        i += 1;
    }
    node
}

/// Parses a `properties:` block (its entries indented at `base` spaces) into a
/// name → `SchemaNode` map, recursing into each property's own block.
fn parse_properties(block: &str, base: usize) -> Vec<(String, SchemaNode)> {
    let lines: Vec<&str> = block.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim_start();
        if indent == base {
            // `name:` with a nested block on following lines.
            if let Some(name) = trimmed.strip_suffix(':') {
                if !name.starts_with(' ') && !name.contains(' ') && !name.is_empty() {
                    let child_start = i + 1;
                    let mut child_end = child_start;
                    while child_end < lines.len() {
                        let l = lines[child_end];
                        if l.trim().is_empty() {
                            child_end += 1;
                            continue;
                        }
                        let ci = l.len() - l.trim_start().len();
                        if ci <= indent {
                            break;
                        }
                        child_end += 1;
                    }
                    let child = lines[child_start..child_end].join("\n");
                    out.push((name.to_owned(), parse_schema_block(&child, indent + 2)));
                    i = child_end;
                    continue;
                }
            } else if let Some((name, tail)) = trimmed.split_once(':') {
                // `name: { ... }` inline map, or `name: <scalar>`.
                let name = name.trim();
                if !name.is_empty() && !name.contains(' ') && !name.contains('-') {
                    let node = if let Some((_, rest)) = tail.split_once('{') {
                        let body = rest.split('}').next().unwrap_or("");
                        parse_inline_map(body)
                    } else {
                        SchemaNode::default()
                    };
                    out.push((name.to_owned(), node));
                }
            }
        }
        i += 1;
    }
    out
}

/// Parses the inside of a one-line `{ ... }` schema map, extracting the pieces
/// this guard needs: `$ref`, `enum`, `additionalProperties: true`, and
/// `items: { ... }`.
fn parse_inline_map(body: &str) -> SchemaNode {
    let mut node = SchemaNode::default();
    if body.contains("additionalProperties: true") {
        node.open = true;
    }
    if let Some((_, tail)) = body.split_once("enum:") {
        if let Some((list, _)) = tail.split_once('[').and_then(|(_, t)| t.split_once(']')) {
            node.enum_values = list
                .split(',')
                .map(|v| v.trim().trim_matches(['\'', '"']).to_owned())
                .filter(|v| !v.is_empty())
                .collect();
        }
    }
    if let Some((_, tail)) = body.split_once("$ref:") {
        let name = tail
            .trim_start()
            .trim_start_matches(['\'', '"'])
            .split(['\'', '"', ',', '}', ' '])
            .next()
            .unwrap_or("");
        if let Some(name) = name.rsplit('/').next() {
            if !name.is_empty() {
                node.reference = Some(name.to_owned());
            }
        }
    }
    if let Some((_, tail)) = body.split_once("items:") {
        if let Some((_, rest)) = tail.split_once('{') {
            let inner = rest.split('}').next().unwrap_or("");
            node.items = Some(Box::new(parse_inline_map(inner)));
        }
    }
    node
}

/// Recursively verifies that `value` conforms to `node`: every emitted key is
/// declared (unless the schema is open), every required key is emitted, and
/// nested objects/arrays recurse. `$ref` is resolved against `schemas`.
fn assert_conforms(
    value: &serde_json::Value,
    node: &SchemaNode,
    schemas: &std::collections::BTreeMap<String, SchemaNode>,
    path: &str,
    errors: &mut Vec<String>,
) {
    // Resolve a $ref, guarding against cycles.
    if let Some(name) = &node.reference {
        if let Some(target) = schemas.get(name) {
            assert_conforms(value, target, schemas, path, errors);
        }
        return;
    }
    match value {
        serde_json::Value::Object(map) => {
            if !node.open {
                for key in map.keys() {
                    if !node.properties.contains_key(key) && !node.required.contains(key) {
                        errors.push(format!("{path}.{key}: emitted but not declared"));
                    }
                }
            }
            for key in &node.required {
                if !map.contains_key(key) {
                    errors.push(format!("{path}.{key}: required by schema but not emitted"));
                }
            }
            for (key, child) in &node.properties {
                if let Some(v) = map.get(key) {
                    assert_conforms(v, child, schemas, &format!("{path}.{key}"), errors);
                }
            }
        }
        serde_json::Value::Array(items) => {
            if let (Some(schema), Some(first)) = (node.items.as_deref(), items.first()) {
                assert_conforms(first, schema, schemas, &format!("{path}[]"), errors);
            }
        }
        serde_json::Value::String(text)
            if !node.enum_values.is_empty() && !node.enum_values.iter().any(|v| v == text) =>
        {
            errors.push(format!(
                "{path}: value {text:?} not in enum {:?}",
                node.enum_values
            ));
        }
        _ => {}
    }
}

/// Regression for AUDIT H-03: the admin request/response bodies were typed as
/// an intentionally open generic object (`AdminPayload`), so the published
/// contract did not pin the fields the server actually reads or returns. They
/// now carry concrete schemas; this test recursively validates each admin GET
/// response against its schema (every emitted key declared, every required key
/// present, at any depth), so the contract cannot silently drift from the
/// handler again — including nested drift such as `server.tls`.
#[tokio::test]
async fn admin_responses_match_their_openapi_schemas() {
    let openapi = include_str!("../openapi.yaml").replace('\r', "");
    let schemas = parse_schemas(&openapi);
    // (path, schema name) for the admin read surfaces that are always 200 on a
    // default service (no persistence needed).
    let cases: &[(&str, &str)] = &[
        ("/providers/marginalia", "ProviderRecordReport"),
        ("/policies", "PoliciesReport"),
        ("/persistence", "PersistenceConfigReport"),
        ("/telemetry", "TelemetryConfigReport"),
        ("/deep/limits", "DeepConfigReport"),
        ("/circuits", "CircuitList"),
        ("/server/pending-config", "ServerConfigReport"),
        ("/server/clients", "ServerClientList"),
    ];
    let app = app().await;
    for (path, schema) in cases {
        let response = app
            .clone()
            .oneshot(authorized(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path} status");
        let body = json_body(response).await;
        let node = schemas
            .get(*schema)
            .unwrap_or_else(|| panic!("OpenAPI declares schema {schema}"));
        let mut errors = Vec::new();
        assert_conforms(&body, node, &schemas, "$", &mut errors);
        assert!(
            errors.is_empty(),
            "{path} does not conform to schema {schema}: {errors:?}"
        );
    }
}

/// A config write returns the typed `ReloadReport` rather than the old generic
/// object (H-03).
#[tokio::test]
async fn toggle_response_matches_the_openapi_reload_report_schema() {
    // A reloadable app edits a real setting and returns ReloadReport; the
    // default test app has no configuration file and would fail closed with 500.
    let base = "schema_version = \"1\"\n\n\
                [data_policy]\ninference = \"remote_explicit\"\n\n\
                [inference]\nremote_endpoint = \"https://embeddings.example/v1\"\n\
                remote_model = \"text-embeddings\"\n\
                remote_timeout_ms = 5000\n";
    let (app, path) = reloadable_app(base).await;
    let response = app
        .oneshot(
            authorized("/inference")
                .method(Method::PATCH)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"remote_timeout_ms":6000}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    let openapi = include_str!("../openapi.yaml").replace('\r', "");
    let schemas = parse_schemas(&openapi);
    let node = schemas
        .get("ReloadReport")
        .expect("OpenAPI declares schema ReloadReport");
    let mut errors = Vec::new();
    assert_conforms(&body, node, &schemas, "$", &mut errors);
    assert!(
        errors.is_empty(),
        "toggle response does not conform to ReloadReport: {errors:?}"
    );
    let _ = std::fs::remove_file(path);
}

/// Regression for the earlier connector audit's H-01: no test asserted that
/// every route registered by `define_public_routes!` has an explicit arm in
/// `required_scope`. The fail-closed default protects a forgotten route (it
/// falls through to `Admin`, never public), but that safety was untested, so a
/// future route could reach the catch-all and no guard would notice. This table
/// pins the expected scope for every registered operation: adding or re-scoping
/// a route fails here until the table is updated, which is the route-to-guard
/// traceability the audit asked for.
#[test]
fn required_scope_covers_every_registered_route() {
    // (method, path, expected scope as Debug string, or None for unauthenticated)
    let expected: &[(Method, &str, Option<&str>)] = &[
        (Method::GET, "/health", None),
        (Method::GET, "/ready", None),
        (Method::GET, "/search", Some("Search")),
        (Method::POST, "/search", Some("Search")),
        (Method::GET, "/search/batch", Some("Search")),
        (Method::POST, "/deep", Some("Deep")),
        (Method::GET, "/providers", Some("Read")),
        (Method::GET, "/status", Some("Read")),
        (Method::GET, "/history", Some("Read")),
        (Method::DELETE, "/history", Some("Write")),
        (Method::DELETE, "/history/{id}", Some("Write")),
        (Method::GET, "/saved", Some("Read")),
        (Method::POST, "/saved", Some("Write")),
        (Method::DELETE, "/saved/{id}", Some("Write")),
        (Method::POST, "/reload", Some("Admin")),
        (Method::POST, "/providers/{name}/enabled", Some("Admin")),
        (Method::POST, "/providers/{name}/credential", Some("Admin")),
        (
            Method::DELETE,
            "/providers/{name}/credential",
            Some("Admin"),
        ),
        (Method::GET, "/providers/{name}", Some("Read")),
        (Method::PATCH, "/providers/{name}", Some("Admin")),
        (Method::PATCH, "/inference", Some("Admin")),
        (Method::GET, "/server/clients", Some("Admin")),
        (Method::POST, "/server/clients", Some("Admin")),
        (Method::PATCH, "/server/clients/{id}", Some("Admin")),
        (Method::DELETE, "/server/clients/{id}", Some("Admin")),
        (Method::POST, "/server/clients/{id}/rotate", Some("Admin")),
        (Method::GET, "/server/pending-config", Some("Read")),
        (Method::PATCH, "/server/pending-config", Some("Admin")),
        (Method::POST, "/data-policy", Some("Admin")),
        (Method::GET, "/policies", Some("Read")),
        (Method::PATCH, "/policies/{name}", Some("Admin")),
        (Method::GET, "/persistence", Some("Read")),
        (Method::PATCH, "/persistence", Some("Admin")),
        (Method::GET, "/persistence/backups", Some("Read")),
        (Method::POST, "/persistence/backup", Some("Admin")),
        (Method::GET, "/circuits", Some("Read")),
        (Method::POST, "/circuits/reset", Some("Admin")),
        (Method::GET, "/telemetry", Some("Read")),
        (Method::PATCH, "/telemetry", Some("Admin")),
        (Method::GET, "/deep/limits", Some("Read")),
        (Method::PATCH, "/deep/limits", Some("Admin")),
        (Method::PATCH, "/deep/extractor", Some("Admin")),
        (Method::PATCH, "/deep/renderer", Some("Admin")),
        (Method::GET, "/security-events", Some("Admin")),
        (Method::GET, "/metrics", None),
    ];

    // Every registered operation appears exactly once in the table, and vice
    // versa: the table cannot quietly drop a route or invent one.
    let mut registered: Vec<(Method, String)> = crate::routes::all_routes()
        .into_iter()
        .flat_map(|route| {
            route
                .methods
                .iter()
                .map(move |method| (method.clone(), route.path.to_owned()))
        })
        .collect();
    registered.sort();
    let mut listed: Vec<(Method, String)> = expected
        .iter()
        .map(|(method, path, _)| (method.clone(), (*path).to_owned()))
        .collect();
    listed.sort();
    assert_eq!(
        registered, listed,
        "the required_scope coverage table must list every registered route exactly once"
    );

    for (method, path, want) in expected {
        let scope = required_scope(method, path).map(|scope| format!("{scope:?}"));
        assert_eq!(
            scope.as_deref(),
            *want,
            "{method} {path}: required_scope mismatch"
        );
    }
}

#[tokio::test]
async fn history_and_saved_surfaces_fail_closed_without_persistence() {
    let app = app().await;
    for path in ["/history", "/saved"] {
        let response = app
            .clone()
            .oneshot(authorized(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = json_body(response).await;
        assert_eq!(body["error"]["code"], "storage_unavailable");
    }
}

#[tokio::test]
async fn search_is_recorded_in_history_and_can_be_deleted() {
    let (app, path) = persistent_app().await;
    let _ = app
        .clone()
        .oneshot(
            authorized("/search?q=recorded+history+entry")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let response = app
        .clone()
        .oneshot(authorized("/history").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    let entries = body["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["raw_query"], "recorded history entry");
    assert_eq!(entries[0]["surface"], "api");
    let id = entries[0]["id"].as_i64().unwrap();

    let response = app
        .clone()
        .oneshot(
            authorized(&format!("/history/{id}"))
                .method(Method::DELETE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let missing = app
        .clone()
        .oneshot(
            authorized(&format!("/history/{id}"))
                .method(Method::DELETE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn saved_documents_round_trip_and_reject_invalid_input() {
    let (app, path) = persistent_app().await;
    let valid = json!({
        "canonical_url": "https://example.com/a",
        "title": "Example",
        "snippet": "snippet",
        "content_hash": "a".repeat(64),
        "extractor_version": "trafilatura-2.2.0-cli-json-v1",
        "payload": "{\"kept\":true}",
        "source_query": "example",
        "tags": []
    });
    let response = app
        .clone()
        .oneshot(
            authorized("/saved")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(valid.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let id = json_body(response).await["id"].as_i64().unwrap();

    let listed = app
        .clone()
        .oneshot(authorized("/saved").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let documents = json_body(listed).await;
    assert_eq!(documents["documents"].as_array().unwrap().len(), 1);
    assert_eq!(
        documents["documents"][0]["canonical_url"],
        "https://example.com/a"
    );

    // A non-SHA-256 content hash is rejected before touching SQLite.
    let invalid = json!({
        "canonical_url": "https://example.com/b",
        "content_hash": "not-a-hash",
        "extractor_version": "v1",
        "payload": "{}"
    });
    let rejected = app
        .clone()
        .oneshot(
            authorized("/saved")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(invalid.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);

    let deleted = app
        .clone()
        .oneshot(
            authorized(&format!("/saved/{id}"))
                .method(Method::DELETE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::OK);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn metrics_expose_latency_quantiles_sources_and_cache_gauges() {
    let app = app().await;
    let _ = app
        .clone()
        .oneshot(
            authorized("/search?q=latency+quantile+sample")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let response = app
        .oneshot(request("/metrics").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let body = String::from_utf8(
        to_bytes(response.into_body(), 128 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    for name in [
        "amatl_search_latency_ms",
        "amatl_deep_latency_ms",
        "amatl_search_latency_samples",
        "amatl_sources_declared",
        "amatl_sources_available",
        "amatl_sources_circuit_open",
        "amatl_cache_hits_total",
        "amatl_cache_misses_total",
        "amatl_cache_hit_rate",
        "amatl_storage_available",
    ] {
        assert!(body.contains(&format!("# HELP {name} ")), "missing {name}");
        assert!(
            body.lines().any(|line| line.starts_with(name)),
            "missing sample line for {name}"
        );
    }
    // The worst-case source gauges are declared unconditionally but only
    // sampled once a source has observations, so only their header is
    // guaranteed here.
    for name in [
        "amatl_sources_success_rate_min",
        "amatl_sources_latency_ms_max",
    ] {
        assert!(body.contains(&format!("# HELP {name} ")), "missing {name}");
    }
    assert!(body.contains("amatl_search_latency_ms{quantile=\"0.95\"}"));
    assert!(body.contains("amatl_cache_hit_rate{cache=\"document\"}"));
    // `/metrics` is public like `/ready`, so it must not name the sources this
    // deployment declares — that inventory is what `/status` (read scope) is
    // for.
    assert!(
        !body.contains("source=\""),
        "public metrics must not label a series with a source name"
    );
    assert!(body.ends_with('\n'));
}

// ── Reload and MCP symmetry ─────────────────────────────────────────

/// Router reading a real configuration file so `/reload` has something to
/// re-read.
async fn reloadable_app(body: &str) -> (Router, std::path::PathBuf) {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "amatl-server-reload-{}-{nonce}.toml",
        std::process::id()
    ));
    std::fs::write(&path, body).unwrap();
    let config = amatl_core::Config::load_optional(&path).unwrap();
    let router = build_router_with_config_path(
        AmatlService::new(config, true).await,
        Some(TOKEN.into()),
        Some(path.clone()),
    )
    .await
    .unwrap();
    (router, path)
}

#[tokio::test]
async fn reload_requires_the_bearer_token() {
    let response = app()
        .await
        .oneshot(
            request("/reload")
                .method(Method::POST)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn reload_picks_up_a_new_source_without_restarting() {
    let base = "schema_version = \"1\"\n\n[providers]\nenabled = []\n";
    let (app, path) = reloadable_app(base).await;
    let before = json_body(
        app.clone()
            .oneshot(authorized("/status").body(Body::empty()).unwrap())
            .await
            .unwrap(),
    )
    .await;
    assert!(before["sources"].is_array());

    // Declare and enable a source in the same file the server was started with.
    std::fs::write(
        &path,
        "schema_version = \"1\"\n\n[providers]\nenabled = []\n\n\
         [circuit_breaker]\nfailure_threshold = 7\n",
    )
    .unwrap();
    let response = app
        .clone()
        .oneshot(
            authorized("/reload")
                .method(Method::POST)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let report = json_body(response).await;
    assert_eq!(report["schema_version"], "1");
    assert!(report["config_file"].is_string());
    assert!(report["registered_sources"]
        .as_array()
        .unwrap()
        .iter()
        .all(|value| value != "unshipped_source"));
    // The operating source is present.
    assert!(["marginalia"]
        .iter()
        .all(|source| report["registered_sources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == source)));

    // The service really was replaced: the new limit is in force.
    let after = app
        .oneshot(authorized("/status").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(after.status(), StatusCode::OK);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn a_rejected_reload_leaves_the_running_service_in_place() {
    let (app, path) = reloadable_app("schema_version = \"1\"\n").await;
    // schema_version mismatch is refused by configuration validation.
    std::fs::write(&path, "schema_version = \"999\"\n").unwrap();
    let response = app
        .clone()
        .oneshot(
            authorized("/reload")
                .method(Method::POST)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        json_body(response).await["error"]["code"],
        "configuration_invalid"
    );

    let still_serving = app
        .oneshot(authorized("/status").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(still_serving.status(), StatusCode::OK);
    let _ = std::fs::remove_file(path);
}

/// A process started with a listener override: the running service was built
/// for `0.0.0.0` with TLS, while the file on disk declares `127.0.0.1`.
///
/// This is the shape `amatl serve --bind 0.0.0.0` produces, because the
/// override is applied to the running configuration and never written back to
/// the file. Every check that trusts the document's declared `bind` is
/// therefore checking an address this process is not listening on.
async fn listener_override_app(body: &str) -> (Router, std::path::PathBuf) {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "amatl-server-listener-{}-{nonce}.toml",
        std::process::id()
    ));
    std::fs::write(&path, body).unwrap();
    let mut config = amatl_core::Config::default();
    config.server.bind = "0.0.0.0".into();
    config.server.tls.cert_path = Some("cert.pem".into());
    config.server.tls.key_path = Some("key.pem".into());
    config.validate().unwrap();
    let router = build_router_with_config_path(
        AmatlService::new(config, true).await,
        Some(TOKEN.into()),
        Some(path.clone()),
    )
    .await
    .unwrap();
    (router, path)
}

#[tokio::test]
async fn reload_refuses_a_document_that_is_unsafe_for_the_effective_listener() {
    // The file declares loopback, which is all `Config::validate` can see, but
    // the process serves every interface. Flipping `no_auth` on must be
    // refused: accepting it would publish an unauthenticated service.
    let base = "schema_version = \"1\"\n\n[server]\nbind = \"127.0.0.1\"\n";
    let (app, path) = listener_override_app(base).await;
    assert_eq!(
        app.clone()
            .oneshot(request("/status").body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );

    std::fs::write(
        &path,
        "schema_version = \"1\"\n\n[server]\nbind = \"127.0.0.1\"\nno_auth = true\n",
    )
    .unwrap();
    let response = app
        .clone()
        .oneshot(
            authorized("/reload")
                .method(Method::POST)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        json_body(response).await["error"]["code"],
        "configuration_invalid"
    );

    // Unchanged on both counts: still refusing anonymous callers, still
    // serving authenticated requests.
    assert_eq!(
        app.clone()
            .oneshot(request("/status").body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.oneshot(authorized("/status").body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn pending_server_patch_cannot_weaken_the_effective_listener() {
    // The same divergence, reached the way an operator reaches it: a reload of
    // a document that declares loopback with TLS and authentication is
    // accepted, and leaves the running configuration disagreeing with the
    // socket. The `[server]` patch must still be judged against the socket.
    let base = "schema_version = \"1\"\n\n[server]\nbind = \"127.0.0.1\"\n\n\
                [server.tls]\ncert_path = \"cert.pem\"\nkey_path = \"key.pem\"\n";
    let (app, path) = listener_override_app(base).await;
    let reloaded = app
        .clone()
        .oneshot(
            authorized("/reload")
                .method(Method::POST)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reloaded.status(), StatusCode::OK);

    let response = app
        .oneshot(
            authorized("/server/pending-config")
                .method(Method::PATCH)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"no_auth": true}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        json_body(response).await["error"]["code"],
        "config_rejected"
    );

    // Nothing was staged for the next restart either.
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(!on_disk.contains("no_auth"));
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn mcp_status_tool_reports_the_limits_actually_in_force() {
    let app = app().await;
    let call = mcp_request(
        "tools/call",
        json!({
            "name": "status",
            "arguments": {},
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientInfo": { "name": "contract-test", "version": "1" },
                "io.modelcontextprotocol/clientCapabilities": {}
            }
        }),
    );
    let response = app
        .oneshot(
            authorized("/mcp")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/json")
                .header("accept", "application/json, text/event-stream")
                .header("mcp-protocol-version", "2026-07-28")
                .header("mcp-method", "tools/call")
                .header("mcp-name", "status")
                .body(Body::from(call.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    let limits = &body["result"]["structuredContent"]["limits"];
    let config = amatl_core::Config::default();
    let expected = amatl_core::ExecutionLimits::for_surface(&config, ServiceSurface::mcp());
    assert_eq!(limits["fetch_timeout_ms"], expected.fetch_timeout_ms);
    assert_eq!(limits["fetch_max_bytes"], expected.fetch_max_bytes);
    assert_eq!(limits["fetch_max_redirects"], expected.fetch_max_redirects);
    assert_eq!(limits["max_page_size"], expected.max_page_size);
    // MCP stays strictly tighter than the local surfaces it shares a core with.
    let cli = amatl_core::ExecutionLimits::for_surface(&config, ServiceSurface::cli());
    assert!(expected.fetch_max_bytes <= cli.fetch_max_bytes);
    assert!(expected.max_page_size < cli.max_page_size);
}

#[tokio::test]
async fn mcp_search_accepts_server_side_pagination() {
    let app = app().await;
    let call = mcp_request(
        "tools/call",
        json!({
            "name": "search",
            "arguments": { "query": "rust", "page": 0, "page_size": 1 },
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientInfo": { "name": "contract-test", "version": "1" },
                "io.modelcontextprotocol/clientCapabilities": {}
            }
        }),
    );
    let response = app
        .oneshot(
            authorized("/mcp")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/json")
                .header("accept", "application/json, text/event-stream")
                .header("mcp-protocol-version", "2026-07-28")
                .header("mcp-method", "tools/call")
                .header("mcp-name", "search")
                .body(Body::from(call.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    let content = &body["result"]["structuredContent"];
    assert_eq!(content["schema_version"], "1");
    assert_eq!(content["page"], 0);
    assert_eq!(content["page_size"], 1);
    assert!(content["results"].as_array().unwrap().len() <= 1);
}

// ── Credentials, scopes and per-tool authorization ──────────────────

const SEARCH_ONLY_TOKEN: &str = "search-only-client-token-0123456789ab";
const ADMIN_TOKEN: &str = "admin-client-token-0123456789abcdef01";

fn sha256_hex(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

/// Configuration with two named clients: one restricted to Search plus the
/// `search` MCP tool, one with everything.
fn scoped_config() -> amatl_core::Config {
    let mut config = amatl_core::Config::default();
    config.server.clients = vec![
        amatl_core::ServerClient {
            id: "search_only".into(),
            token_sha256: Some(sha256_hex(SEARCH_ONLY_TOKEN)),
            scopes: vec![amatl_core::Scope::Search, amatl_core::Scope::Mcp],
            tools: vec!["search".into()],
            ..Default::default()
        },
        amatl_core::ServerClient {
            id: "operator".into(),
            token_sha256: Some(sha256_hex(ADMIN_TOKEN)),
            scopes: amatl_core::Scope::ALL.to_vec(),
            tools: MCP_TOOLS.iter().map(|tool| (*tool).to_owned()).collect(),
            ..Default::default()
        },
    ];
    config.validate().unwrap();
    config
}

async fn scoped_app() -> Router {
    build_router(
        AmatlService::new(scoped_config(), true).await,
        Some(TOKEN.into()),
    )
    .await
    .unwrap()
}

fn as_client(path: &str, token: &str) -> axum::http::request::Builder {
    request(path).header(AUTHORIZATION, format!("Bearer {token}"))
}

#[tokio::test]
async fn several_credentials_are_accepted_and_scoped_independently() {
    let app = scoped_app().await;

    // The restricted client may search…
    let allowed = app
        .clone()
        .oneshot(
            as_client("/search?q=scoped+client", SEARCH_ONLY_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(allowed.status(), StatusCode::OK);

    // …but not reach an operator surface, and the refusal is a scope refusal,
    // not a generic authentication failure.
    let denied = app
        .clone()
        .oneshot(
            as_client("/status", SEARCH_ONLY_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    assert_eq!(json_body(denied).await["error"]["code"], "scope_denied");

    // Reload is admin-only.
    let reload_denied = app
        .clone()
        .oneshot(
            as_client("/reload", SEARCH_ONLY_TOKEN)
                .method(Method::POST)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reload_denied.status(), StatusCode::FORBIDDEN);

    // The operator credential reaches all of them.
    for (path, method) in [("/status", Method::GET), ("/reload", Method::POST)] {
        let response = app
            .clone()
            .oneshot(
                as_client(path, ADMIN_TOKEN)
                    .method(method)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
    }

    // An unknown token is still rejected outright.
    let unknown = app
        .oneshot(
            as_client("/search?q=nope", "unknown-token-0123456789abcdefghij")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unknown.status(), StatusCode::UNAUTHORIZED);
}

#[test]
fn duplicate_resolved_credentials_fail_closed() {
    let mut config = scoped_config();
    config.server.clients[0].token_sha256 = Some(sha256_hex(SEARCH_ONLY_TOKEN));
    config.validate().unwrap();

    let result = resolve_clients(&config, Some(SEARCH_ONLY_TOKEN.into()));
    assert!(
        matches!(result, Err(ServerError::Configuration(_))),
        "a token must not resolve to multiple identities"
    );
}

#[test]
fn rate_limiter_windows_are_bounded() {
    let config = amatl_core::Config::default();
    let security = SecurityState {
        clients: resolve_clients(&config, Some(TOKEN.into())).unwrap(),
        allowed_hosts: config.server.allowed_hosts.clone(),
        allowed_origins: effective_origins(&config, false),
        max_header_bytes: config.server.max_header_bytes,
        max_body_bytes: config.server.max_body_bytes,
        timeout: Duration::from_millis(config.server.request_timeout_ms),
        rate_limit_per_minute: config.server.rate_limit_per_minute,
        https: false,
    };
    let limiter = Mutex::new(RateLimiter {
        windows: BTreeMap::new(),
        last_cleanup: Instant::now(),
    });

    for index in 0..=RATE_LIMITER_CAPACITY {
        let ip = IpAddr::V6(std::net::Ipv6Addr::from(index as u128 + 1));
        let mut request = Request::builder()
            .uri("/health")
            .body(Body::empty())
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(std::net::SocketAddr::new(ip, 8080)));
        assert!(within_rate_limit(&request, &security, &limiter));
    }
    assert_eq!(limiter.lock().unwrap().windows.len(), RATE_LIMITER_CAPACITY);
}

#[tokio::test]
async fn an_expired_credential_stops_being_accepted() {
    let mut config = scoped_config();
    config.server.clients[0].expires_at = Some("2020-01-01".into());
    config.validate().unwrap();
    let app = build_router(AmatlService::new(config, true).await, Some(TOKEN.into()))
        .await
        .unwrap();
    let response = app
        .oneshot(
            as_client("/search?q=expired", SEARCH_ONLY_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn provider_credential_requires_admin_scope() {
    let app = scoped_app().await;
    for method in [Method::POST, Method::DELETE] {
        let body = if method == Method::POST {
            Body::from(r#"{"key":"candidate-key"}"#)
        } else {
            Body::empty()
        };
        let response = app
            .clone()
            .oneshot(
                as_client("/providers/marginalia/credential", SEARCH_ONLY_TOKEN)
                    .method(method)
                    .header(CONTENT_TYPE, "application/json")
                    .body(body)
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(json_body(response).await["error"]["code"], "scope_denied");
    }
}

#[tokio::test]
async fn provider_credential_put_is_fail_closed_under_isolated() {
    let app = isolated_app().await;
    let response = app
        .clone()
        .oneshot(
            authorized("/providers/marginalia/credential")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"key":"candidate-key"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    // Egress is denied, so the live canary fails closed and nothing is stored.
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        json_body(response).await["error"]["code"],
        "invalid_request"
    );

    // The credential provenance of the *declared* providers belongs to
    // `/providers`; in mock mode `/status` names the mock sources that
    // `/search` actually answers with, so `marginalia` is asserted there.
    let providers = app
        .clone()
        .oneshot(authorized("/providers").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(providers.status(), StatusCode::OK);
    let body = json_body(providers).await;
    let marginalia = body["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|provider| provider["name"] == "marginalia")
        .expect("marginalia source is reported");
    // Never "ephemeral": the rejected key was not committed.
    assert_ne!(marginalia["credential_source"], "ephemeral");

    // Whichever sources `/status` reports, none of them may claim a credential
    // that the canary rejected.
    let status = app
        .oneshot(authorized("/status").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(status.status(), StatusCode::OK);
    let body = json_body(status).await;
    assert!(
        body["sources"]
            .as_array()
            .unwrap()
            .iter()
            .all(|source| source["credential_source"] != "ephemeral"),
        "no source may report a credential the canary rejected: {}",
        body["sources"]
    );
}

#[tokio::test]
async fn provider_credential_delete_is_idempotent_and_reports_absent() {
    let app = app().await;
    for _ in 0..2 {
        let response = app
            .clone()
            .oneshot(
                authorized("/providers/marginalia/credential")
                    .method(Method::DELETE)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = json_body(response).await;
        assert_eq!(body["name"], "marginalia");
        assert_eq!(body["set"], false);
        assert_eq!(body["available"], false);
    }
}

#[tokio::test]
async fn mcp_tools_are_authorized_per_client() {
    let app = scoped_app().await;
    let call = |tool: &str| {
        mcp_request(
            "tools/call",
            json!({
                "name": tool,
                "arguments": if tool == "fetch" {
                    json!({ "url": "https://example.com/" })
                } else {
                    json!({ "query": "rust" })
                },
                "_meta": {
                    "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                    "io.modelcontextprotocol/clientInfo": { "name": "scoped", "version": "1" },
                    "io.modelcontextprotocol/clientCapabilities": {}
                }
            }),
        )
    };
    let send = |tool: &'static str, token: &'static str, app: Router| async move {
        let response = app
            .oneshot(
                as_client("/mcp", token)
                    .method(Method::POST)
                    .header(CONTENT_TYPE, "application/json")
                    .header("accept", "application/json, text/event-stream")
                    .header("mcp-protocol-version", "2026-07-28")
                    .header("mcp-method", "tools/call")
                    .header("mcp-name", tool)
                    .body(Body::from(call(tool).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        json_body(response).await
    };

    // The restricted client may call `search`…
    let allowed = send("search", SEARCH_ONLY_TOKEN, app.clone()).await;
    assert_ne!(allowed["result"]["isError"], true, "{allowed}");

    // …and is refused `fetch`, the sensitive tool, without disabling egress
    // for anyone else.
    let denied = send("fetch", SEARCH_ONLY_TOKEN, app.clone()).await;
    assert_eq!(denied["result"]["isError"], true);
    assert_eq!(
        denied["result"]["structuredContent"]["error"]["code"],
        "scope_denied"
    );

    // A client-supplied tool header cannot widen the allowlist. The transport
    // refuses a header that disagrees with the body before any tool runs, and
    // the tool itself decides from the authenticated identity, never the
    // header — so neither half of the spoof works.
    let spoofed = app
        .clone()
        .oneshot(
            as_client("/mcp", SEARCH_ONLY_TOKEN)
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/json")
                .header("accept", "application/json, text/event-stream")
                .header("mcp-protocol-version", "2026-07-28")
                .header("mcp-method", "tools/call")
                .header("mcp-name", "search")
                .body(Body::from(call("fetch").to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let spoofed = json_body(spoofed).await;
    assert!(spoofed["result"].is_null(), "{spoofed}");
    assert!(
        spoofed["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("does not match body value"),
        "{spoofed}"
    );

    // The operator credential keeps full access.
    let operator = send("providers", ADMIN_TOKEN, app).await;
    assert_ne!(operator["result"]["isError"], true, "{operator}");
}

#[tokio::test]
async fn reload_rotates_credentials_without_restarting() {
    let rotated = "rotated-operator-token-0123456789abcd";
    let base = format!(
        "schema_version = \"1\"\n\n[[server.clients]]\nid = \"operator\"\ntoken_sha256 = \"{}\"\nscopes = [\"read\", \"admin\"]\n",
        sha256_hex(ADMIN_TOKEN)
    );
    let (app, path) = reloadable_app(&base).await;

    assert_eq!(
        app.clone()
            .oneshot(
                as_client("/status", ADMIN_TOKEN)
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );

    std::fs::write(
        &path,
        format!(
            "schema_version = \"1\"\n\n[[server.clients]]\nid = \"operator\"\ntoken_sha256 = \"{}\"\nscopes = [\"read\", \"admin\"]\n",
            sha256_hex(rotated)
        ),
    )
    .unwrap();
    let report = app
        .clone()
        .oneshot(
            as_client("/reload", ADMIN_TOKEN)
                .method(Method::POST)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(report.status(), StatusCode::OK);
    let report = json_body(report).await;
    let clients = report["clients"].as_array().unwrap();
    assert!(
        clients.iter().any(|client| client == "operator"),
        "{report}"
    );

    // The old secret stops working and the new one starts, same process.
    assert_eq!(
        app.clone()
            .oneshot(
                as_client("/status", ADMIN_TOKEN)
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.oneshot(as_client("/status", rotated).body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn security_rejections_are_persisted_and_queryable() {
    let (app, path) = persistent_app().await;

    // Two different rejections: no credential, and a bad one.
    for headers in [None, Some("Bearer wrong-token-0123456789abcdefghijkl")] {
        let mut builder = request("/search?q=audited");
        if let Some(value) = headers {
            builder = builder.header(AUTHORIZATION, value);
        }
        let response = app
            .clone()
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    // Auditing is backgrounded, so the trail settles a moment later.
    let mut body = serde_json::Value::Null;
    for _ in 0..50 {
        let response = app
            .clone()
            .oneshot(
                authorized("/security-events?limit=10")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        body = json_body(response).await;
        if body["events"].as_array().unwrap().len() >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let events = body["events"].as_array().unwrap();
    assert!(events.len() >= 2, "{body}");
    assert!(events
        .iter()
        .all(|event| event["event"] == "unauthorized" && event["path"] == "/search"));
    assert!(events[0]["request_id"].is_string());
    assert_eq!(body["dropped"], 0);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn the_audit_trail_is_admin_only_and_fails_closed_without_persistence() {
    // Persistence disabled: the endpoint reports it instead of returning an
    // empty trail that looks like "nothing happened".
    let response = app()
        .await
        .oneshot(authorized("/security-events").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        json_body(response).await["error"]["code"],
        "storage_unavailable"
    );

    // A non-admin credential cannot read it at all.
    let scoped = scoped_app().await;
    let denied = scoped
        .oneshot(
            as_client("/security-events", SEARCH_ONLY_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
}

// ── Server client administration ─────────────────────────────────────

#[tokio::test]
async fn server_clients_endpoints_require_the_admin_scope() {
    let scoped = scoped_app().await;
    for (path, method) in [
        ("/server/clients", Method::GET),
        ("/server/clients", Method::POST),
        ("/server/clients/search_only", Method::DELETE),
        ("/server/clients/search_only/rotate", Method::POST),
    ] {
        let response = scoped
            .clone()
            .oneshot(
                as_client(path, SEARCH_ONLY_TOKEN)
                    .method(method.clone())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path} {method}");
    }
}

#[tokio::test]
async fn create_server_client_mints_a_token_and_persists_only_its_digest() {
    let (app, path) = reloadable_app("schema_version = \"1\"\n").await;

    let response = app
        .clone()
        .oneshot(
            authorized("/server/clients")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"id": "dashboard", "scopes": ["read", "write"], "tools": []})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    let token = body["token"].as_str().unwrap().to_string();
    assert_eq!(token.len(), 64, "expected a 256-bit hex token");
    assert_eq!(body["id"], "dashboard");
    assert_eq!(body["credential_kind"], "token_sha256");

    // The raw token never touches disk — only its digest does.
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(!on_disk.contains(&token));
    assert!(on_disk.contains(&sha256_hex(&token)));

    // The minted token authenticates immediately, without a restart.
    let authenticated = app
        .oneshot(as_client("/status", &token).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(authenticated.status(), StatusCode::OK);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn create_server_client_refuses_a_scopeless_client_without_writing() {
    let (app, path) = reloadable_app("schema_version = \"1\"\n").await;

    let response = app
        .oneshot(
            authorized("/server/clients")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"id": "dashboard", "scopes": []}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        json_body(response).await["error"]["code"],
        "config_rejected"
    );
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(!on_disk.contains("dashboard"));
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn list_server_clients_never_exposes_a_credential() {
    let (app, path) = reloadable_app("schema_version = \"1\"\n").await;
    app.clone()
        .oneshot(
            authorized("/server/clients")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"id": "dashboard", "scopes": ["read"]}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let response = app
        .oneshot(authorized("/server/clients").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    let clients = body["clients"].as_array().unwrap();
    assert_eq!(clients.len(), 1);
    assert_eq!(clients[0]["id"], "dashboard");
    assert!(clients[0].get("token").is_none());
    assert!(clients[0].get("token_sha256").is_none());
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn create_server_client_rejects_a_duplicate_id_without_overwriting() {
    // Regression: POST used to `retain` the id away and re-push, silently
    // replacing the existing entry — minting a new credential that stopped
    // authenticating whoever held the old one, with no warning.
    let (app, path) = reloadable_app("schema_version = \"1\"\n").await;
    let created = json_body(
        app.clone()
            .oneshot(
                authorized("/server/clients")
                    .method(Method::POST)
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({"id": "dashboard", "scopes": ["read"]}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    let original_token = created["token"].as_str().unwrap().to_string();

    let duplicate = app
        .clone()
        .oneshot(
            authorized("/server/clients")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"id": "dashboard", "scopes": ["read", "admin"]}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(duplicate.status(), StatusCode::CONFLICT);
    assert_eq!(json_body(duplicate).await["error"]["code"], "conflict");

    // Nothing changed on disk and the original credential still authenticates.
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains(&sha256_hex(&original_token)));
    let authenticated = app
        .oneshot(
            as_client("/status", &original_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(authenticated.status(), StatusCode::OK);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn update_server_client_changes_scopes_without_rotating_the_credential() {
    let (app, path) = reloadable_app("schema_version = \"1\"\n").await;
    let created = json_body(
        app.clone()
            .oneshot(
                authorized("/server/clients")
                    .method(Method::POST)
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({"id": "dashboard", "scopes": ["read"]}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    let token = created["token"].as_str().unwrap().to_string();

    let response = app
        .clone()
        .oneshot(
            authorized("/server/clients/dashboard")
                .method(Method::PATCH)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"scopes": ["read", "write"]}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["client"]["scopes"], json!(["read", "write"]));

    // The original token, untouched by the update, still authenticates.
    let authenticated = app
        .oneshot(as_client("/status", &token).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(authenticated.status(), StatusCode::OK);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn update_server_client_reports_not_found_for_an_unknown_id() {
    let (app, path) = reloadable_app("schema_version = \"1\"\n").await;
    let response = app
        .oneshot(
            authorized("/server/clients/nobody")
                .method(Method::PATCH)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"scopes": ["read"]}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn delete_server_client_revokes_access_immediately() {
    let (app, path) = reloadable_app("schema_version = \"1\"\n").await;
    let created = json_body(
        app.clone()
            .oneshot(
                authorized("/server/clients")
                    .method(Method::POST)
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({"id": "dashboard", "scopes": ["read"]}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    let token = created["token"].as_str().unwrap().to_string();

    let deleted = app
        .clone()
        .oneshot(
            authorized("/server/clients/dashboard")
                .method(Method::DELETE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::OK);

    let revoked = app
        .clone()
        .oneshot(as_client("/status", &token).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(revoked.status(), StatusCode::UNAUTHORIZED);

    // Deleting it again is a clean not-found, not a crash.
    let again = app
        .oneshot(
            authorized("/server/clients/dashboard")
                .method(Method::DELETE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(again.status(), StatusCode::NOT_FOUND);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn rotate_server_client_token_invalidates_the_old_token() {
    let (app, path) = reloadable_app("schema_version = \"1\"\n").await;
    let created = json_body(
        app.clone()
            .oneshot(
                authorized("/server/clients")
                    .method(Method::POST)
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({"id": "dashboard", "scopes": ["read"]}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    let original_token = created["token"].as_str().unwrap().to_string();

    let rotated = json_body(
        app.clone()
            .oneshot(
                authorized("/server/clients/dashboard/rotate")
                    .method(Method::POST)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    let new_token = rotated["token"].as_str().unwrap().to_string();
    assert_ne!(original_token, new_token);
    // Scope carried forward from the entry being rotated.
    assert_eq!(rotated["scopes"], json!(["read"]));

    let old_rejected = app
        .clone()
        .oneshot(
            as_client("/status", &original_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(old_rejected.status(), StatusCode::UNAUTHORIZED);

    let new_accepted = app
        .oneshot(
            as_client("/status", &new_token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(new_accepted.status(), StatusCode::OK);
    let _ = std::fs::remove_file(path);
}

// ── Data policy administration ───────────────────────────────────────

#[tokio::test]
async fn data_policy_update_requires_the_admin_scope() {
    let scoped = scoped_app().await;
    let response = scoped
        .oneshot(
            as_client("/data-policy", SEARCH_ONLY_TOKEN)
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"egress": "deny"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn data_policy_update_changes_only_the_requested_fields_and_applies_without_restart() {
    let base = "schema_version = \"1\"\n\n\
                # A comment an operator wrote about something unrelated.\n\
                [data_policy]\nprofile = \"standard\"\negress = \"governed\"\ninference = \"disabled\"\n";
    let (app, path) = reloadable_app(base).await;

    let response = app
        .clone()
        .oneshot(
            authorized("/data-policy")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"inference": "local_only"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains("# A comment an operator wrote about something unrelated."));
    assert!(on_disk.contains("inference = \"local_only\""));
    // Untouched: only `inference` was in the request body.
    assert!(on_disk.contains("profile = \"standard\""));
    assert!(on_disk.contains("egress = \"governed\""));

    let status = json_body(
        app.oneshot(authorized("/status").body(Body::empty()).unwrap())
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status["data_policy"]["inference"], "local_only");
    assert_eq!(status["data_policy"]["profile"], "standard");
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn data_policy_update_refuses_a_contradictory_combination_without_writing() {
    // isolated requires denied egress; asking for isolated while leaving
    // egress governed must fail Config::validate and never touch the file.
    let base = "schema_version = \"1\"\n\n\
                [data_policy]\nprofile = \"standard\"\negress = \"governed\"\n";
    let (app, path) = reloadable_app(base).await;

    let response = app
        .oneshot(
            authorized("/data-policy")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"profile": "isolated"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        json_body(response).await["error"]["code"],
        "config_rejected"
    );
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains("profile = \"standard\""));
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn patch_endpoint_rejects_a_value_that_cannot_be_stored() {
    // Regression: `usize::MAX` used to pass validation (only `!= 0` was
    // checked), was cast `as i64` on write (wrapping to -1), and corrupted
    // the file for the next reload. The candidate must fail `Config::validate`
    // before anything touches the file.
    let base = "schema_version = \"1\"\n\n[inference]\nmax_documents = 64\n";
    let (app, path) = reloadable_app(base).await;

    let response = app
        .oneshot(
            authorized("/inference")
                .method(Method::PATCH)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"max_documents": usize::MAX}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        json_body(response).await["error"]["code"],
        "config_rejected"
    );

    // The file is byte-for-byte untouched; in particular it never gains a
    // negative value that the next reload would refuse to parse.
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains("max_documents = 64"));
    assert!(!on_disk.contains("-1"), "file corrupted: {on_disk}");
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn unknown_paths_are_admin_protected_and_public_surfaces_stay_public() {
    // The scope matcher fails closed: a path with no declared arm is
    // admin-protected (the structural guarantee `docs/arquitectura.md`
    // promises), while the explicitly public surfaces keep working without
    // a credential.
    let app = app().await;

    // Public surfaces: liveness probes, the metrics scrape, UI assets.
    // `/ready` is public too but reports 503 while persistence (the thing it
    // checks) is disabled in the default test app — the point is that it is
    // reachable *without a credential*, not its payload.
    for path in ["/health", "/metrics", "/", "/app.js", "/styles.css"] {
        let response = app
            .clone()
            .oneshot(request(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path} must stay public");
    }
    let ready = app
        .clone()
        .oneshot(request("/ready").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_ne!(
        ready.status(),
        StatusCode::UNAUTHORIZED,
        "/ready must be public"
    );

    // An undeclared path is protected by default: no credential -> 401,
    // a non-admin credential -> 403, an admin credential -> reaches the
    // fallback and 404s cleanly.
    let scoped = scoped_app().await;
    let anonymous = app
        .clone()
        .oneshot(
            request("/some-undeclared-route")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);

    let denied = scoped
        .oneshot(
            as_client("/some-undeclared-route", SEARCH_ONLY_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);

    let admin = app
        .clone()
        .oneshot(
            authorized("/some-undeclared-route")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(admin.status(), StatusCode::NOT_FOUND);
}
/// The public-surface exemption is read-only. It is decided *before* routing,
/// so exempting a path by itself would publish a future `POST` on one of those
/// paths — or any state-changing route added under an asset path — as
/// unauthenticated. A mutating method now falls through to the same fail-closed
/// default as an undeclared path, while the read-only methods those routes
/// actually serve stay public.
#[tokio::test]
async fn mutating_methods_do_not_inherit_the_public_surface_exemption() {
    let app = app().await;

    for path in ["/health", "/metrics", "/app.js"] {
        let response = app
            .clone()
            .oneshot(request(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "GET {path} must stay public"
        );
    }

    for (method, path) in [
        (Method::POST, "/health"),
        (Method::DELETE, "/metrics"),
        (Method::POST, "/"),
        (Method::PATCH, "/app.js"),
    ] {
        let response = app
            .clone()
            .oneshot(
                request(path)
                    .method(method.clone())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{method} {path} must not be public"
        );
    }
}

// ── Provider governance ficha and inference advanced fields ───

#[tokio::test]
async fn provider_record_is_read_scoped_and_reports_not_found_for_an_undeclared_name() {
    let app = app().await;
    let response = app
        .clone()
        .oneshot(
            authorized("/providers/marginalia")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["name"], "marginalia");
    assert!(body["record"]["adapter_version"].is_string());

    let missing = app
        .oneshot(
            authorized("/providers/does_not_exist")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn update_provider_record_edits_the_ficha_without_enabling_traffic() {
    let base = "schema_version = \"1\"\n\n\
                # A comment an operator wrote about something unrelated.\n\
                [providers]\nenabled = []\n";
    let (app, path) = reloadable_app(base).await;

    let response = app
        .clone()
        .oneshot(
            authorized("/providers/custom_archive")
                .method(Method::PATCH)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "adapter_version": "custom-v1",
                        "approval_status": "approved",
                        "reviewer": "Alexis Hernandez",
                        "supported_regions": ["us", "eu"]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["record"]["adapter_version"], "custom-v1");
    assert_eq!(body["record"]["approval_status"], "approved");

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains("# A comment an operator wrote about something unrelated."));
    assert!(on_disk.contains("custom-v1"));

    // The ficha exists now, but the source was never added to
    // `providers.enabled` — editing governance never turns traffic on.
    let reparsed = amatl_core::Config::load_optional(&path).unwrap();
    assert!(!reparsed
        .providers
        .enabled
        .contains(&"custom_archive".to_string()));
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn update_provider_record_requires_the_admin_scope() {
    let scoped = scoped_app().await;
    let response = scoped
        .oneshot(
            as_client("/providers/marginalia", SEARCH_ONLY_TOKEN)
                .method(Method::PATCH)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"reviewer": "someone"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn update_inference_changes_only_the_requested_fields_and_applies_without_restart() {
    let base = "schema_version = \"1\"\n\n\
                # A comment an operator wrote about something unrelated.\n\
                [data_policy]\ninference = \"remote_explicit\"\n\n\
                [inference]\nremote_endpoint = \"https://old.example/embeddings\"\n\
                remote_model = \"old-model\"\n\
                remote_timeout_ms = 5000\n";
    let (app, path) = reloadable_app(base).await;

    let response = app
        .clone()
        .oneshot(
            authorized("/inference")
                .method(Method::PATCH)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "remote_endpoint": "https://api.deepinfra.com/v1/openai/embeddings",
                        "remote_model": "BAAI/bge-base-en-v1.5"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains("# A comment an operator wrote about something unrelated."));
    assert!(on_disk.contains("api.deepinfra.com"));
    // Untouched: not part of the request body.
    assert!(on_disk.contains("remote_timeout_ms = 5000"));

    let reparsed = amatl_core::Config::load_optional(&path).unwrap();
    assert_eq!(
        reparsed.inference.remote_endpoint.as_deref(),
        Some("https://api.deepinfra.com/v1/openai/embeddings")
    );
    assert_eq!(reparsed.inference.remote_timeout_ms, 5000);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn update_inference_refuses_clearing_the_endpoint_out_from_under_remote_explicit() {
    // Starts valid (remote_explicit with endpoint/model set); clearing the
    // endpoint via the empty-string convention while the mode stays
    // remote_explicit must be refused by Config::validate and never written.
    let base = "schema_version = \"1\"\n\n\
                [data_policy]\ninference = \"remote_explicit\"\n\n\
                [inference]\nremote_endpoint = \"https://old.example/embeddings\"\n\
                remote_model = \"old-model\"\n";
    let (app, path) = reloadable_app(base).await;

    let response = app
        .oneshot(
            authorized("/inference")
                .method(Method::PATCH)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"remote_endpoint": ""}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains("https://old.example/embeddings"));
    let _ = std::fs::remove_file(path);
}

// ── Search-quality policies ────────────────────────────────────────────

#[tokio::test]
async fn policies_reports_all_five_and_is_read_scoped() {
    let scoped = scoped_app().await;
    let response = scoped
        .clone()
        .oneshot(
            as_client("/policies", SEARCH_ONLY_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let response = scoped
        .oneshot(
            as_client("/policies", ADMIN_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert!(body["ranking_policy"]["version"].is_string());
    assert!(body["diversity_policy"].is_object());
    assert!(body["search_policy"].is_object());
    assert!(body["ranking_v2_policy"].is_object());
    assert!(body["gap_policy"].is_object());
}

#[tokio::test]
async fn update_policy_replaces_ranking_and_applies_without_restart() {
    let base = "schema_version = \"1\"\n\n\
                # A comment an operator wrote about something unrelated.\n\
                [ranking_policy]\nversion = \"v1\"\nrrf_k = 60\n";
    let (app, path) = reloadable_app(base).await;

    let current = json_body(
        app.clone()
            .oneshot(authorized("/policies").body(Body::empty()).unwrap())
            .await
            .unwrap(),
    )
    .await;
    let mut ranking = current["ranking_policy"].clone();
    ranking["rrf_k"] = json!(42);

    let response = app
        .clone()
        .oneshot(
            authorized("/policies/ranking")
                .method(Method::PATCH)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(ranking.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains("# A comment an operator wrote about something unrelated."));
    assert!(on_disk.contains("rrf_k = 42"));

    let updated = json_body(
        app.oneshot(authorized("/policies").body(Body::empty()).unwrap())
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(updated["ranking_policy"]["rrf_k"], 42);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn update_policy_requires_the_admin_scope() {
    let scoped = scoped_app().await;
    let ranking = json_body(
        scoped
            .clone()
            .oneshot(
                as_client("/policies", ADMIN_TOKEN)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await["ranking_policy"]
        .clone();

    let response = scoped
        .oneshot(
            as_client("/policies/ranking", SEARCH_ONLY_TOKEN)
                .method(Method::PATCH)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(ranking.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn update_policy_rejects_an_unknown_policy_name() {
    let (app, path) = reloadable_app("schema_version = \"1\"\n").await;
    let response = app
        .oneshot(
            authorized("/policies/nonsense")
                .method(Method::PATCH)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json!({}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn update_policy_diversity_alone_can_fail_the_search_policy_cross_check() {
    let (app, path) = reloadable_app("schema_version = \"1\"\n").await;
    let current = json_body(
        app.clone()
            .oneshot(authorized("/policies").body(Body::empty()).unwrap())
            .await
            .unwrap(),
    )
    .await;
    let mut diversity = current["diversity_policy"].clone();
    let bumped = diversity["max_visible_per_domain"].as_u64().unwrap() + 1;
    diversity["max_visible_per_domain"] = json!(bumped);

    let response = app
        .oneshot(
            authorized("/policies/diversity")
                .method(Method::PATCH)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(diversity.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    // `diversity_policy.max_visible_per_domain` now disagrees with
    // `search_policy.maximum_results_per_domain` — Config::validate refuses
    // the pair, so the write never happens.
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let _ = std::fs::remove_file(path);
}

// ── Persistence, backups, circuits and telemetry administration ───────

#[tokio::test]
async fn persistence_config_is_read_scoped_and_reports_current_settings() {
    let scoped = scoped_app().await;
    let denied = scoped
        .clone()
        .oneshot(
            as_client("/persistence", SEARCH_ONLY_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);

    let response = scoped
        .oneshot(
            as_client("/persistence", ADMIN_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["persistence"]["enabled"], false);
}

#[tokio::test]
async fn update_persistence_changes_only_the_requested_fields_and_applies_without_restart() {
    let base = "schema_version = \"1\"\n\n\
                # A comment an operator wrote about something unrelated.\n\
                [persistence]\nenabled = true\npath = \"amatl.sqlite3\"\n\
                history_retention_days = 90\n";
    let (app, path) = reloadable_app(base).await;

    let response = app
        .clone()
        .oneshot(
            authorized("/persistence")
                .method(Method::PATCH)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"auto_backup_enabled": true, "auto_backup_interval_seconds": 7200})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains("# A comment an operator wrote about something unrelated."));
    assert!(on_disk.contains("auto_backup_interval_seconds = 7200"));
    // Untouched: not part of the request body.
    assert!(on_disk.contains("path = \"amatl.sqlite3\""));
    assert!(on_disk.contains("history_retention_days = 90"));

    let status = json_body(
        app.oneshot(authorized("/persistence").body(Body::empty()).unwrap())
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status["persistence"]["auto_backup_enabled"], true);
    assert_eq!(status["persistence"]["path"], "amatl.sqlite3");
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn update_persistence_requires_the_admin_scope() {
    let scoped = scoped_app().await;
    let response = scoped
        .oneshot(
            as_client("/persistence", SEARCH_ONLY_TOKEN)
                .method(Method::PATCH)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"auto_backup_enabled": true}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn backups_fail_closed_without_persistence_and_round_trip_when_enabled() {
    // Without persistence, both surfaces report the same storage_unavailable
    // failure the history/saved endpoints already use.
    let app = app().await;
    let response = app
        .clone()
        .oneshot(
            authorized("/persistence/backups")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let response = app
        .oneshot(
            authorized("/persistence/backup")
                .method(Method::POST)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    // Enabled: a manual backup is written and then listed.
    let (app, path) = persistent_app().await;
    let created = app
        .clone()
        .oneshot(
            authorized("/persistence/backup")
                .method(Method::POST)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::OK);
    let created_body = json_body(created).await;
    let backup_path = created_body["path"].as_str().unwrap().to_string();

    let listed = json_body(
        app.oneshot(
            authorized("/persistence/backups")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap(),
    )
    .await;
    let backups = listed["backups"].as_array().unwrap();
    assert!(backups.iter().any(|entry| entry == &backup_path));

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&backup_path);
}

#[tokio::test]
async fn create_backup_requires_the_admin_scope() {
    let scoped = scoped_app().await;
    let response = scoped
        .oneshot(
            as_client("/persistence/backup", SEARCH_ONLY_TOKEN)
                .method(Method::POST)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

/// Paths `GET /persistence/backups` reports, newest first.
async fn list_backups(app: &Router) -> Vec<String> {
    let listed = json_body(
        app.clone()
            .oneshot(
                authorized("/persistence/backups")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    listed["backups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry.as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn create_backup_replays_a_repeated_idempotency_key() {
    // `create_backup` runs the write on a task that survives the request
    // deadline, so a client that timed out on a 504 and retries is doing the
    // normal thing — and used to get a second backup file, which rotation can
    // then use to evict an older one the operator wanted to keep.
    let (app, path) = persistent_app().await;
    // Opening the store already wrote a migration backup, so the assertions
    // below compare against that baseline instead of an absolute count.
    let before = list_backups(&app).await;
    let mut paths = Vec::new();
    for _ in 0..2 {
        let response = app
            .clone()
            .oneshot(
                authorized("/persistence/backup")
                    .method(Method::POST)
                    .header("idempotency-key", "before-migration")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        paths.push(
            json_body(response).await["path"]
                .as_str()
                .unwrap()
                .to_owned(),
        );
    }
    assert_eq!(
        paths[0], paths[1],
        "a repeated key must replay the backup it already wrote"
    );

    let after = list_backups(&app).await;
    assert_eq!(
        after.len(),
        before.len() + 1,
        "a repeated key must not write a second file: {after:?}"
    );
    assert!(
        after.contains(&paths[0]),
        "the replay must point at the file the first call wrote: {after:?}"
    );

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&paths[0]);
}

#[tokio::test]
async fn concurrent_backups_with_one_key_write_one_backup() {
    // The case the header exists for: the retry arrives while the first attempt
    // is still running, which is exactly what a client does once its deadline
    // fired. The second request must wait for the in-flight write and replay it
    // rather than start a competing `VACUUM INTO`.
    let (app, path) = persistent_app().await;
    let before = list_backups(&app).await;
    let request = || {
        authorized("/persistence/backup")
            .method(Method::POST)
            .header("idempotency-key", "concurrent")
            .body(Body::empty())
            .unwrap()
    };
    let (first, second) = tokio::join!(
        app.clone().oneshot(request()),
        app.clone().oneshot(request())
    );
    let first = json_body(first.unwrap()).await;
    let second = json_body(second.unwrap()).await;
    assert_eq!(
        first["path"], second["path"],
        "the in-flight attempt must be replayed, not raced"
    );

    let after = list_backups(&app).await;
    assert_eq!(
        after.len(),
        before.len() + 1,
        "two concurrent requests with one key must write one file: {after:?}"
    );
    assert!(
        after
            .iter()
            .any(|entry| entry == first["path"].as_str().unwrap()),
        "the replay must point at the file the first call wrote: {after:?}"
    );

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(first["path"].as_str().unwrap());
}

#[tokio::test]
async fn create_backup_without_a_key_writes_a_new_backup_each_time() {
    // The header is what buys the replay guarantee, so an operator asking twice
    // on purpose must still get two backups.
    let (app, path) = persistent_app().await;
    let mut paths = Vec::new();
    for _ in 0..2 {
        let response = app
            .clone()
            .oneshot(
                authorized("/persistence/backup")
                    .method(Method::POST)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        paths.push(
            json_body(response).await["path"]
                .as_str()
                .unwrap()
                .to_owned(),
        );
    }
    assert_ne!(
        paths[0], paths[1],
        "calls without a key must not be replayed"
    );

    let _ = std::fs::remove_file(&path);
    for entry in paths {
        let _ = std::fs::remove_file(entry);
    }
}

#[tokio::test]
async fn create_backup_rejects_an_unusable_idempotency_key() {
    // A caller that sends the header is relying on the retry guarantee, so a key
    // the server cannot honour is refused instead of silently ignored: a
    // silently ignored key leaves the caller believing a retry was safe.
    let (app, path) = persistent_app().await;
    let too_long = "x".repeat(MAX_IDEMPOTENCY_KEY_BYTES + 1);
    for key in ["", "with space", too_long.as_str()] {
        let response = app
            .clone()
            .oneshot(
                authorized("/persistence/backup")
                    .method(Method::POST)
                    .header("idempotency-key", key)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "key {key:?} must be rejected"
        );
        assert_eq!(
            json_body(response).await["error"]["code"],
            "invalid_request",
            "key {key:?} must report a malformed request"
        );
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn backup_idempotency_bounds_the_keys_it_remembers() {
    // A client that sends a fresh key per request must not be able to grow the
    // map without bound.
    let attempts = BackupIdempotency::default();
    for index in 0..(BACKUP_IDEMPOTENCY_CAPACITY + 16) {
        let _ = attempts.attempt(&format!("key-{index}"));
    }
    assert!(
        attempts.attempts.lock().unwrap().len() <= BACKUP_IDEMPOTENCY_CAPACITY,
        "remembered keys must stay bounded"
    );
}

#[tokio::test]
async fn backup_idempotency_keeps_a_key_whose_backup_is_still_running() {
    // An entry past its window must still survive while its write is in flight:
    // evicting it would let a retry start a second backup for a key that is
    // already being written.
    let attempts = BackupIdempotency::default();
    let running = attempts.attempt("in-flight");
    let guard = running.lock().await;
    attempts
        .attempts
        .lock()
        .unwrap()
        .get_mut("in-flight")
        .unwrap()
        .expires = Instant::now();

    for index in 0..(BACKUP_IDEMPOTENCY_CAPACITY + 1) {
        let _ = attempts.attempt(&format!("other-{index}"));
    }

    assert!(
        Arc::ptr_eq(&running, &attempts.attempt("in-flight")),
        "an in-flight key must survive eviction"
    );
    drop(guard);
}

#[tokio::test]
async fn circuits_reports_snapshots_and_reset_closes_them() {
    let app = app().await;
    let response = app
        .clone()
        .oneshot(authorized("/circuits").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert!(body["circuits"].is_array());

    let response = app
        .oneshot(
            authorized("/circuits/reset")
                .method(Method::POST)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["circuits"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn circuits_read_and_reset_both_require_a_scope_the_search_only_client_lacks() {
    let scoped = scoped_app().await;
    let read = scoped
        .clone()
        .oneshot(
            as_client("/circuits", SEARCH_ONLY_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(read.status(), StatusCode::FORBIDDEN);

    let reset = scoped
        .oneshot(
            as_client("/circuits/reset", SEARCH_ONLY_TOKEN)
                .method(Method::POST)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reset.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn update_telemetry_changes_only_the_requested_field_and_applies_without_restart() {
    let base = "schema_version = \"1\"\n\n[telemetry]\nretention_days = 30\n";
    let (app, path) = reloadable_app(base).await;

    let response = app
        .clone()
        .oneshot(
            authorized("/telemetry")
                .method(Method::PATCH)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"retention_days": 45}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let status = json_body(
        app.oneshot(authorized("/telemetry").body(Body::empty()).unwrap())
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status["telemetry"]["retention_days"], 45);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn update_telemetry_requires_the_admin_scope() {
    let scoped = scoped_app().await;
    let response = scoped
        .oneshot(
            as_client("/telemetry", SEARCH_ONLY_TOKEN)
                .method(Method::PATCH)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"retention_days": 45}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

// ── Deep advanced: top-level limits, extractor, renderer ───────────────

#[tokio::test]
async fn deep_limits_does_not_collide_with_the_domain_deep_fetch_route() {
    // `/deep` (GET/POST) stays the domain surface; config lives at
    // `/deep/limits` precisely so the two never share a path.
    let app = app().await;
    let domain = app
        .clone()
        .oneshot(
            authorized("/deep?url=https://example.com")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(domain.status(), StatusCode::NOT_FOUND);

    let config = app
        .oneshot(authorized("/deep/limits").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(config.status(), StatusCode::OK);
    let body = json_body(config).await;
    assert!(body["deep"]["max_fetches"].is_number());
    assert!(body["deep"]["extractor"].is_object());
    assert!(body["deep"]["renderer"].is_object());
}

#[tokio::test]
async fn update_deep_limits_changes_only_top_level_fields_and_applies_without_restart() {
    let base = "schema_version = \"1\"\n\n\
                # A comment an operator wrote about something unrelated.\n\
                [deep]\nmax_fetches = 10\n\n\
                [deep.extractor]\nexecutable = \"trafilatura\"\n";
    let (app, path) = reloadable_app(base).await;

    let response = app
        .clone()
        .oneshot(
            authorized("/deep/limits")
                .method(Method::PATCH)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"max_fetches": 20}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains("# A comment an operator wrote about something unrelated."));
    assert!(on_disk.contains("max_fetches = 20"));
    assert!(on_disk.contains("executable = \"trafilatura\""));

    let updated = json_body(
        app.oneshot(authorized("/deep/limits").body(Body::empty()).unwrap())
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(updated["deep"]["max_fetches"], 20);
    assert_eq!(updated["deep"]["extractor"]["executable"], "trafilatura");
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn update_deep_limits_requires_the_admin_scope() {
    let scoped = scoped_app().await;
    let response = scoped
        .oneshot(
            as_client("/deep/limits", SEARCH_ONLY_TOKEN)
                .method(Method::PATCH)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"max_fetches": 20}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn update_deep_extractor_writes_into_its_own_nested_table() {
    let (app, path) = reloadable_app("schema_version = \"1\"\n").await;
    let response = app
        .oneshot(
            authorized("/deep/extractor")
                .method(Method::PATCH)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"timeout_ms": 12000}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let reparsed = amatl_core::Config::load_optional(&path).unwrap();
    assert_eq!(reparsed.deep.extractor.timeout_ms, 12_000);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn update_deep_renderer_refuses_enabling_under_the_isolated_profile() {
    let base = "schema_version = \"1\"\n\n\
                [data_policy]\nprofile = \"isolated\"\negress = \"deny\"\n";
    let (app, path) = reloadable_app(base).await;
    let response = app
        .oneshot(
            authorized("/deep/renderer")
                .method(Method::PATCH)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"enabled": true}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(!on_disk.contains("[deep.renderer]"));
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn update_deep_extractor_and_renderer_require_the_admin_scope() {
    let scoped = scoped_app().await;
    for path in ["/deep/extractor", "/deep/renderer"] {
        let response = scoped
            .clone()
            .oneshot(
                as_client(path, SEARCH_ONLY_TOKEN)
                    .method(Method::PATCH)
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(json!({}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path}");
    }
}

// ── Server "cold" fields: bind/port/TLS/allowed_hosts/limits ──────────

#[tokio::test]
async fn server_pending_config_is_read_scoped_and_omits_clients() {
    let scoped = scoped_app().await;
    let denied = scoped
        .clone()
        .oneshot(
            as_client("/server/pending-config", SEARCH_ONLY_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);

    let response = scoped
        .oneshot(
            as_client("/server/pending-config", ADMIN_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert!(body["server"]["bind"].is_string());
    assert!(body["server"]["clients"].is_null());
}

#[tokio::test]
async fn update_server_pending_config_writes_but_never_applies_and_classifies_cold_fields() {
    let base = "schema_version = \"1\"\n\n\
                # A comment an operator wrote about something unrelated.\n\
                [server]\nport = 8080\n";
    let (app, path) = reloadable_app(base).await;

    let response = app
        .clone()
        .oneshot(
            authorized("/server/pending-config")
                .method(Method::PATCH)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"port": 9090}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["applied"], false);
    assert_eq!(body["requires_restart"], true);
    assert_eq!(body["cold_fields"], json!(["server.port"]));
    assert_eq!(body["hot_fields"], json!([]));

    // Written to disk...
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains("# A comment an operator wrote about something unrelated."));
    assert!(on_disk.contains("port = 9090"));

    // ...but a live request to the running listener is unaffected: the old
    // bind/port are still what's actually serving, exactly as promised.
    let status = app
        .oneshot(authorized("/status").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(status.status(), StatusCode::OK);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn update_server_pending_config_classifies_a_hot_field_without_requiring_restart() {
    let (app, path) = reloadable_app("schema_version = \"1\"\n").await;

    let response = app
        .oneshot(
            authorized("/server/pending-config")
                .method(Method::PATCH)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"rate_limit_per_minute": 30}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["applied"], false);
    assert_eq!(body["requires_restart"], false);
    assert_eq!(body["hot_fields"], json!(["server.rate_limit_per_minute"]));
    assert_eq!(body["cold_fields"], json!([]));
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn update_server_pending_config_a_hot_field_only_applies_after_a_manual_reload() {
    // `request()`/`authorized()` send `Host: localhost:8080`, which the
    // default `allowed_hosts` accepts. Narrowing it to a host that excludes
    // that should keep working until an explicit `/reload`, then start
    // rejecting — proving the write alone changed nothing live.
    let (app, path) = reloadable_app("schema_version = \"1\"\n").await;

    let patched = app
        .clone()
        .oneshot(
            authorized("/server/pending-config")
                .method(Method::PATCH)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"allowed_hosts": ["example.internal"]}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(patched.status(), StatusCode::OK);

    // Still live on the old allowlist: the write did not reload anything.
    let before_reload = app
        .clone()
        .oneshot(authorized("/status").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(before_reload.status(), StatusCode::OK);

    let reloaded = app
        .clone()
        .oneshot(
            authorized("/reload")
                .method(Method::POST)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reloaded.status(), StatusCode::OK);

    // Now that the operator explicitly reloaded, the narrowed allowlist is
    // in force and `localhost` is no longer on it.
    let after_reload = app
        .oneshot(authorized("/status").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(after_reload.status(), StatusCode::BAD_REQUEST);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn update_server_pending_config_requires_the_admin_scope() {
    let scoped = scoped_app().await;
    let response = scoped
        .oneshot(
            as_client("/server/pending-config", SEARCH_ONLY_TOKEN)
                .method(Method::PATCH)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"port": 9090}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn update_server_pending_config_refuses_an_invalid_candidate_without_writing() {
    // Remote (non-loopback) bind without TLS or auth is refused by
    // Config::validate.
    let (app, path) = reloadable_app("schema_version = \"1\"\n").await;
    let response = app
        .oneshot(
            authorized("/server/pending-config")
                .method(Method::PATCH)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"bind": "0.0.0.0"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(!on_disk.contains("0.0.0.0"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn openapi_covers_every_router_operation() {
    // AUDIT-01: keep the public OpenAPI inventory synchronized with the Axum
    // router. The endpoint-specific request/response contracts are documented
    // in the same file; this guard specifically prevents a route or method
    // from being omitted altogether.
    use crate::routes::all_routes;

    // The contract ships inside the crate, next to the code that serves it:
    // Cargo refuses a path that escapes the package, so an `include_str!` of
    // `docs/` compiles in a clone and breaks `cargo test` on the extracted
    // tarball (C1 in docs/security/hallazgos-2026-10-26.md).
    // Strip CR so the "\n"-anchored matches below hold on a Windows checkout
    // that rewrote the fixture's line endings (see .gitattributes).
    let openapi = include_str!("../openapi.yaml").replace('\r', "");
    let openapi = openapi.as_str();
    let routes = all_routes();

    for route in routes {
        let (_, section) = openapi
            .split_once(&format!("  {}:\n", route.path))
            .unwrap_or_else(|| panic!("OpenAPI omite la ruta {}", route.path));
        let section = section.split("\n  /").next().unwrap();

        for method in route.methods {
            let method_str = format!("{:?}", method).to_lowercase();
            assert!(
                section.contains(&format!("    {}:", method_str)),
                "OpenAPI omite {} {}",
                method_str,
                route.path
            );
        }
    }
}

#[test]
fn openapi_error_catalog_matches_shared_catalog() {
    // The `ErrorResponse.code` enum is the client-visible half of the shared
    // catalog in `amatl-core/src/errors.rs`. A code the server can emit but
    // the contract omits makes a strictly-validating client reject a
    // legitimate response, so both lists are asserted equal in each direction
    // rather than merely "documented somewhere".
    use amatl_core::ERROR_CATALOG;

    // Strip CR so the "\n"-anchored matches below hold on a Windows checkout
    // that rewrote the fixture's line endings (see .gitattributes).
    let openapi = include_str!("../openapi.yaml").replace('\r', "");
    let (_, after) = openapi
        .split_once("    ErrorResponse:\n")
        .expect("OpenAPI declares the ErrorResponse schema");
    // Bound the search to this schema so an enum added to a later one cannot
    // be picked up by mistake. A sibling schema starts at exactly four spaces
    // of indentation, so stop at the first line that is indented by exactly
    // four (deeper lines still belong to ErrorResponse).
    let section: String = after
        .split_inclusive('\n')
        .take_while(|line| !line.starts_with("    ") || line.starts_with("     "))
        .collect();
    let enum_line = section
        .lines()
        .find(|line| line.trim_start().starts_with("enum: ["))
        .expect("ErrorResponse.code declares an enum");
    let documented: Vec<&str> = enum_line
        .trim()
        .trim_start_matches("enum: [")
        .trim_end_matches(']')
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect();

    for code in ERROR_CATALOG {
        assert!(
            documented.contains(&code.as_str()),
            "OpenAPI ErrorResponse omits catalog code {}",
            code.as_str()
        );
    }
    for code in &documented {
        assert!(
            ERROR_CATALOG.iter().any(|entry| entry.as_str() == *code),
            "OpenAPI ErrorResponse documents unknown code {code}"
        );
    }
}

/// One operation documented in `crates/amatl-server/openapi.yaml`.
///
/// Every contract guard below needs the same three facts about an operation —
/// the status codes it documents, whether it is public (`security: []`) and
/// whether it carries a request body — so they share one parser instead of
/// each re-slicing the YAML with its own assumptions.
struct DocumentedOperation {
    path: String,
    method: String,
    statuses: Vec<String>,
    public: bool,
    has_request_body: bool,
}

impl DocumentedOperation {
    fn documents(&self, status: &str) -> bool {
        self.statuses.iter().any(|documented| documented == status)
    }

    fn label(&self) -> String {
        format!("{} {}", self.method.to_uppercase(), self.path)
    }
}

/// Collect the quoted three-digit keys of one line.
///
/// The contract writes response keys in both the block form (`'200':`) and the
/// compact form (`responses: { '200': ..., '404': ... }`). No description,
/// `$ref` or other value in the file contains a quoted three-digit string, so
/// scanning for quoted triplets is exact rather than a heuristic.
fn status_codes_in(line: &str) -> Vec<String> {
    let mut codes = Vec::new();
    let mut rest = line;
    while let Some(open) = rest.find('\'') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('\'') else { break };
        let candidate = &after[..close];
        if candidate.len() == 3 && candidate.chars().all(|c| c.is_ascii_digit()) {
            codes.push(candidate.to_owned());
        }
        rest = &after[close + 1..];
    }
    codes
}

/// Parse every operation documented under `paths:` in the checked-in contract.
fn documented_operations() -> Vec<DocumentedOperation> {
    // Strip CR so the indentation-anchored matches below hold on a Windows
    // checkout that rewrote the fixture's line endings (see .gitattributes).
    let openapi = include_str!("../openapi.yaml").replace('\r', "");
    const METHODS: [&str; 5] = ["get", "post", "patch", "delete", "put"];

    let mut operations: Vec<DocumentedOperation> = Vec::new();
    let mut in_paths = false;
    let mut path = String::new();
    let mut in_responses = false;

    for line in openapi.lines() {
        // Only `paths:` holds operations; `components:` reuses the same
        // indentation for schemas, so bound the scan explicitly.
        if line == "paths:" {
            in_paths = true;
            continue;
        }
        if !line.starts_with(' ') {
            in_paths = false;
            continue;
        }
        if !in_paths {
            continue;
        }
        // A path key is indented by exactly two spaces.
        if let Some(rest) = line.strip_prefix("  /") {
            if line.ends_with(':') {
                path = format!("/{}", rest.trim_end_matches(':'));
                continue;
            }
        }
        // A method key is indented by exactly four spaces.
        if let Some(rest) = line.strip_prefix("    ") {
            if !rest.starts_with(' ') {
                let method = rest.trim_end_matches(':');
                if METHODS.contains(&method) {
                    operations.push(DocumentedOperation {
                        path: path.clone(),
                        method: method.to_owned(),
                        statuses: Vec::new(),
                        public: false,
                        has_request_body: false,
                    });
                    in_responses = false;
                    continue;
                }
            }
        }
        let Some(operation) = operations.last_mut() else {
            continue;
        };
        let trimmed = line.trim_start();
        if trimmed.starts_with("security:") {
            operation.public = trimmed.contains("[]");
        }
        if trimmed.starts_with("requestBody:") {
            operation.has_request_body = true;
        }
        if trimmed.starts_with("responses:") {
            in_responses = true;
        } else if in_responses && line.starts_with("      ") && !line.starts_with("       ") {
            // A key at exactly six spaces is a sibling of `responses:`, so the
            // block ended. Deeper lines are still part of a response.
            in_responses = false;
        }
        if in_responses {
            operation.statuses.extend(status_codes_in(line));
        }
    }
    operations
}

#[test]
fn openapi_operations_match_router_inventory() {
    // Every contract guard below reads the shared parser, so this one proves the
    // parser sees exactly the router's operations. Without it, a parser that
    // silently dropped an operation would make each of those assertions
    // vacuous.
    use crate::routes::all_routes;

    let mut documented: Vec<String> = documented_operations()
        .iter()
        .map(|operation| format!("{} {}", operation.method, operation.path))
        .collect();
    let mut routed: Vec<String> = all_routes()
        .iter()
        .flat_map(|route| {
            route
                .methods
                .iter()
                .map(|method| format!("{} {}", format!("{method:?}").to_lowercase(), route.path))
        })
        .collect();
    documented.sort();
    routed.sort();
    assert_eq!(
        documented, routed,
        "el inventario OpenAPI diverge del router de Axum"
    );
}

#[test]
fn openapi_documents_transport_level_rejections() {
    // `security_middleware` (lib.rs) wraps every route in the router, so these
    // rejections are reachable on any operation and not only on the domain
    // ones. Each status is traceable to an emitter and to the catalog entry
    // that maps it:
    //   400 invalid_host        `valid_host`
    //   403 invalid_origin      `valid_origin`; scope_denied on protected routes
    //   429 rate_limited        the per-client window
    //   431 headers_too_large   the header budget
    //   504 request_timeout     the request deadline
    //   401 unauthorized        missing or expired credential (protected only)
    //   413 body_too_large      the Content-Length pre-check, then DefaultBodyLimit
    // A client generated from a contract that omits them cannot recover from an
    // expired token or a throttled admin call, which is the drift this guard
    // exists to prevent.
    const UNIVERSAL: [&str; 5] = ["400", "403", "429", "431", "504"];

    for operation in documented_operations() {
        for status in UNIVERSAL {
            assert!(
                operation.documents(status),
                "OpenAPI omite {status} en {}",
                operation.label()
            );
        }
        if !operation.public {
            assert!(
                operation.documents("401"),
                "OpenAPI omite 401 en la ruta protegida {}",
                operation.label()
            );
        }
        if operation.has_request_body {
            assert!(
                operation.documents("413"),
                "OpenAPI omite 413 en {} aunque declara requestBody",
                operation.label()
            );
        }
    }
}

#[test]
fn openapi_component_refs_resolve() {
    // A dangling `$ref` is invisible to every other guard and to a reader, yet
    // it makes the document unrenderable for tooling. Both directions matter:
    // every `#/components/...` target must exist, and every declared response
    // or parameter must be referenced by at least one operation, because a
    // component nothing points at is contract text that no longer describes the
    // server. Security schemes are exempt: the top-level `security` block
    // references them by bare name, not by `$ref`.
    let openapi = include_str!("../openapi.yaml").replace('\r', "");
    let openapi = openapi.as_str();

    let mut sections: Vec<(String, Vec<String>)> = Vec::new();
    let mut in_components = false;
    for line in openapi.lines() {
        if line == "components:" {
            in_components = true;
            continue;
        }
        if !line.starts_with(' ') {
            in_components = false;
            continue;
        }
        if !in_components {
            continue;
        }
        // A section key is indented by exactly two spaces, its entries by four.
        for (indent, is_entry) in [("  ", false), ("    ", true)] {
            if let Some(rest) = line.strip_prefix(indent) {
                if rest.starts_with(' ') {
                    continue;
                }
                let Some(name) = rest.strip_suffix(':') else {
                    continue;
                };
                if is_entry {
                    if let Some((_, names)) = sections.last_mut() {
                        names.push(name.to_owned());
                    }
                } else {
                    sections.push((name.to_owned(), Vec::new()));
                }
            }
        }
    }
    assert!(
        !sections.is_empty(),
        "no se pudo leer `components:` de crates/amatl-server/openapi.yaml"
    );

    let mut referenced: Vec<(String, String)> = Vec::new();
    let mut rest = openapi;
    while let Some(index) = rest.find("#/components/") {
        let tail = &rest[index + "#/components/".len()..];
        let end = tail
            .find(|character: char| {
                !(character.is_ascii_alphanumeric() || character == '_' || character == '/')
            })
            .unwrap_or(tail.len());
        if let Some((section, name)) = tail[..end].split_once('/') {
            referenced.push((section.to_owned(), name.to_owned()));
        }
        // `end` is zero only if the target is empty, in which case stepping one
        // byte keeps the scan moving instead of looping forever.
        rest = &tail[end.max(1)..];
    }
    assert!(
        !referenced.is_empty(),
        "crates/amatl-server/openapi.yaml no referencia ningún componente"
    );

    for (section, name) in &referenced {
        assert!(
            sections.iter().any(|(declared, names)| {
                declared == section && names.iter().any(|candidate| candidate == name)
            }),
            "OpenAPI tiene un $ref roto: #/components/{section}/{name}"
        );
    }
    for (section, names) in &sections {
        if !matches!(section.as_str(), "responses" | "parameters") {
            continue;
        }
        for name in names {
            assert!(
                referenced
                    .iter()
                    .any(|(target, entry)| target == section && entry == name),
                "OpenAPI declara {section}/{name} sin referenciarlo desde ninguna operación"
            );
        }
    }
}

#[test]
fn server_error_io_preserves_the_source() {
    let source = std::io::Error::new(std::io::ErrorKind::AddrInUse, "address in use");
    let wrapped = ServerError::Io(source);
    let message = wrapped.to_string();
    assert!(message.contains("address in use"), "message was: {message}");
    assert!(std::error::Error::source(&wrapped).is_some());
}

#[test]
fn server_error_tls_preserves_the_source() {
    let source = std::io::Error::new(std::io::ErrorKind::InvalidData, "bad pem");
    let wrapped = ServerError::Tls(source);
    let message = wrapped.to_string();
    assert!(message.contains("bad pem"), "message was: {message}");
    assert!(std::error::Error::source(&wrapped).is_some());
}

#[test]
fn server_error_configuration_preserves_the_source() {
    let source = std::io::Error::new(std::io::ErrorKind::InvalidInput, "bad config");
    let wrapped = ServerError::Configuration(Box::new(source));
    let message = wrapped.to_string();
    assert!(message.contains("bad config"), "message was: {message}");
    assert!(std::error::Error::source(&wrapped).is_some());
}

/// Readiness counts *available* sources, not usable ones, and the difference is
/// observable: a single-source deployment whose only source is `available` but
/// has its circuit open answers `503` with `sources_available` still at `1`.
/// `docs/operacion.md` states the rule for operators, and this test pins it so a
/// later "cleanup" of the count cannot silently change what the field means.
///
/// The helpers stay next to the only test that uses them — and at the end of the
/// file, like `FailingFactory` in `amatl-core` — so the line ranges cited by
/// `docs/security/threat-model.md` and `docs/security/asvs-checklist.md` keep
/// resolving to the tests they name.
#[tokio::test]
async fn ready_reports_degraded_while_counting_an_available_source_with_an_open_circuit() {
    let router = failing_archive_app().await;
    // The same deployment is ready before the first failure, so the assertions
    // below isolate the circuit breaker and nothing else.
    let before = router
        .clone()
        .oneshot(request("/ready").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(before.status(), StatusCode::OK);
    let before_body = json_body(before).await;
    assert_eq!(before_body["sources_available"], 1);
    // With the breaker closed the source is both advertised and usable.
    assert_eq!(before_body["sources_usable"], 1);
    let search = router
        .clone()
        .oneshot(authorized("/search?q=rust").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(search.status(), StatusCode::OK);
    assert_eq!(
        json_body(search).await["providers_failed"],
        json!(["custom_archive"])
    );

    let after = router
        .oneshot(request("/ready").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(after.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = json_body(after).await;
    assert_eq!(body["status"], "degraded");
    // The count still reports the surface status `/status` shows for the source,
    // so it is deliberately *not* the readiness predicate: gate on the status
    // code, or on `status`, never on this field.
    assert_eq!(body["sources_available"], 1);
    // `sources_usable` folds the open breaker in: the source is advertised but
    // cannot serve right now, which is the count an automation should gate on.
    assert_eq!(body["sources_usable"], 0);
}

/// A deployment with exactly one enabled source, governed, and always failing.
/// It is registered under a name no shipped factory claims, so the search never
/// leaves the process — it only has to trip the circuit breaker.
async fn failing_archive_app() -> Router {
    let mut config = amatl_core::Config::default();
    config
        .providers
        .declare("custom_archive", approved_archive_record());
    config.providers.enabled = vec!["custom_archive".into()];
    config.circuit_breaker.failure_threshold = 1;
    config.circuit_breaker.open_seconds = 600;
    config.validate().unwrap();
    build_router(
        AmatlService::with_registry(
            config,
            false,
            amatl_core::ProviderRegistry::builtin().with(Arc::new(FailingFactory)),
        )
        .await,
        Some(TOKEN.into()),
    )
    .await
    .unwrap()
}

/// Governance record that satisfies the runtime approval gate today.
fn approved_archive_record() -> amatl_core::ProviderRuntimeConfig {
    amatl_core::ProviderRuntimeConfig {
        adapter_version: Some("archive-v1".into()),
        approval_status: amatl_core::ApprovalStatus::Approved,
        reviewed_at: Some(today_iso()),
        reviewer: Some("owner".into()),
        terms_url: Some("https://archive.invalid/terms".into()),
        terms_version_or_date: Some("2026-08-01".into()),
        allowed_access_method: Some("official_api".into()),
        plan_or_contract: Some("contract-1".into()),
        rate_limit: Some("1 qps".into()),
        cost_model: Some("free".into()),
        data_handling_notes: Some("no cache".into()),
        operational_risk: Some("none".into()),
        ..amatl_core::ProviderRuntimeConfig::default()
    }
}

/// Same name as the record above, but every search fails.
struct FailingFactory;

impl amatl_core::ProviderFactory for FailingFactory {
    fn name(&self) -> &str {
        "custom_archive"
    }

    fn requires_credential(&self) -> bool {
        false
    }

    fn build(
        &self,
        context: &amatl_core::ProviderBuildContext<'_>,
    ) -> Arc<dyn amatl_core::Provider> {
        Arc::new(amatl_core::MockProvider::new(
            context.name,
            amatl_core::MockBehavior::Failure(amatl_core::ProviderErrorKind::Unavailable),
        ))
    }
}

/// `amatl_sources_available` and `/ready`'s `sources_available` are one quantity
/// published on two surfaces, and the relation to `amatl_sources_declared` is
/// what an alert has to get right: the *declared* gauge counts every source the
/// deployment declares — including the ones the operator left disabled, which
/// report `Unavailable` with `provider_disabled` — while the *available* gauge
/// counts only those whose surface status is `available`.
///
/// So `declared - available` is not an incident count. On a single-source
/// deployment it is one by configuration, and alerting on the difference pages
/// the operator for a decision they made on purpose. `docs/operacion.md` states
/// the relation for operators; this test pins both numbers against one
/// deployment so neither gauge can drift into the other's meaning.
#[tokio::test]
async fn source_gauges_count_declared_sources_while_readiness_counts_available_ones() {
    let router = marginalia_only_app().await;
    let ready = router
        .clone()
        .oneshot(request("/ready").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(ready.status(), StatusCode::OK);
    assert_eq!(json_body(ready).await["sources_available"], 1);

    let response = router
        .oneshot(request("/metrics").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = String::from_utf8(
        to_bytes(response.into_body(), 128 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    let gauge = |name: &str| -> f64 {
        body.lines()
            .find_map(|line| line.strip_prefix(&format!("{name} ")))
            .and_then(|value| value.trim().parse::<f64>().ok())
            .unwrap_or_else(|| panic!("missing {name} sample in: {body}"))
    };

    // One source declared (Marginalia), enabled and available: there is no
    // disabled second source, so declared equals available and the difference is
    // zero by configuration.
    assert_eq!(gauge("amatl_sources_declared"), 1.0);
    assert_eq!(gauge("amatl_sources_available"), 1.0);
    assert_eq!(gauge("amatl_sources_circuit_open"), 0.0);
}

/// `--mock` is a deployment the CLI ships, and it is the one place where the two
/// surfaces of the source count disagree: `/ready` reports the mock sources the
/// service actually searches with, while `/metrics` keeps reporting the declared
/// ones. Measured on the isolated mock deployment below: `/ready` answers `200`
/// with `"sources_available":2` and `/search` answers `200` — the deployment
/// serves — while `/metrics` publishes `amatl_sources_declared 1` and
/// `amatl_sources_available 0`, because the real source is declared but cannot
/// serve under `egress = "deny"`.
///
/// The divergence has the shape D2 removed from `/status` — a surface announcing
/// that nothing can serve while `/search` succeeds — so it is pinned rather than
/// fixed unilaterally: making `/metrics` mock-aware changes the value of a
/// published metric, which needs an owner decision and its own consumer review.
/// `docs/operacion.md` documents the case for operators.
#[tokio::test]
async fn mock_deployments_disagree_between_ready_and_the_source_gauges() {
    let router = isolated_app().await;
    let ready = router
        .clone()
        .oneshot(request("/ready").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(ready.status(), StatusCode::OK);
    assert_eq!(json_body(ready).await["sources_available"], 2);

    // The deployment really serves: the mock sources answer, which is what makes
    // the zero published below a trap rather than a harmless default.
    let search = router
        .clone()
        .oneshot(
            authorized("/search")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"q":"rust"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(search.status(), StatusCode::OK);

    let response = router
        .oneshot(request("/metrics").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let body = String::from_utf8(
        to_bytes(response.into_body(), 512 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    let gauge = |name: &str| -> f64 {
        body.lines()
            .find_map(|line| line.strip_prefix(&format!("{name} ")))
            .and_then(|value| value.trim().parse::<f64>().ok())
            .unwrap_or_else(|| panic!("missing {name} sample in: {body}"))
    };

    assert_eq!(gauge("amatl_sources_declared"), 1.0);
    assert_eq!(gauge("amatl_sources_available"), 0.0);
}
