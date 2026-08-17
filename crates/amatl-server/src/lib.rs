//! Hardened HTTP API, UI hosting and MCP Streamable HTTP surface for AMATL.

mod mcp;

use amatl_core::{
    AmatlService, ConfigError, ErrorCode, Scope, ServiceError, ServiceSurface, MCP_TOOLS,
    SCHEMA_VERSION,
};
use amatl_ui::{asset, security_headers};
use axum::{
    body::Body,
    extract::{
        rejection::{JsonRejection, QueryRejection},
        ConnectInfo, DefaultBodyLimit, Extension, Path, Query, Request, State,
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
    routing::{delete, get, post},
    Json, Router,
};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, VecDeque},
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, RwLock,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use thiserror::Error;
use tower::limit::ConcurrencyLimitLayer;
use tower_http::cors::CorsLayer;
use tracing::Instrument;

const REQUEST_ID_HEADER: &str = "x-request-id";
/// Shortest bearer token accepted from configuration or the environment.
const MINIMUM_TOKEN_BYTES: usize = 32;
static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// Wrapper so handlers can extract the request-id injected by the security
/// middleware via [`axum::Extension`] or [`Request::extensions`].
#[derive(Clone, Debug)]
struct RequestId(String);

impl RequestId {
    fn into_inner(self) -> String {
        self.0
    }
}

/// Shared, swappable service handle.
///
/// A reload builds a brand new [`AmatlService`] and swaps the pointer, so
/// requests already running finish against the configuration they started
/// with and the next request picks up the new one. Handlers therefore take a
/// snapshot (`state.service()`) instead of holding the lock across `.await`.
/// Swappable service shared by every surface in one process.
pub(crate) type ServiceHandle = Arc<RwLock<AmatlService>>;

#[derive(Clone)]
struct AppState {
    service: ServiceHandle,
    /// Swapped on reload so credentials rotate without a restart.
    security: Arc<RwLock<Arc<SecurityState>>>,
    /// Survives reloads on purpose: a reload must not reset rate windows.
    rate_limiter: Arc<Mutex<RateLimiter>>,
    /// Token supplied programmatically at construction, if any; re-applied on
    /// every reload so an embedder keeps its credential.
    explicit_token: Option<String>,
    /// Configuration file a reload re-reads, when the process was started with
    /// one. Without it a reload revalidates and rebuilds from the running
    /// configuration, which still resets provider construction and breakers.
    config_path: Option<PathBuf>,
    metrics: Arc<RequestMetrics>,
}

