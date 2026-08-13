use super::*;
use axum::{body::to_bytes, http::Request};
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
