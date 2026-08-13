//! Hardened HTTP API, UI hosting and MCP Streamable HTTP surface for AMATL.

mod mcp;

use amatl_core::{AmatlService, ConfigError, ServiceError, ServiceSurface, SCHEMA_VERSION};
use amatl_ui::{asset, security_headers};
use axum::{
    body::Body,
    extract::{
        rejection::{JsonRejection, QueryRejection},
        ConnectInfo, DefaultBodyLimit, Query, Request, State,
    },
    http::{
        header::{
            AUTHORIZATION, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE, HOST, ORIGIN,
            WWW_AUTHENTICATE,
        },
        HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri,
    },
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::BTreeMap,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use thiserror::Error;
use tower::limit::ConcurrencyLimitLayer;
use tower_http::cors::CorsLayer;

#[derive(Clone)]
struct AppState {
    service: AmatlService,
    security: Arc<SecurityState>,
}

struct SecurityState {
    token: Option<String>,
    allowed_hosts: Vec<String>,
    allowed_origins: Vec<String>,
    max_header_bytes: usize,
    max_body_bytes: usize,
    timeout: Duration,
    rate_limit_per_minute: u32,
    rate_limiter: Mutex<RateLimiter>,
    https: bool,
}

struct RateWindow {
    started: Instant,
    count: u32,
}

struct RateLimiter {
    windows: BTreeMap<IpAddr, RateWindow>,
    last_cleanup: Instant,
}

#[derive(Debug, Error)]
pub enum ServerError {
    #[error("invalid server configuration")]
    Configuration,
    #[error("server token is missing or too short")]
    MissingToken,
    #[error("TLS configuration failed")]
    Tls,
    #[error("server failed")]
    Io,
}

impl From<ConfigError> for ServerError {
    fn from(_: ConfigError) -> Self {
        Self::Configuration
    }
}

#[derive(Debug, Deserialize)]
struct SearchParams {
    q: String,
}

#[derive(Debug, Serialize)]
struct ProviderResponse {
    schema_version: String,
    providers: Vec<amatl_core::ProviderSummary>,
}

pub async fn build_router(
    service: AmatlService,
    explicit_token: Option<String>,
) -> Result<Router, ServerError> {
    service.config().validate()?;
    let server = service.config().server.clone();
    let https = server.tls.cert_path.is_some();
    let token = if server.no_auth {
        None
    } else {
        explicit_token
            .or_else(|| std::env::var(&server.token_env).ok())
            .filter(|value| value.len() >= 32)
            .ok_or(ServerError::MissingToken)
            .map(Some)?
    };
    let allowed_origins = effective_origins(service.config(), https);
    let security = Arc::new(SecurityState {
        token,
        allowed_hosts: server.allowed_hosts.clone(),
        allowed_origins: allowed_origins.clone(),
        max_header_bytes: server.max_header_bytes,
        max_body_bytes: server.max_body_bytes,
        timeout: Duration::from_millis(server.request_timeout_ms),
        rate_limit_per_minute: server.rate_limit_per_minute,
        rate_limiter: Mutex::new(RateLimiter {
            windows: BTreeMap::new(),
            last_cleanup: Instant::now(),
        }),
        https,
    });
    let mcp_config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_sse_keep_alive(None)
        .with_allowed_hosts(server.allowed_hosts.clone())
        .with_allowed_origins(allowed_origins.clone())
        .with_max_request_body_bytes(server.max_body_bytes)
        .with_stateless_protocol_metadata_required(true);
    let mcp_service: StreamableHttpService<mcp::McpSurface, LocalSessionManager> =
        StreamableHttpService::new(
            {
                let service = service.clone();
                move || Ok(mcp::McpSurface::new(service.clone()))
            },
            Default::default(),
            mcp_config,
        );
    let state = AppState { service, security };
    let cors = cors_layer(&allowed_origins)?;
    Ok(Router::new()
        .route("/search", get(search).post(search_post))
        .route("/deep", get(deep).post(deep_post))
        .route("/providers", get(providers))
        .route("/health", get(health))
        .nest_service("/mcp", mcp_service)
        .fallback(static_asset)
        .with_state(state.clone())
        .layer(ConcurrencyLimitLayer::new(server.max_connections))
        .layer(DefaultBodyLimit::max(server.max_body_bytes))
        .layer(cors)
        .layer(middleware::from_fn_with_state(state, security_middleware)))
}

pub async fn serve(service: AmatlService) -> Result<(), ServerError> {
    let address = SocketAddr::new(
        service
            .config()
            .server
            .bind
            .parse::<IpAddr>()
            .map_err(|_| ServerError::Configuration)?,
        service.config().server.port,
    );
    let idle = Duration::from_millis(service.config().server.idle_timeout_ms);
    let tls = service.config().server.tls.clone();
    let app = build_router(service, None).await?;
    let make_service = app.into_make_service_with_connect_info::<SocketAddr>();
    match (tls.cert_path, tls.key_path) {
        (Some(cert), Some(key)) => {
            let config = axum_server::tls_rustls::RustlsConfig::from_pem_file(cert, key)
                .await
                .map_err(|_| ServerError::Tls)?;
            let mut server = axum_server::bind_rustls(address, config);
            server
                .http_builder()
                .http1()
                .timer(hyper_util::rt::TokioTimer::new())
                .keep_alive(false)
                .header_read_timeout(Some(idle));
            server
                .http_builder()
                .http2()
                .timer(hyper_util::rt::TokioTimer::new())
                .keep_alive_interval(Some(idle / 2))
                .keep_alive_timeout(idle);
            server
                .serve(make_service)
                .await
                .map_err(|_| ServerError::Io)
        }
        (None, None) => {
            let mut server = axum_server::bind(address);
            server
                .http_builder()
                .http1()
                .timer(hyper_util::rt::TokioTimer::new())
                .keep_alive(false)
                .header_read_timeout(Some(idle));
            server
                .http_builder()
                .http2()
                .timer(hyper_util::rt::TokioTimer::new())
                .keep_alive_interval(Some(idle / 2))
                .keep_alive_timeout(idle);
            server
                .serve(make_service)
                .await
                .map_err(|_| ServerError::Io)
        }
        _ => Err(ServerError::Configuration),
    }
}

async fn search(
    State(state): State<AppState>,
    params: Result<Query<SearchParams>, QueryRejection>,
) -> Response {
    let Ok(Query(params)) = params else {
        return api_error(StatusCode::BAD_REQUEST, "invalid_request");
    };
    if !valid_query(&params.q) {
        return api_error(StatusCode::BAD_REQUEST, "invalid_query");
    }
    match state.service.search(params.q, ServiceSurface::Api).await {
        Ok(value) => Json(value.response).into_response(),
        Err(error) => service_error(error),
    }
}

async fn deep(
    State(state): State<AppState>,
    params: Result<Query<SearchParams>, QueryRejection>,
) -> Response {
    let Ok(Query(params)) = params else {
        return api_error(StatusCode::BAD_REQUEST, "invalid_request");
    };
    if !valid_query(&params.q) {
        return api_error(StatusCode::BAD_REQUEST, "invalid_query");
    }
    match state.service.deep(params.q, ServiceSurface::Api).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => service_error(error),
    }
}