impl AppState {
    /// Current security snapshot; cheap, and never held across an await.
    fn security(&self) -> Arc<SecurityState> {
        self.security
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    /// Current service snapshot; cheap, every field behind it is an `Arc`.
    fn service(&self) -> AmatlService {
        self.service
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    /// Rebuild the service from the configuration file, or from the running
    /// configuration when the process was started without one.
    ///
    /// The new service is validated and fully built before the swap, so a
    /// rejected reload leaves the running one in place.
    async fn reload(self) -> Result<ReloadReport, ServiceError> {
        let current = self.service();
        let config = match &self.config_path {
            Some(path) => {
                amatl_core::Config::load_optional(path).map_err(|_| ServiceError::Configuration)?
            }
            None => current.config().clone(),
        };
        // Credentials are part of the reloaded configuration: rotating a token
        // or retiring a client must not need a restart either. Rebuilt before
        // the swap so a bad credential set changes nothing.
        let security =
            resolve_clients(replacement_config_ref(&config), self.explicit_token.clone())
                .map_err(|_| ServiceError::Configuration)?;
        let clients = security
            .iter()
            .map(|client| client.id.clone())
            .collect::<Vec<_>>();
        let replacement = current.reloaded_detached(config).await?;
        let security = Arc::new(SecurityState {
            clients: security,
            allowed_hosts: replacement.config().server.allowed_hosts.clone(),
            allowed_origins: effective_origins(
                replacement.config(),
                replacement.config().server.tls.cert_path.is_some(),
            ),
            max_header_bytes: replacement.config().server.max_header_bytes,
            max_body_bytes: replacement.config().server.max_body_bytes,
            timeout: Duration::from_millis(replacement.config().server.request_timeout_ms),
            rate_limit_per_minute: replacement.config().server.rate_limit_per_minute,
            https: replacement.config().server.tls.cert_path.is_some(),
        });
        let report = ReloadReport {
            schema_version: SCHEMA_VERSION.into(),
            config_file: self
                .config_path
                .as_ref()
                .map(|path| path.display().to_string()),
            declared_sources: replacement
                .config()
                .providers
                .names()
                .into_iter()
                .map(str::to_owned)
                .collect(),
            enabled_sources: replacement.config().providers.enabled.clone(),
            registered_sources: replacement
                .registry()
                .names()
                .into_iter()
                .map(str::to_owned)
                .collect(),
            inference_backend: replacement.inference_backend().map(str::to_owned),
            clients,
        };
        *self
            .service
            .write()
            .unwrap_or_else(|error| error.into_inner()) = replacement;
        *self
            .security
            .write()
            .unwrap_or_else(|error| error.into_inner()) = security;
        tracing::info!(
            target: "amatl::http",
            enabled_sources = ?report.enabled_sources,
            "configuration reloaded"
        );
        Ok(report)
    }
}

/// Borrow helper that keeps the reload readable: the configuration is moved
/// into the rebuild, so credentials are resolved from it first.
fn replacement_config_ref(config: &amatl_core::Config) -> &amatl_core::Config {
    config
}

/// What a successful reload put in place.
#[derive(Debug, Serialize)]
struct ReloadReport {
    schema_version: String,
    config_file: Option<String>,
    declared_sources: Vec<String>,
    enabled_sources: Vec<String>,
    registered_sources: Vec<String>,
    inference_backend: Option<String>,
    /// Credential identities accepted after the reload. Never their secrets.
    clients: Vec<String>,
}

/// Lightweight request counters exposed via `/metrics` in Prometheus
/// exposition format. Counters are monotonic and reset on restart.
#[derive(Default)]
struct RequestMetrics {
    search_total: AtomicU64,
    deep_total: AtomicU64,
    answer_total: AtomicU64,
    search_errors: AtomicU64,
    deep_errors: AtomicU64,
    answer_errors: AtomicU64,
    rate_limited_total: AtomicU64,
    unauthorized_total: AtomicU64,
    request_timeout_total: AtomicU64,
    search_latency: LatencyWindow,
    deep_latency: LatencyWindow,
    answer_latency: LatencyWindow,
}

/// Bounded ring of recent latencies used to publish quantiles without a
/// dependency on a metrics runtime. Only the last [`LATENCY_WINDOW`] samples
/// are kept, so memory stays constant under load.
#[derive(Default)]
struct LatencyWindow {
    samples: Mutex<VecDeque<u64>>,
}

const LATENCY_WINDOW: usize = 1_024;

impl LatencyWindow {
    fn record(&self, elapsed_ms: u64) {
        let mut samples = self
            .samples
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if samples.len() == LATENCY_WINDOW {
            samples.pop_front();
        }
        samples.push_back(elapsed_ms);
    }

    /// `(samples, p50, p95, p99)` over the retained window.
    fn quantiles(&self) -> (usize, u64, u64, u64) {
        let samples = self
            .samples
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if samples.is_empty() {
            return (0, 0, 0, 0);
        }
        let mut sorted = samples.iter().copied().collect::<Vec<_>>();
        sorted.sort_unstable();
        let at = |quantile: f64| {
            let index = (((sorted.len() - 1) as f64) * quantile).ceil() as usize;
            sorted[index]
        };
        (sorted.len(), at(0.50), at(0.95), at(0.99))
    }
}

/// Everything the security middleware needs for one request.
///
/// Rebuilt wholesale on reload, which is what makes credential rotation work
/// without a restart. The rate limiter deliberately lives outside: keeping it
/// across reloads means a reload cannot be used to reset someone's window.
struct SecurityState {
    /// Accepted credentials. Empty means authentication is disabled
    /// (`no_auth`, loopback only).
    clients: Vec<AuthorizedClient>,
    allowed_hosts: Vec<String>,
    allowed_origins: Vec<String>,
    max_header_bytes: usize,
    max_body_bytes: usize,
    timeout: Duration,
    rate_limit_per_minute: u32,
    https: bool,
}

/// One credential resolved to the digest actually compared at request time.
struct AuthorizedClient {
    id: String,
    /// SHA-256 of the bearer token. The token itself is never stored.
    digest: [u8; 32],
    expires_at: Option<String>,
    scopes: Vec<Scope>,
    tools: Vec<String>,
}

/// Identity of the caller, attached to the request for handlers, the MCP
/// surface and audit events.
#[derive(Clone, Debug)]
pub(crate) struct ClientIdentity {
    pub(crate) id: String,
    pub(crate) tools: Vec<String>,
}

impl ClientIdentity {
    /// Identity used when authentication is disabled: loopback development
    /// only, and still explicit rather than implied.
    fn unauthenticated() -> Self {
        Self {
            id: "anonymous".into(),
            tools: MCP_TOOLS.iter().map(|tool| (*tool).to_owned()).collect(),
        }
    }

    pub(crate) fn allows_tool(&self, tool: &str) -> bool {
        self.tools.iter().any(|allowed| allowed == tool)
    }
}

/// Build the accepted credential set from configuration and the environment.
///
/// A declared client whose secret is missing from the environment is skipped
/// with a warning instead of silently accepting anything: the surface stays
/// closed for that identity and open for the others.
fn resolve_clients(
    config: &amatl_core::Config,
    explicit_token: Option<String>,
) -> Result<Vec<AuthorizedClient>, ServerError> {
    let server = &config.server;
    if server.no_auth {
        return Ok(vec![]);
    }
    let mut clients = Vec::new();
    if let Some(token) = explicit_token
        .or_else(|| std::env::var(&server.token_env).ok())
        .filter(|value| value.len() >= MINIMUM_TOKEN_BYTES)
    {
        clients.push(AuthorizedClient {
            id: "default".into(),
            digest: token_digest(&token),
            expires_at: None,
            scopes: Scope::ALL.to_vec(),
            tools: MCP_TOOLS.iter().map(|tool| (*tool).to_owned()).collect(),
        });
    }
    for declared in &server.clients {
        let digest = match (&declared.token_env, &declared.token_sha256) {
            (Some(name), _) => match std::env::var(name)
                .ok()
                .filter(|value| value.len() >= MINIMUM_TOKEN_BYTES)
            {
                Some(token) => token_digest(&token),
                None => {
                    tracing::warn!(
                        target: "amatl::security",
                        security_event = "client_credential_missing",
                        client_id = %declared.id,
                        "declared client has no usable credential in the environment; it cannot authenticate"
                    );
                    continue;
                }
            },
            (None, Some(hex)) => match digest_from_hex(hex) {
                Some(digest) => digest,
                None => continue,
            },
            (None, None) => continue,
        };
        clients.push(AuthorizedClient {
            id: declared.id.clone(),
            digest,
            expires_at: declared.expires_at.clone(),
            scopes: declared.scopes.clone(),
            tools: declared.tools.clone(),
        });
    }
    if clients.is_empty() {
        return Err(ServerError::MissingToken);
    }
    Ok(clients)
}

fn token_digest(token: &str) -> [u8; 32] {
    let mut digest = [0_u8; 32];
    digest.copy_from_slice(&Sha256::digest(token.as_bytes()));
    digest
}

fn digest_from_hex(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64 {
        return None;
    }
    let mut digest = [0_u8; 32];
    for (index, byte) in digest.iter_mut().enumerate() {
        *byte = u8::from_str_radix(value.get(index * 2..index * 2 + 2)?, 16).ok()?;
    }
    Some(digest)
}

/// Capability a route requires. `None` means the route is public.
///
/// This is the single source of truth for "is this protected": reading and
/// mutating the same path are different capabilities, so the method is part of
/// the decision.
fn required_scope(method: &Method, path: &str) -> Option<Scope> {
    let mutating = matches!(*method, Method::POST | Method::PUT | Method::DELETE);
    match path {
        "/search" => Some(Scope::Search),
        "/deep" => Some(Scope::Deep),
        // Reuses Deep's scope rather than a dedicated one: like Deep, this
        // runs a full search and then does more expensive, sensitive work on
        // top of it (here, an outbound call to a third-party LLM).
        "/answer" => Some(Scope::Deep),
        "/providers" | "/status" => Some(Scope::Read),
        "/history" | "/saved" if mutating => Some(Scope::Write),
        "/history" | "/saved" => Some(Scope::Read),
        // The audit trail names identities and addresses: operator only.
        // The answer toggle is admin too, same tier as /reload: it rewrites
        // the running configuration file, even though the field it touches
        // is narrow (see `Config::set_answer_enabled`).
        "/security-events" | "/reload" | "/answer/enabled" => Some(Scope::Admin),
        "/mcp" => Some(Scope::Mcp),
        _ if path.starts_with("/mcp/") => Some(Scope::Mcp),
        // `/history/{id}` and `/saved/{id}` only ever delete.
        _ if path.starts_with("/history/") || path.starts_with("/saved/") => Some(Scope::Write),
        _ => None,
    }
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
    #[serde(default)]
    page: Option<u32>,
    #[serde(default)]
    page_size: Option<u32>,
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
    build_router_with_config_path(service, explicit_token, None).await
}

/// Triggers the same reload as `POST /reload` from outside the HTTP surface.
#[derive(Clone)]
pub struct ReloadHandle {
    state: AppState,
}

impl ReloadHandle {
    /// Rebuild the service; errors are reported, never fatal to the listener.
    pub async fn reload(&self) -> Result<(), ServerError> {
        self.state
            .clone()
            .reload()
            .await
            .map(|_| ())
            .map_err(|_| ServerError::Configuration)
    }
}

/// Router that can reload from a configuration file on demand.
///
/// `config_path` is the file `POST /reload` and `SIGHUP` re-read; pass `None`
/// when the process was started without one.
pub async fn build_router_with_config_path(
    service: AmatlService,
    explicit_token: Option<String>,
    config_path: Option<PathBuf>,
) -> Result<Router, ServerError> {
    build_router_with_reload(service, explicit_token, config_path)
        .await
        .map(|(router, _)| router)
}

/// Router plus the handle a signal listener uses to reload it.
pub async fn build_router_with_reload(
    service: AmatlService,
    explicit_token: Option<String>,
    config_path: Option<PathBuf>,
) -> Result<(Router, ReloadHandle), ServerError> {
    service.config().validate()?;
    let server = service.config().server.clone();
    let https = server.tls.cert_path.is_some();
    let clients = resolve_clients(service.config(), explicit_token.clone())?;
    let allowed_origins = effective_origins(service.config(), https);
    let security = Arc::new(SecurityState {
        clients,
        allowed_hosts: server.allowed_hosts.clone(),
        allowed_origins: allowed_origins.clone(),
        max_header_bytes: server.max_header_bytes,
        max_body_bytes: server.max_body_bytes,
        timeout: Duration::from_millis(server.request_timeout_ms),
        rate_limit_per_minute: server.rate_limit_per_minute,
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
    // One handle, shared by HTTP and MCP, so a reload reaches both surfaces.
    let handle: ServiceHandle = Arc::new(RwLock::new(service));
    let mcp_service: StreamableHttpService<mcp::McpSurface, LocalSessionManager> =
        StreamableHttpService::new(
            {
                let handle = handle.clone();
                move || Ok(mcp::McpSurface::new(handle.clone()))
            },
            Default::default(),
            mcp_config,
        );
    let state = AppState {
        service: handle,
        config_path,
        security: Arc::new(RwLock::new(security)),
        rate_limiter: Arc::new(Mutex::new(RateLimiter {
            windows: BTreeMap::new(),
            last_cleanup: Instant::now(),
        })),
        explicit_token,
        metrics: Arc::new(RequestMetrics::default()),
    };
    let cors = cors_layer(&allowed_origins)?;
    let reload_handle = ReloadHandle {
        state: state.clone(),
    };
    let router = Router::new()
        .route("/search", get(search).post(search_post))
        .route("/deep", get(deep).post(deep_post))
        .route("/answer", post(answer_post))
        .route("/providers", get(providers))
        .route("/status", get(status))
        .route("/history", get(history).delete(purge_history))
        .route("/history/{id}", delete(delete_history_entry))
        .route("/saved", get(saved_documents).post(save_document))
        .route("/saved/{id}", delete(delete_saved_document))
        .route("/reload", axum::routing::post(reload))
        .route("/answer/enabled", post(answer_toggle))
        .route("/security-events", get(security_events))
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/metrics", get(metrics))
        .nest_service("/mcp", mcp_service)
        .fallback(static_asset)
        .with_state(state.clone())
        .layer(ConcurrencyLimitLayer::new(server.max_connections))
        .layer(DefaultBodyLimit::max(server.max_body_bytes))
        .layer(cors)
        .layer(middleware::from_fn_with_state(state, security_middleware));
    Ok((router, reload_handle))
}

pub async fn serve(service: AmatlService) -> Result<(), ServerError> {
    serve_with_config_path(service, None).await
}

/// Serve with a configuration file that `POST /reload` and `SIGHUP` re-read.
///
/// Startup reports the effective listener as one structured log line, so a
/// supervisor can confirm bind, TLS, authentication and reload support without
/// parsing prose.
pub async fn serve_with_config_path(
    service: AmatlService,
    config_path: Option<PathBuf>,
) -> Result<(), ServerError> {
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
    let authenticated = !service.config().server.no_auth;
    let (app, reload_handle) = build_router_with_reload(service, None, config_path.clone()).await?;
    install_reload_signal(reload_handle);
    tracing::info!(
        target: "amatl::http",
        event = "listening",
        bind = %address,
        tls = tls.cert_path.is_some(),
        authenticated,
        config_file = config_path.as_ref().map(|path| path.display().to_string()),
        reload = "POST /reload or SIGHUP",
        "AMATL server is listening"
    );
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

/// Reload on `SIGHUP`, the conventional signal for "re-read configuration".
///
/// Unix only; on other platforms `POST /reload` remains the way in.
#[cfg(unix)]
fn install_reload_signal(handle: ReloadHandle) {
    tokio::spawn(async move {
        let Ok(mut hangup) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
        else {
            tracing::warn!(
                target: "amatl::http",
                "SIGHUP handler could not be installed; use POST /reload instead"
            );
            return;
        };
        while hangup.recv().await.is_some() {
            match handle.reload().await {
                Ok(()) => tracing::info!(
                    target: "amatl::http",
                    event = "reloaded",
                    signal = "SIGHUP",
                    "configuration reloaded"
                ),
                Err(_) => tracing::warn!(
                    target: "amatl::http",
                    event = "reload_rejected",
                    signal = "SIGHUP",
                    "configuration reload was rejected; the running service is unchanged"
                ),
            }
        }
    });
}

#[cfg(not(unix))]
fn install_reload_signal(_handle: ReloadHandle) {}

async fn search(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    params: Result<Query<SearchParams>, QueryRejection>,
) -> Response {
    let Ok(Query(params)) = params else {
        return api_error(ErrorCode::InvalidRequest);
    };
    if !valid_query(&params.q) {
        return api_error(ErrorCode::InvalidQuery);
    }
    let started = Instant::now();
    let outcome = state
        .service()
        .search_paginated(
            params.q,
            ServiceSurface::api(Some(request_id.into_inner())),
            params.page,
            params.page_size,
        )
        .await;
    state
        .metrics
        .search_latency
        .record(started.elapsed().as_millis() as u64);
    match outcome {
        Ok(value) => {
            state.metrics.search_total.fetch_add(1, Ordering::Relaxed);
            Json(value.response).into_response()
        }
        Err(error) => {
            state.metrics.search_errors.fetch_add(1, Ordering::Relaxed);
            service_error(error)
        }
    }
}

async fn deep(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    params: Result<Query<SearchParams>, QueryRejection>,
) -> Response {
    let Ok(Query(params)) = params else {
        return api_error(ErrorCode::InvalidRequest);
    };
    if !valid_query(&params.q) {
        return api_error(ErrorCode::InvalidQuery);
    }
    let started = Instant::now();
    let outcome = state
        .service()
        .deep(params.q, ServiceSurface::api(Some(request_id.into_inner())))
        .await;
    state
        .metrics
        .deep_latency
        .record(started.elapsed().as_millis() as u64);
    match outcome {
        Ok(value) => {
            state.metrics.deep_total.fetch_add(1, Ordering::Relaxed);
            Json(value).into_response()
        }
        Err(error) => {
            state.metrics.deep_errors.fetch_add(1, Ordering::Relaxed);
            service_error(error)
        }
    }
}

async fn search_post(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    params: Result<Json<SearchParams>, JsonRejection>,
) -> Response {
    let Json(params) = match params {
        Ok(params) => params,
        Err(rejection) => {
            return api_error_with_status(rejection.status(), ErrorCode::InvalidRequest)
        }
    };
    if !valid_query(&params.q) {
        return api_error(ErrorCode::InvalidQuery);
    }
    let started = Instant::now();
    let outcome = state
        .service()
        .search_paginated(
            params.q,
            ServiceSurface::api(Some(request_id.into_inner())),
            params.page,
            params.page_size,
        )
        .await;
    state
        .metrics
        .search_latency
        .record(started.elapsed().as_millis() as u64);
    match outcome {
        Ok(value) => {
            state.metrics.search_total.fetch_add(1, Ordering::Relaxed);
            Json(value.response).into_response()
        }
        Err(error) => {
            state.metrics.search_errors.fetch_add(1, Ordering::Relaxed);
            service_error(error)
        }
    }
}

async fn deep_post(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    params: Result<Json<SearchParams>, JsonRejection>,
) -> Response {
    let Json(params) = match params {
        Ok(params) => params,
        Err(rejection) => {
            return api_error_with_status(rejection.status(), ErrorCode::InvalidRequest)
        }
    };
    if !valid_query(&params.q) {
        return api_error(ErrorCode::InvalidQuery);
    }
    let started = Instant::now();
    let outcome = state
        .service()
        .deep(params.q, ServiceSurface::api(Some(request_id.into_inner())))
        .await;
    state
        .metrics
        .deep_latency
        .record(started.elapsed().as_millis() as u64);
    match outcome {
        Ok(value) => {
            state.metrics.deep_total.fetch_add(1, Ordering::Relaxed);
            Json(value).into_response()
        }
        Err(error) => {
            state.metrics.deep_errors.fetch_add(1, Ordering::Relaxed);
            service_error(error)
        }
    }
}

/// Runs a search and synthesizes a grounded, cited answer from it. Mirrors
/// `search_post`/`deep_post` exactly; the only difference is which service
/// method it calls. Distinct from both on purpose: `search`/`deep` never
/// change behavior because this route exists, and this route never runs
/// unless the caller explicitly hits it.
async fn answer_post(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    params: Result<Json<SearchParams>, JsonRejection>,
) -> Response {
    let Json(params) = match params {
        Ok(params) => params,
        Err(rejection) => {
            return api_error_with_status(rejection.status(), ErrorCode::InvalidRequest)
        }
    };
    if !valid_query(&params.q) {
        return api_error(ErrorCode::InvalidQuery);
    }
    let started = Instant::now();
    let outcome = state
        .service()
        .answer(params.q, ServiceSurface::api(Some(request_id.into_inner())))
        .await;
    state
        .metrics
        .answer_latency
        .record(started.elapsed().as_millis() as u64);
    match outcome {
        Ok(value) => {
            state.metrics.answer_total.fetch_add(1, Ordering::Relaxed);
            Json(value).into_response()
        }
        Err(error) => {
            state.metrics.answer_errors.fetch_add(1, Ordering::Relaxed);
            service_error(error)
        }
    }
}

async fn providers(State(state): State<AppState>) -> Response {
    match state.service().provider_summaries() {
        Ok(providers) => Json(ProviderResponse {
            schema_version: SCHEMA_VERSION.into(),
            providers,
        })
        .into_response(),
        Err(error) => service_error(error),
    }
}

/// Bounded listing window for the local domain surfaces.
#[derive(Debug, Deserialize)]
struct PageParams {
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    offset: Option<u32>,
}

impl PageParams {
    fn window(&self) -> (u32, u32) {
        (
            self.limit.unwrap_or(50).clamp(1, 200),
            self.offset.unwrap_or(0),
        )
    }
}

/// Rebuild the service from configuration without restarting the process.
///
/// Adding, removing or re-approving a source is therefore a file edit plus one
/// call. The reload is atomic from a client's point of view: it either swaps a
/// fully built service or changes nothing and reports why.
async fn reload(State(state): State<AppState>) -> Response {
    match state.clone().reload().await {
        Ok(report) => Json(report).into_response(),
        Err(error) => service_error(error),
    }
}

#[derive(Debug, Deserialize)]
struct AnswerToggleParams {
    enabled: bool,
}

/// Flip `answer.enabled` and nothing else — admin scoped, same trust tier as
/// `/reload`, which this reuses for the actual apply step.
///
/// Validates a would-be config with the flag flipped *before* writing
/// anything: if turning the feature on would leave `answer` unable to pass
/// `Config::validate` (missing `endpoint`/`model`, an operator never
/// finished configuring), this fails closed without ever touching the file
/// or the credential — a half-written toggle is worse than no toggle.
async fn answer_toggle(
    State(state): State<AppState>,
    params: Result<Json<AnswerToggleParams>, JsonRejection>,
) -> Response {
    let Json(params) = match params {
        Ok(params) => params,
        Err(rejection) => {
            return api_error_with_status(rejection.status(), ErrorCode::InvalidRequest)
        }
    };
    let Some(path) = state.config_path.clone() else {
        return api_error(ErrorCode::ConfigurationInvalid);
    };
    let mut candidate = state.service().config().clone();
    candidate.answer.enabled = params.enabled;
    if candidate.validate().is_err() {
        return api_error(ErrorCode::ConfigurationInvalid);
    }
    if amatl_core::Config::set_answer_enabled(&path, params.enabled).is_err() {
        return api_error(ErrorCode::ConfigurationInvalid);
    }
    match state.clone().reload().await {
        Ok(report) => Json(report).into_response(),
        Err(error) => service_error(error),
    }
}

/// Recorded security rejections, newest first.
///
/// Requires the admin scope: the trail names client identities and addresses.
async fn security_events(
    State(state): State<AppState>,
    params: Result<Query<PageParams>, QueryRejection>,
) -> Response {
    let Ok(Query(params)) = params else {
        return api_error(ErrorCode::InvalidRequest);
    };
    let (limit, offset) = params.window();
    let service = state.service();
    let audit = service.audit();
    match audit.events(limit, offset).await {
        Ok(events) => Json(json!({
            "schema_version": SCHEMA_VERSION,
            "events": events,
            "dropped": audit.dropped()
        }))
        .into_response(),
        Err(_) => api_error(ErrorCode::StorageUnavailable),
    }
}

/// Operator status: source availability, persistence and cache state.
async fn status(State(state): State<AppState>) -> Response {
    match state.service().status().await {
        Ok(value) => Json(value).into_response(),
        Err(error) => service_error(error),
    }
}

async fn history(
    State(state): State<AppState>,
    params: Result<Query<PageParams>, QueryRejection>,
) -> Response {
    let Ok(Query(params)) = params else {
        return api_error(ErrorCode::InvalidRequest);
    };
    let (limit, offset) = params.window();
    match state.service().history(limit, offset).await {
        Ok(entries) => Json(json!({
            "schema_version": SCHEMA_VERSION,
            "entries": entries
        }))
        .into_response(),
        Err(error) => service_error(error),
    }
}

async fn delete_history_entry(State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    match state.service().delete_history_entry(id).await {
        Ok(true) => Json(json!({ "schema_version": SCHEMA_VERSION, "deleted": 1 })).into_response(),
        Ok(false) => api_error(ErrorCode::NotFound),
        Err(error) => service_error(error),
    }
}

async fn purge_history(State(state): State<AppState>) -> Response {
    match state.service().purge_history().await {
        Ok(deleted) => {
            Json(json!({ "schema_version": SCHEMA_VERSION, "deleted": deleted })).into_response()
        }
        Err(error) => service_error(error),
    }
}

async fn saved_documents(
    State(state): State<AppState>,
    params: Result<Query<PageParams>, QueryRejection>,
) -> Response {
    let Ok(Query(params)) = params else {
        return api_error(ErrorCode::InvalidRequest);
    };
    let (limit, offset) = params.window();
    match state.service().saved_documents(limit, offset).await {
        Ok(documents) => Json(json!({
            "schema_version": SCHEMA_VERSION,
            "documents": documents
        }))
        .into_response(),
        Err(error) => service_error(error),
    }
}

async fn save_document(
    State(state): State<AppState>,
    input: Result<Json<amatl_core::SaveDocumentInput>, JsonRejection>,
) -> Response {
    let Json(input) = match input {
        Ok(input) => input,
        Err(rejection) => {
            return api_error_with_status(rejection.status(), ErrorCode::InvalidRequest)
        }
    };
    match state.service().save_document(input).await {
        Ok(id) => Json(json!({ "schema_version": SCHEMA_VERSION, "id": id })).into_response(),
        Err(error) => service_error(error),
    }
}

async fn delete_saved_document(State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    match state.service().delete_saved_document(id).await {
        Ok(true) => Json(json!({ "schema_version": SCHEMA_VERSION, "deleted": 1 })).into_response(),
        Ok(false) => api_error(ErrorCode::NotFound),
        Err(error) => service_error(error),
    }
}

/// Liveness: the process is up and the router is serving.
///
/// Deliberately stateless and always `200`. Orchestrators use it to decide
/// whether to restart the process, which must not depend on a provider being
/// reachable or on SQLite being healthy. Readiness lives on `/ready`.
async fn health() -> Json<serde_json::Value> {
    Json(json!({ "schema_version": SCHEMA_VERSION, "status": "ok" }))
}

/// Readiness: whether this instance can currently serve useful traffic.
///
/// Public like `/health`, so the body is intentionally coarse: aggregate
/// booleans and a count, never source names, error codes or paths. Those name
/// the deployment's internals and stay behind `/status`, which requires the
/// `read` scope.
///
/// Returns `503` when degraded so a load balancer can drain the instance
/// without parsing the body.
async fn ready(State(state): State<AppState>) -> Response {
    let Ok(status) = state.service().status().await else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "schema_version": SCHEMA_VERSION,
                "status": "degraded",
                "storage_ok": false,
                "sources_available": 0,
            })),
        )
            .into_response();
    };
    // Persistence is optional by design: disabled is healthy, enabled but
    // unavailable is not.
    let storage_ok = !status.storage.enabled || status.storage.available;
    let sources_available = status
        .sources
        .iter()
        .filter(|source| source.status == amatl_core::ProviderSurfaceStatus::Available)
        .count();
    let ready = status.status == "ok";
    let code = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        code,
        Json(json!({
            "schema_version": SCHEMA_VERSION,
            "status": if ready { "ok" } else { "degraded" },
            "storage_ok": storage_ok,
            "sources_available": sources_available,
        })),
    )
        .into_response()
}

