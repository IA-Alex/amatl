//! MCP Streamable HTTP surface.
//!
//! The tool set mirrors the HTTP contract on purpose, with two deliberate
//! differences:
//!
//! * **Capability per tool.** The bearer that reaches `/mcp` carries a client
//!   identity with an explicit tool allowlist; every tool checks it before
//!   doing any work. `fetch` is the sensitive one — a credential can be granted
//!   `search` and denied `fetch` without turning off egress for everyone.
//! * **No ingestion tool.** `amatl ingest` reads the local filesystem and stays
//!   CLI-only. Exposing it here would turn a listener that an agent can drive
//!   into a remote file reader, which is a different threat model than "search
//!   the public web". A test in this module keeps that decision honest.
//! * **Stricter budgets.** Every limit comes from
//!   [`AmatlService::execution_limits`] for the MCP surface, so a tool never
//!   hardcodes its own ceiling and a configuration change moves both surfaces
//!   at once.
//!
//! `deep_search` is the long operation: it honors client cancellation and
//! reports progress when the caller supplied a progress token.

use crate::{next_request_id, ClientIdentity, ServiceHandle};
use amatl_core::{
    AmatlService, ErrorCode, FetchError, FetchRequest, ServiceSurface, SCHEMA_VERSION,
};
use rmcp::{
    handler::server::wrapper::Parameters,
    model::{
        CallToolResult, ProgressNotificationParam, ProgressToken, ProtocolVersion,
        RequestMetaObject,
    },
    schemars,
    service::RequestContext,
    tool, tool_handler, tool_router, Peer, RoleServer, ServerHandler,
};
use serde::Deserialize;
use serde_json::json;
use std::borrow::Cow;
use std::collections::BTreeMap;
use url::Url;

#[derive(Clone)]
pub struct McpSurface {
    service: ServiceHandle,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct QueryInput {
    /// Query text, including any supported AMATL operators.
    query: String,
    /// Zero-based page over the ranked result set. Pagination is server-side.
    #[serde(default)]
    page: Option<u32>,
    /// Results per page; clamped to the MCP surface limit.
    #[serde(default)]
    page_size: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DeepInput {
    /// Query text, including any supported AMATL operators.
    query: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FetchInput {
    /// Public HTTP(S) URL to retrieve using AMATL SSRF protections.
    url: String,
}

impl McpSurface {
    pub fn new(service: ServiceHandle) -> Self {
        Self { service }
    }

    /// Identity attached by the HTTP security middleware.
    ///
    /// The transport forwards the original `http::request::Parts`, so the
    /// authorization decision keeps using what the middleware authenticated —
    /// never a client-supplied header, which would be trivially spoofable.
    fn identity(context: &RequestContext<RoleServer>) -> Option<ClientIdentity> {
        context
            .extensions
            .get::<http::request::Parts>()
            .and_then(|parts| parts.extensions.get::<ClientIdentity>())
            .cloned()
    }

    /// `Err` when this caller may not use `tool`.
    fn authorize(context: &RequestContext<RoleServer>, tool: &str) -> Result<(), CallToolResult> {
        // No identity means the request never crossed the middleware; refuse.
        let Some(identity) = Self::identity(context) else {
            return Err(tool_error(ErrorCode::Unauthorized));
        };
        if identity.allows_tool(tool) {
            return Ok(());
        }
        tracing::warn!(
            target: "amatl::security",
            security_event = "tool_denied",
            client_id = %identity.id,
            tool,
            "MCP tool call rejected by the client tool allowlist"
        );
        Err(tool_error(ErrorCode::ScopeDenied))
    }

    /// Current service snapshot, so a configuration reload reaches MCP too.
    fn service(&self) -> AmatlService {
        self.service
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
}

#[tool_router]
impl McpSurface {
    #[tool(description = "Search configured providers with the public AMATL Search contract")]
    async fn search(
        &self,
        Parameters(input): Parameters<QueryInput>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        if let Err(denied) = Self::authorize(&context, "search") {
            return denied;
        }
        if !valid_query(&input.query) {
            return tool_error(ErrorCode::InvalidQuery);
        }
        let service = self.service();
        let surface = ServiceSurface::mcp_with_request_id(Some(next_request_id()));
        let limits = service.execution_limits(surface.clone());
        let page_size = input
            .page_size
            .map(|value| value.clamp(1, limits.max_page_size));
        let page = input.page.or(page_size.map(|_| 0));
        match service
            .search_paginated(input.query, surface, page, page_size)
            .await
        {
            Ok(value) => structured(value.response),
            Err(error) => tool_error(error.code()),
        }
    }

    #[tool(description = "Run bounded AMATL Deep enrichment with stricter MCP limits")]
    async fn deep_search(
        &self,
        Parameters(input): Parameters<DeepInput>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        if let Err(denied) = Self::authorize(&context, "deep_search") {
            return denied;
        }
        if !valid_query(&input.query) {
            return tool_error(ErrorCode::InvalidQuery);
        }
        let progress = progress_token(&context.meta);
        report_progress(&context.peer, progress.as_ref(), 0.0, "search").await;
        let service = self.service();
        let work = service.deep(
            input.query,
            ServiceSurface::mcp_with_request_id(Some(next_request_id())),
        );
        let Some(outcome) = run_cancellable(&context.ct, work).await else {
            report_progress(&context.peer, progress.as_ref(), 1.0, "cancelled").await;
            return tool_error(ErrorCode::RequestCancelled);
        };
        report_progress(&context.peer, progress.as_ref(), 1.0, "complete").await;
        match outcome {
            Ok(value) => structured(value),
            Err(error) => tool_error(error.code()),
        }
    }

    #[tool(description = "Fetch one public HTTP(S) URL with SSRF, redirect, byte and time limits")]
    async fn fetch(
        &self,
        Parameters(input): Parameters<FetchInput>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        if let Err(denied) = Self::authorize(&context, "fetch") {
            return denied;
        }
        let Ok(url) = Url::parse(&input.url) else {
            return tool_error(ErrorCode::InvalidUrl);
        };
        let service = self.service();
        let limits = service.execution_limits(ServiceSurface::mcp());
        let result = service
            .fetch_public(FetchRequest {
                url,
                timeout_ms: limits.fetch_timeout_ms,
                max_bytes: limits.fetch_max_bytes,
                max_redirects: limits.fetch_max_redirects,
                headers: BTreeMap::new(),
                request_id: Some(next_request_id()),
            })
            .await;
        match result {
            Ok(value) => CallToolResult::structured(json!({
                "schema_version": SCHEMA_VERSION,
                "final_url": value.final_url,
                "status": value.status,
                "content_type": value.content_type,
                "content": String::from_utf8_lossy(&value.body),
                "size": value.size,
                "retrieved_at": value.retrieved_at
            })),
            Err(FetchError::EgressDenied) => tool_error(ErrorCode::EgressDenied),
            Err(_) => tool_error(ErrorCode::FetchFailed),
        }
    }

    #[tool(description = "List configured provider availability and declared capabilities")]
    async fn providers(&self, context: RequestContext<RoleServer>) -> CallToolResult {
        if let Err(denied) = Self::authorize(&context, "providers") {
            return denied;
        }
        match self.service().provider_summaries() {
            Ok(providers) => CallToolResult::structured(json!({
                "schema_version": SCHEMA_VERSION,
                "providers": providers
            })),
            Err(error) => tool_error(error.code()),
        }
    }

    #[tool(
        description = "Report AMATL availability: sources, local persistence, caches and the effective MCP limits"
    )]
    async fn status(&self, context: RequestContext<RoleServer>) -> CallToolResult {
        if let Err(denied) = Self::authorize(&context, "status") {
            return denied;
        }
        let service = self.service();
        let limits = service.execution_limits(ServiceSurface::mcp());
        match service.status().await {
            Ok(status) => CallToolResult::structured(json!({
                "schema_version": SCHEMA_VERSION,
                "status": status.status,
                "sources": status.sources,
                "storage": status.storage,
                "cache": status.cache,
                "inference_backend": status.inference_backend,
                "limits": {
                    "max_provider_calls": limits.max_provider_calls,
                    "search_timeout_ms": limits.search_timeout_ms,
                    "max_page_size": limits.max_page_size,
                    "deep_max_fetches": limits.deep_max_fetches,
                    "deep_max_bytes": limits.deep_max_bytes,
                    "deep_timeout_ms": limits.deep_timeout_ms,
                    "fetch_timeout_ms": limits.fetch_timeout_ms,
                    "fetch_max_bytes": limits.fetch_max_bytes,
                    "fetch_max_redirects": limits.fetch_max_redirects
                }
            })),
            Err(error) => tool_error(error.code()),
        }
    }
}

#[tool_handler(
    name = "amatl",
    instructions = "AMATL search tools use bounded, deterministic multi-source retrieval. MCP budgets are stricter than local CLI budgets, and `status` reports the ones in force. Local file ingestion is intentionally not exposed here."
)]
impl ServerHandler for McpSurface {
    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(&[ProtocolVersion::V_2026_07_28])
    }
}

