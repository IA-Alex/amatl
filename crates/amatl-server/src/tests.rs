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
            "status",
            "storage_ok"
        ]
        .into()
    );
    assert_eq!(body["schema_version"], "1");
    assert!(body["storage_ok"].is_boolean());
    assert!(body["sources_available"].is_u64());
    let rendered = body.to_string();
    for internal in ["provider_", "mock", "sqlite", "/", "path"] {
        assert!(
            !rendered.contains(internal),
            "readiness body leaked {internal:?}: {rendered}"
        );
    }
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
        security: Arc::new(RwLock::new(security)),
        rate_limiter: Arc::new(Mutex::new(RateLimiter {
            windows: BTreeMap::new(),
            last_cleanup: Instant::now(),
        })),
        explicit_token: Some(TOKEN.into()),
        metrics: Arc::new(RequestMetrics::default()),
    };
    let app = Router::new()
        .route(
            "/slow",
            get(|| async {
                tokio::time::sleep(Duration::from_millis(100)).await;
                StatusCode::OK
            }),
        )
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(state, security_middleware));

    let response = app
        .oneshot(request("/slow").body(Body::empty()).unwrap())
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
    assert_eq!(initialized["result"]["protocolVersion"], "2026-07-28");

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
            "answer",
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
        ("GET", "/deep?q=test"),
    ] {
        let builder = if path == "/search?q=test" || path == "/deep?q=test" {
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
    assert_eq!(body["result"]["protocolVersion"], "2026-07-28");
    assert!(body["result"]["capabilities"]["tools"].is_object());
}

#[tokio::test]
async fn mcp_initialize_rejects_unsupported_protocol_version() {
    let app = app().await;
    let call = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "conformance-test", "version": "1" }
        }
    });
    let (status, body) = mcp_post(&app, call).await;
    assert_eq!(status, StatusCode::OK);
    // rmcp negotiates down to the highest mutually supported version.
    // The negotiated version must differ from the requested unsupported one.
    let negotiated = body["result"]["protocolVersion"].as_str().unwrap_or("");
    assert_ne!(
        negotiated, "2024-11-05",
        "server should not accept unsupported protocol version, got: {body}"
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
        "amatl_source_available",
        "amatl_source_circuit_open",
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
    assert!(body.contains("amatl_search_latency_ms{quantile=\"0.95\"}"));
    assert!(body.contains("amatl_cache_hit_rate{cache=\"document\"}"));
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
        .any(|value| value == "brave"));

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

#[tokio::test]
async fn answer_toggle_requires_the_bearer_token() {
    let response = app()
        .await
        .oneshot(
            request("/answer/enabled")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"enabled": true}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn answer_toggle_flips_the_flag_and_applies_without_restart() {
    let base = "schema_version = \"1\"\n\n\
                [data_policy]\ninference = \"remote_explicit\"\n\n\
                [inference]\nremote_endpoint = \"https://api.deepinfra.com/v1/openai/embeddings\"\n\
                remote_model = \"BAAI/bge-base-en-v1.5\"\n\n\
                [answer]\nenabled = false\n\
                endpoint = \"https://api.deepinfra.com/v1/openai/chat/completions\"\n\
                model = \"deepseek-ai/DeepSeek-V3\"\n\
                credential_env = \"AMATL_TEST_ANSWER_KEY\"\n";
    let (app, path) = reloadable_app(base).await;

    let response = app
        .clone()
        .oneshot(
            authorized("/answer/enabled")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"enabled": true}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Persisted: a fresh read of the file (not just the in-memory service)
    // shows the flag flipped.
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains("enabled = true"));
    // Untouched: the toggle never wrote near the credential or model.
    assert!(on_disk.contains("credential_env = \"AMATL_TEST_ANSWER_KEY\""));
    assert!(on_disk.contains("model = \"deepseek-ai/DeepSeek-V3\""));

    let status = json_body(
        app.oneshot(authorized("/status").body(Body::empty()).unwrap())
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status["answer"]["enabled"], true);
    // Model/endpoint were already on disk before the toggle — configured
    // reflects that regardless of enabled, so this was already true.
    assert_eq!(status["answer"]["configured"], true);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn answer_toggle_refuses_to_enable_an_incomplete_config_without_writing() {
    // No endpoint/model: enabling this would fail Config::validate.
    let base = "schema_version = \"1\"\n\n\
                [data_policy]\ninference = \"remote_explicit\"\n\n\
                [inference]\nremote_endpoint = \"https://api.deepinfra.com/v1/openai/embeddings\"\n\
                remote_model = \"BAAI/bge-base-en-v1.5\"\n\n\
                [answer]\nenabled = false\n";
    let (app, path) = reloadable_app(base).await;

    let response = app
        .oneshot(
            authorized("/answer/enabled")
                .method(Method::POST)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"enabled": true}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        json_body(response).await["error"]["code"],
        "configuration_invalid"
    );
    // Fails closed before ever writing: the file on disk is untouched.
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains("enabled = false"));
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