/// Exposes Prometheus-compatible metrics in text exposition format.
///
/// Counters are monotonic since the last server restart; latency quantiles
/// describe the last [`LATENCY_WINDOW`] requests per surface, and the source,
/// cache and storage gauges are read from the service at scrape time.
async fn metrics(State(state): State<AppState>) -> Response {
    let m = &state.metrics;
    let mut body = format!(
        "# HELP amatl_search_requests_total Total search requests received.\n\
         # TYPE amatl_search_requests_total counter\n\
         amatl_search_requests_total {}\n\
         # HELP amatl_deep_requests_total Total deep requests received.\n\
         # TYPE amatl_deep_requests_total counter\n\
         amatl_deep_requests_total {}\n\
         # HELP amatl_answer_requests_total Total answer requests received.\n\
         # TYPE amatl_answer_requests_total counter\n\
         amatl_answer_requests_total {}\n\
         # HELP amatl_search_errors_total Search requests that resulted in error.\n\
         # TYPE amatl_search_errors_total counter\n\
         amatl_search_errors_total {}\n\
         # HELP amatl_deep_errors_total Deep requests that resulted in error.\n\
         # TYPE amatl_deep_errors_total counter\n\
         amatl_deep_errors_total {}\n\
         # HELP amatl_answer_errors_total Answer requests that resulted in error.\n\
         # TYPE amatl_answer_errors_total counter\n\
         amatl_answer_errors_total {}\n\
         # HELP amatl_rate_limited_total Requests rejected by rate limiter.\n\
         # TYPE amatl_rate_limited_total counter\n\
         amatl_rate_limited_total {}\n\
         # HELP amatl_unauthorized_total Requests rejected for missing/invalid auth.\n\
         # TYPE amatl_unauthorized_total counter\n\
         amatl_unauthorized_total {}\n\
         # HELP amatl_request_timeout_total Requests that exceeded the timeout.\n\
         # TYPE amatl_request_timeout_total counter\n\
         amatl_request_timeout_total {}\n",
        m.search_total.load(Ordering::Relaxed),
        m.deep_total.load(Ordering::Relaxed),
        m.answer_total.load(Ordering::Relaxed),
        m.search_errors.load(Ordering::Relaxed),
        m.deep_errors.load(Ordering::Relaxed),
        m.answer_errors.load(Ordering::Relaxed),
        m.rate_limited_total.load(Ordering::Relaxed),
        m.unauthorized_total.load(Ordering::Relaxed),
        m.request_timeout_total.load(Ordering::Relaxed),
    );
    body.push_str(&latency_metrics("search", &m.search_latency));
    body.push_str(&latency_metrics("deep", &m.deep_latency));
    body.push_str(&latency_metrics("answer", &m.answer_latency));
    let service = state.service();
    body.push_str(&source_metrics(&service));
    body.push_str(&cache_metrics(&service).await);
    let mut response = Response::new(Body::from(body));
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/plain; version=0.0.4; charset=utf-8"),
    );
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// Latency quantiles for one surface, as gauges over the retained window.
fn latency_metrics(surface: &str, window: &LatencyWindow) -> String {
    let (samples, p50, p95, p99) = window.quantiles();
    format!(
        "# HELP amatl_{surface}_latency_ms Request latency quantiles over the last {LATENCY_WINDOW} {surface} requests.\n\
         # TYPE amatl_{surface}_latency_ms gauge\n\
         amatl_{surface}_latency_ms{{quantile=\"0.5\"}} {p50}\n\
         amatl_{surface}_latency_ms{{quantile=\"0.95\"}} {p95}\n\
         amatl_{surface}_latency_ms{{quantile=\"0.99\"}} {p99}\n\
         # HELP amatl_{surface}_latency_samples Retained latency samples for {surface}.\n\
         # TYPE amatl_{surface}_latency_samples gauge\n\
         amatl_{surface}_latency_samples {samples}\n"
    )
}