async fn search_post(
    State(state): State<AppState>,
    params: Result<Json<SearchParams>, JsonRejection>,
) -> Response {
    let Json(params) = match params {
        Ok(params) => params,
        Err(rejection) => return api_error(rejection.status(), "invalid_request"),
    };
    if !valid_query(&params.q) {
        return api_error(StatusCode::BAD_REQUEST, "invalid_query");
    }
    match state.service.search(params.q, ServiceSurface::Api).await {
        Ok(value) => Json(value.response).into_response(),
        Err(error) => service_error(error),
    }
}

async fn deep_post(
    State(state): State<AppState>,
    params: Result<Json<SearchParams>, JsonRejection>,
) -> Response {
    let Json(params) = match params {
        Ok(params) => params,
        Err(rejection) => return api_error(rejection.status(), "invalid_request"),
    };
    if !valid_query(&params.q) {
        return api_error(StatusCode::BAD_REQUEST, "invalid_query");
    }
    match state.service.deep(params.q, ServiceSurface::Api).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => service_error(error),
    }
}

async fn providers(State(state): State<AppState>) -> Response {
    match state.service.provider_summaries() {
        Ok(providers) => Json(ProviderResponse {
            schema_version: SCHEMA_VERSION.into(),
            providers,
        })
        .into_response(),
        Err(error) => service_error(error),
    }
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "schema_version": SCHEMA_VERSION, "status": "ok" }))
}

async fn static_asset(uri: Uri) -> Response {
    let Some(value) = asset(uri.path()) else {
        return api_error(StatusCode::NOT_FOUND, "not_found");
    };
    let mut response = Response::new(Body::from(value.body));
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(value.content_type));
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static(value.cache_control));
    response
}

async fn security_middleware(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let security = &state.security;
    if header_size(request.headers()) > security.max_header_bytes {
        return secured(
            api_error(
                StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
                "headers_too_large",
            ),
            security.https,
        );
    }
    if request
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|value| value > security.max_body_bytes)
    {
        return secured(
            api_error(StatusCode::PAYLOAD_TOO_LARGE, "body_too_large"),
            security.https,
        );
    }
    if !valid_host(request.headers(), &security.allowed_hosts) {
        return secured(
            api_error(StatusCode::BAD_REQUEST, "invalid_host"),
            security.https,
        );
    }
    if !valid_origin(request.headers(), &security.allowed_origins) {
        return secured(
            api_error(StatusCode::FORBIDDEN, "invalid_origin"),
            security.https,
        );
    }
    let protected = is_protected(request.uri().path());
    if request.method() != Method::OPTIONS && !within_rate_limit(&request, security) {
        let mut response = api_error(StatusCode::TOO_MANY_REQUESTS, "rate_limited");
        response
            .headers_mut()
            .insert("retry-after", HeaderValue::from_static("60"));
        return secured(response, security.https);
    }
    if protected
        && request.method() != Method::OPTIONS
        && !authorized(request.headers(), security.token.as_deref())
    {
        let mut response = api_error(StatusCode::UNAUTHORIZED, "unauthorized");
        response
            .headers_mut()
            .insert(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        return secured(response, security.https);
    }
    let timeout = security.timeout;
    let https = security.https;
    let response = match tokio::time::timeout(timeout, next.run(request)).await {
        Ok(response) => response,
        Err(_) => api_error(StatusCode::GATEWAY_TIMEOUT, "request_timeout"),
    };
    secured(response, https)
}

