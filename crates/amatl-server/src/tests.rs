use super::*;
use axum::{body::to_bytes, http::Request};
use std::io::Write;
use std::sync::{Arc, Mutex, Once, OnceLock};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

const TOKEN: &str = "0123456789abcdef0123456789abcdef";

async fn app() -> Router {
    build_router(
        AmatlService::new(amatl_core::Config::default(), true).await,
        Some(TOKEN.into()),
    )
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

#[tokio::test]
async fn health_is_lightweight_public_and_hardened() {
    let response = app()
        .await
        .oneshot(request("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().contains_key("content-security-policy"));
    assert_eq!(
        response.headers()["x-content-type-options"],
        HeaderValue::from_static("nosniff")
    );
    assert!(!response.headers().contains_key("server"));
    let body = json_body(response).await;
    assert_eq!(body, json!({"schema_version": "1", "status": "ok"}));
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

fn security_logs() -> Arc<Mutex<Vec<u8>>> {
    static CAPTURED: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();
    static INSTALL: Once = Once::new();
    let captured = CAPTURED
        .get_or_init(|| Arc::new(Mutex::new(Vec::new())))
        .clone();
    INSTALL.call_once(|| {
        let sink = captured.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_target(true)
            .with_max_level(tracing::Level::TRACE)
            .with_writer(move || LogWriter(sink.clone()))
            .finish();
        tracing::subscriber::set_global_default(subscriber)
            .expect("test security log subscriber should install once");
    });
    captured
}

#[tokio::test]
async fn rejected_requests_emit_secret_safe_security_events() {
    let captured = security_logs();
    captured
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clear();
    let supplied_secret = "never-log-this-invalid-token";
    let response = app()
        .await
        .oneshot(
            request("/search?q=rust%0Aforged")
                .header(AUTHORIZATION, format!("Bearer {supplied_secret}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
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
    assert!(!logs.contains(supplied_secret), "{logs}");
    assert!(!logs.contains("forged"), "{logs}");
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
async fn mcp_uses_streamable_http_and_exposes_exactly_four_tools() {
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
        ["deep_search", "fetch", "providers", "search"].into()
    );

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