/// Per-source availability and observed value, labelled by source name.
fn source_metrics(service: &AmatlService) -> String {
    let Ok(summaries) = service.provider_summaries() else {
        return String::new();
    };
    let snapshots = service.source_snapshots();
    let circuits = service.circuit_snapshots();
    let mut body = String::from(
        "# HELP amatl_source_available Whether a declared source is available (1) or not (0).\n\
         # TYPE amatl_source_available gauge\n",
    );
    let mut value_block = String::from(
        "# HELP amatl_source_success_rate Observed success ratio per source in the telemetry window.\n\
         # TYPE amatl_source_success_rate gauge\n",
    );
    let mut latency_block = String::from(
        "# HELP amatl_source_latency_ms Observed average latency per source in the telemetry window.\n\
         # TYPE amatl_source_latency_ms gauge\n",
    );
    let mut circuit_block = String::from(
        "# HELP amatl_source_circuit_open Whether a source is currently in circuit cooldown.\n\
         # TYPE amatl_source_circuit_open gauge\n",
    );
    for summary in summaries {
        let label = escape_label(&summary.name);
        let available = u8::from(summary.status == amatl_core::ProviderSurfaceStatus::Available);
        body.push_str(&format!(
            "amatl_source_available{{source=\"{label}\"}} {available}\n"
        ));
        let open = u8::from(
            circuits
                .iter()
                .find(|snapshot| snapshot.provider == summary.name)
                .is_some_and(|snapshot| snapshot.state == amatl_core::CircuitState::Open),
        );
        circuit_block.push_str(&format!(
            "amatl_source_circuit_open{{source=\"{label}\"}} {open}\n"
        ));
        if let Some(snapshot) = snapshots
            .iter()
            .find(|snapshot| snapshot.provider == summary.name)
            .filter(|snapshot| snapshot.sample > 0)
        {
            value_block.push_str(&format!(
                "amatl_source_success_rate{{source=\"{label}\"}} {:.4}\n",
                snapshot.success_rate
            ));
            latency_block.push_str(&format!(
                "amatl_source_latency_ms{{source=\"{label}\"}} {:.1}\n",
                snapshot.average_latency_ms
            ));
        }
    }
    body.push_str(&circuit_block);
    body.push_str(&value_block);
    body.push_str(&latency_block);
    body
}