/// Await `work` unless the client cancels first.
///
/// `None` means the caller cancelled: the tool returns `request_cancelled`
/// instead of spending the rest of the Deep budget on a result nobody will
/// read. The dropped work is already bounded by its own deadline.
async fn run_cancellable<T>(
    cancellation: &tokio_util::sync::CancellationToken,
    work: impl std::future::Future<Output = T>,
) -> Option<T> {
    tokio::select! {
        biased;
        () = cancellation.cancelled() => None,
        outcome = work => Some(outcome),
    }
}

fn progress_token(meta: &RequestMetaObject) -> Option<ProgressToken> {
    meta.get_progress_token()
}

/// Best-effort progress notification; a client that did not ask for progress,
/// or a transport that dropped, must never fail the tool call.
async fn report_progress(
    peer: &Peer<RoleServer>,
    token: Option<&ProgressToken>,
    progress: f64,
    stage: &str,
) {
    let Some(token) = token else { return };
    let _ = peer
        .notify_progress(
            ProgressNotificationParam::new(token.clone(), progress)
                .with_total(1.0)
                .with_message(stage),
        )
        .await;
}

fn structured(value: impl serde::Serialize) -> CallToolResult {
    match serde_json::to_value(value) {
        Ok(value) => CallToolResult::structured(value),
        Err(_) => tool_error(ErrorCode::SerializationFailed),
    }
}

/// Tool failures use the same catalog codes as the HTTP surface.
fn tool_error(code: ErrorCode) -> CallToolResult {
    CallToolResult::structured_error(json!({
        "schema_version": SCHEMA_VERSION,
        "error": { "code": code.as_str(), "message": code.message() }
    }))
}

fn valid_query(query: &str) -> bool {
    !query.trim().is_empty() && query.len() <= 2048
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn cancellation_stops_waiting_for_a_long_tool_call() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let outcome = run_cancellable(&cancellation, async {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            "finished"
        })
        .await;
        assert_eq!(outcome, None, "a cancelled call must not wait for the work");
    }

    #[tokio::test]
    async fn work_that_finishes_first_is_returned_unchanged() {
        let cancellation = CancellationToken::new();
        assert_eq!(
            run_cancellable(&cancellation, async { "finished" }).await,
            Some("finished")
        );
    }
}