fn secured(mut response: Response, https: bool) -> Response {
    for (name, value) in security_headers(https) {
        response.headers_mut().insert(
            HeaderName::from_static(match name {
                "Content-Security-Policy" => "content-security-policy",
                "X-Content-Type-Options" => "x-content-type-options",
                "Referrer-Policy" => "referrer-policy",
                "Permissions-Policy" => "permissions-policy",
                "X-Frame-Options" => "x-frame-options",
                "Strict-Transport-Security" => "strict-transport-security",
                _ => continue,
            }),
            HeaderValue::from_static(value),
        );
    }
    if !response.headers().contains_key(CACHE_CONTROL) {
        response
            .headers_mut()
            .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    response
}

fn valid_query(query: &str) -> bool {
    !query.trim().is_empty() && query.len() <= 2048
}

fn is_protected(path: &str) -> bool {
    matches!(path, "/search" | "/deep" | "/providers" | "/mcp") || path.starts_with("/mcp/")
}

fn authorized(headers: &HeaderMap, expected: Option<&str>) -> bool {
    let Some(expected) = expected else {
        return true;
    };
    let Some(actual) = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
    else {
        return false;
    };
    constant_time_eq(actual.as_bytes(), expected.as_bytes())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    for index in 0..left.len().max(right.len()) {
        difference |= usize::from(
            left.get(index).copied().unwrap_or_default()
                ^ right.get(index).copied().unwrap_or_default(),
        );
    }
    difference == 0
}

fn valid_host(headers: &HeaderMap, allowed: &[String]) -> bool {
    let Some(authority) = headers.get(HOST).and_then(|value| value.to_str().ok()) else {
        return false;
    };
    let Ok(parsed) = authority.parse::<axum::http::uri::Authority>() else {
        return false;
    };
    allowed.iter().any(|entry| {
        entry.eq_ignore_ascii_case(parsed.as_str())
            || entry.eq_ignore_ascii_case(parsed.host())
            || (entry == "[::1]" && parsed.host() == "[::1]")
    })
}

fn valid_origin(headers: &HeaderMap, allowed: &[String]) -> bool {
    let Some(origin) = headers.get(ORIGIN) else {
        return true;
    };
    origin
        .to_str()
        .ok()
        .is_some_and(|origin| allowed.iter().any(|value| value == origin))
}

fn header_size(headers: &HeaderMap) -> usize {
    headers
        .iter()
        .map(|(name, value)| name.as_str().len().saturating_add(value.as_bytes().len()))
        .sum()
}

fn within_rate_limit(request: &Request, security: &SecurityState) -> bool {
    let ip = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|value| value.0.ip())
        .unwrap_or(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
    let now = Instant::now();
    let mut limiter = security
        .rate_limiter
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if now.duration_since(limiter.last_cleanup) >= Duration::from_secs(60) {
        limiter
            .windows
            .retain(|_, window| now.duration_since(window.started) < Duration::from_secs(60));
        limiter.last_cleanup = now;
    }
    let window = limiter.windows.entry(ip).or_insert(RateWindow {
        started: now,
        count: 0,
    });
    if window.count >= security.rate_limit_per_minute {
        return false;
    }
    window.count += 1;
    true
}

fn effective_origins(config: &amatl_core::Config, https: bool) -> Vec<String> {
    if !config.server.allowed_origins.is_empty() {
        return config.server.allowed_origins.clone();
    }
    let scheme = if https { "https" } else { "http" };
    config
        .server
        .allowed_hosts
        .iter()
        .map(|host| format!("{scheme}://{host}:{}", config.server.port))
        .collect()
}

fn cors_layer(origins: &[String]) -> Result<CorsLayer, ServerError> {
    let origins = origins
        .iter()
        .map(|value| HeaderValue::from_str(value).map_err(|_| ServerError::Configuration))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([AUTHORIZATION, CONTENT_TYPE, axum::http::header::ACCEPT]))
}

fn service_error(error: ServiceError) -> Response {
    match error {
        ServiceError::InvalidQuery => api_error(StatusCode::BAD_REQUEST, "invalid_query"),
        ServiceError::MissingPlan | ServiceError::Configuration => {
            api_error(StatusCode::INTERNAL_SERVER_ERROR, "service_unavailable")
        }
    }
}

fn api_error(status: StatusCode, code: &'static str) -> Response {
    (
        status,
        Json(json!({
            "schema_version": SCHEMA_VERSION,
            "error": { "code": code, "message": code }
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests;