/// Cache effectiveness and persistence gauges.
async fn cache_metrics(service: &AmatlService) -> String {
    let cache = service.cache_effectiveness();
    let storage_available = u8::from(service.storage().is_some());
    let audit_dropped = service.audit().dropped();
    let telemetry = service.telemetry_status();
    format!(
        "# HELP amatl_cache_hits_total Cache lookups served from the local cache.\n\
         # TYPE amatl_cache_hits_total counter\n\
         amatl_cache_hits_total{{cache=\"provider_search\"}} {}\n\
         amatl_cache_hits_total{{cache=\"document\"}} {}\n\
         # HELP amatl_cache_misses_total Cache lookups that reached the origin.\n\
         # TYPE amatl_cache_misses_total counter\n\
         amatl_cache_misses_total{{cache=\"provider_search\"}} {}\n\
         amatl_cache_misses_total{{cache=\"document\"}} {}\n\
         # HELP amatl_cache_hit_rate Hit ratio per cache since start.\n\
         # TYPE amatl_cache_hit_rate gauge\n\
         amatl_cache_hit_rate{{cache=\"provider_search\"}} {:.4}\n\
         amatl_cache_hit_rate{{cache=\"document\"}} {:.4}\n\
         # HELP amatl_storage_available Whether local persistence is usable (1) or not (0).\n\
         # TYPE amatl_storage_available gauge\n\
         amatl_storage_available {storage_available}\n\
         # HELP amatl_audit_events_dropped_total Security events dropped because too many audit writes were in flight.\n\
         # TYPE amatl_audit_events_dropped_total counter\n\
         amatl_audit_events_dropped_total {audit_dropped}\n\
         # HELP amatl_telemetry_persistence_failures_total Telemetry storage writes that failed since start; memory stays authoritative and self-heals on restart via restore_best_effort.\n\
         # TYPE amatl_telemetry_persistence_failures_total counter\n\
         amatl_telemetry_persistence_failures_total {}\n\
         # HELP amatl_telemetry_in_memory_observations Observations currently retained in the in-memory telemetry window.\n\
         # TYPE amatl_telemetry_in_memory_observations gauge\n\
         amatl_telemetry_in_memory_observations {}\n",
        cache.provider_search_hits,
        cache.document_hits,
        cache.provider_search_misses,
        cache.document_misses,
        cache.provider_search_hit_rate,
        cache.document_hit_rate,
        telemetry.persistence_failures,
        telemetry.in_memory_observations,
    )
}

/// Escape a Prometheus label value; source names are configuration-controlled
/// but the exposition format must stay parseable regardless.
fn escape_label(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| match character {
            '\\' => vec!['\\', '\\'],
            '"' => vec!['\\', '"'],
            '\n' => vec!['\\', 'n'],
            other => vec![other],
        })
        .collect()
}

async fn static_asset(uri: Uri) -> Response {
    let Some(value) = asset(uri.path()) else {
        return api_error(ErrorCode::NotFound);
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
    let security = state.security();
    let security = security.as_ref();
    let request_id = next_request_id();
    if header_size(request.headers()) > security.max_header_bytes {
        audit_security_event(&state, "headers_too_large", &request_id, &request);
        return secured(
            api_error(ErrorCode::HeadersTooLarge),
            security.https,
            &request_id,
        );
    }
    if request
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|value| value > security.max_body_bytes)
    {
        audit_security_event(&state, "body_too_large", &request_id, &request);
        return secured(
            api_error(ErrorCode::BodyTooLarge),
            security.https,
            &request_id,
        );
    }
    if !valid_host(request.headers(), &security.allowed_hosts) {
        audit_security_event(&state, "invalid_host", &request_id, &request);
        return secured(
            api_error(ErrorCode::InvalidHost),
            security.https,
            &request_id,
        );
    }
    if !valid_origin(request.headers(), &security.allowed_origins) {
        audit_security_event(&state, "invalid_origin", &request_id, &request);
        return secured(
            api_error(ErrorCode::InvalidOrigin),
            security.https,
            &request_id,
        );
    }
    let protected = is_protected(request.method(), request.uri().path());
    if request.method() != Method::OPTIONS
        && !within_rate_limit(&request, security, &state.rate_limiter)
    {
        state
            .metrics
            .rate_limited_total
            .fetch_add(1, Ordering::Relaxed);
        audit_security_event(&state, "rate_limited", &request_id, &request);
        let mut response = api_error(ErrorCode::RateLimited);
        response
            .headers_mut()
            .insert("retry-after", HeaderValue::from_static("60"));
        return secured(response, security.https, &request_id);
    }
    // Authenticate once, then authorize the route against that identity. Both
    // rejections look the same on the wire; the audit event distinguishes them.
    let mut identity = ClientIdentity::unauthenticated();
    if protected && request.method() != Method::OPTIONS {
        let today = today_iso();
        match authenticate(request.headers(), security, &today) {
            Authentication::Anonymous => {}
            Authentication::Client(client) => {
                let required =
                    required_scope(request.method(), request.uri().path()).unwrap_or(Scope::Admin);
                if !client.scopes.contains(&required) {
                    state
                        .metrics
                        .unauthorized_total
                        .fetch_add(1, Ordering::Relaxed);
                    audit_security_event_for_client(
                        &state,
                        "scope_denied",
                        &request_id,
                        &request,
                        &client.id,
                    );
                    return secured(
                        api_error(ErrorCode::ScopeDenied),
                        security.https,
                        &request_id,
                    );
                }
                identity = ClientIdentity {
                    id: client.id.clone(),
                    tools: client.tools.clone(),
                };
            }
            Authentication::Expired(id) => {
                state
                    .metrics
                    .unauthorized_total
                    .fetch_add(1, Ordering::Relaxed);
                audit_security_event_for_client(
                    &state,
                    "credential_expired",
                    &request_id,
                    &request,
                    &id,
                );
                let mut response = api_error(ErrorCode::Unauthorized);
                response
                    .headers_mut()
                    .insert(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
                return secured(response, security.https, &request_id);
            }
            Authentication::Rejected => {
                state
                    .metrics
                    .unauthorized_total
                    .fetch_add(1, Ordering::Relaxed);
                audit_security_event(&state, "unauthorized", &request_id, &request);
                let mut response = api_error(ErrorCode::Unauthorized);
                response
                    .headers_mut()
                    .insert(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
                return secured(response, security.https, &request_id);
            }
        }
    }
    let timeout = security.timeout;
    let https = security.https;
    let path = request.uri().path().to_owned();
    let client_ip = request_client_ip(&request);
    let mut request = request;
    request
        .extensions_mut()
        .insert(RequestId(request_id.clone()));
    let client_id = identity.id.clone();
    // MCP tools read this back out of `http::request::Parts` to enforce their
    // own per-tool policy.
    request.extensions_mut().insert(identity);
    let request_span = tracing::info_span!(
        target: "amatl::http",
        "http_request",
        request_id = %request_id,
        path = %path,
        client_ip = %client_ip,
        client_id = %client_id
    );
    let response =
        match tokio::time::timeout(timeout, next.run(request).instrument(request_span)).await {
            Ok(response) => response,
            Err(_) => {
                state
                    .metrics
                    .request_timeout_total
                    .fetch_add(1, Ordering::Relaxed);
                audit_security_event_context(
                    &state,
                    "request_timeout",
                    &request_id,
                    &path,
                    client_ip,
                    None,
                );
                api_error(ErrorCode::RequestTimeout)
            }
        };
    secured(response, https, &request_id)
}

fn audit_security_event(
    state: &AppState,
    event: &'static str,
    request_id: &str,
    request: &Request,
) {
    audit_security_event_context(
        state,
        event,
        request_id,
        request.uri().path(),
        request_client_ip(request),
        None,
    );
}

/// Audit an event that is attributable to an authenticated identity.
fn audit_security_event_for_client(
    state: &AppState,
    event: &'static str,
    request_id: &str,
    request: &Request,
    client_id: &str,
) {
    audit_security_event_context(
        state,
        event,
        request_id,
        request.uri().path(),
        request_client_ip(request),
        Some(client_id.to_owned()),
    );
}

/// Log the rejection and, when persistence is available, record it durably.
///
/// The log line stays authoritative: persistence is best effort and never
/// delays the response.
fn audit_security_event_context(
    state: &AppState,
    event: &'static str,
    request_id: &str,
    path: &str,
    client_ip: IpAddr,
    client_id: Option<String>,
) {
    tracing::warn!(
        target: "amatl::security",
        security_event = event,
        request_id,
        path,
        client_ip = %client_ip,
        client_id = client_id.as_deref().unwrap_or("-"),
        "HTTP security control rejected request"
    );
    state
        .service()
        .audit()
        .record(amatl_core::SecurityEventInput {
            event: event.to_owned(),
            request_id: Some(request_id.to_owned()),
            client_id,
            path: Some(path.to_owned()),
            client_ip: Some(client_ip.to_string()),
        });
}

pub(crate) fn next_request_id() -> String {
    let epoch_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{epoch_nanos:032x}-{sequence:016x}")
}

fn request_client_ip(request: &Request) -> IpAddr {
    request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|value| value.0.ip())
        .unwrap_or(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST))
}

fn secured(mut response: Response, https: bool, request_id: &str) -> Response {
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
    response.headers_mut().insert(
        REQUEST_ID_HEADER,
        HeaderValue::from_str(request_id).expect("generated request IDs are valid HTTP headers"),
    );
    response
}

fn valid_query(query: &str) -> bool {
    !query.trim().is_empty() && query.len() <= 2048
}

/// Every domain and operator surface requires the bearer token; only the UI
/// assets, `/health` and `/metrics` stay reachable without it.
fn is_protected(method: &Method, path: &str) -> bool {
    required_scope(method, path).is_some()
}

/// Outcome of matching a bearer token against the accepted credentials.
enum Authentication<'a> {
    /// Authentication is disabled; loopback development only.
    Anonymous,
    Client(&'a AuthorizedClient),
    /// The credential matched but its declared expiry has passed.
    Expired(String),
    Rejected,
}

/// Match the presented bearer against every accepted credential.
///
/// Comparison is over SHA-256 digests in constant time, and every credential is
/// checked so the answer does not leak which one nearly matched.
fn authenticate<'a>(
    headers: &HeaderMap,
    security: &'a SecurityState,
    today: &str,
) -> Authentication<'a> {
    if security.clients.is_empty() {
        return Authentication::Anonymous;
    }
    let Some(presented) = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
    else {
        return Authentication::Rejected;
    };
    let digest = token_digest(presented);
    let mut matched: Option<&AuthorizedClient> = None;
    for client in &security.clients {
        if constant_time_eq(&digest, &client.digest) {
            matched = Some(client);
        }
    }
    match matched {
        None => Authentication::Rejected,
        Some(client) if !unexpired(client, today) => Authentication::Expired(client.id.clone()),
        Some(client) => Authentication::Client(client),
    }
}

fn unexpired(client: &AuthorizedClient, today: &str) -> bool {
    amatl_core::ServerClient {
        expires_at: client.expires_at.clone(),
        ..Default::default()
    }
    .unexpired_on(today)
}

/// Current UTC day as `YYYY-MM-DD`, the granularity credentials expire at.
fn today_iso() -> String {
    let days = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        / 86_400;
    let (year, month, day) = civil_from_days(days as i64);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Days since the Unix epoch to a civil date (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
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

fn within_rate_limit(
    request: &Request,
    security: &SecurityState,
    rate_limiter: &Mutex<RateLimiter>,
) -> bool {
    let ip = request_client_ip(request);
    let now = Instant::now();
    let mut limiter = rate_limiter
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
        .allow_headers([AUTHORIZATION, CONTENT_TYPE, axum::http::header::ACCEPT])
        .expose_headers([HeaderName::from_static(REQUEST_ID_HEADER)]))
}

/// Render a domain failure with its catalog code and status.
fn service_error(error: ServiceError) -> Response {
    let code = error.code();
    if code.http_status() >= 500 {
        tracing::warn!(
            target: "amatl::http",
            error_code = code.as_str(),
            error = %error,
            "request failed"
        );
    }
    api_error(code)
}

/// Error body for a catalog code, using the code's own transport status.
fn api_error(code: ErrorCode) -> Response {
    api_error_with_status(
        StatusCode::from_u16(code.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        code,
    )
}

/// Error body for a catalog code with a transport-imposed status, used when the
/// framework already decided the status (for example a body rejection).
fn api_error_with_status(status: StatusCode, code: ErrorCode) -> Response {
    (
        status,
        Json(json!({
            "schema_version": SCHEMA_VERSION,
            "error": { "code": code.as_str(), "message": code.message() }
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests;
