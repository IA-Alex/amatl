use crate::next_request_id;
use amatl_core::{
    AmatlService, ErrorCode, FetchError, FetchRequest, ServiceSurface, SCHEMA_VERSION,
};
use rmcp::{
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ProtocolVersion},
    schemars, tool, tool_handler, tool_router, ServerHandler,
};
use serde::Deserialize;
use serde_json::json;
use std::borrow::Cow;
use std::collections::BTreeMap;
use url::Url;

#[derive(Clone)]
pub struct McpSurface {
    service: AmatlService,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct QueryInput {
    /// Query text, including any supported AMATL operators.
    query: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FetchInput {
    /// Public HTTP(S) URL to retrieve using AMATL SSRF protections.
    url: String,
}

impl McpSurface {
    pub fn new(service: AmatlService) -> Self {
        Self { service }
    }
}

#[tool_router]
impl McpSurface {
    #[tool(description = "Search configured providers with the public AMATL Search contract")]
    async fn search(&self, Parameters(input): Parameters<QueryInput>) -> CallToolResult {
        if !valid_query(&input.query) {
            return tool_error(ErrorCode::InvalidQuery);
        }
        match self
            .service
            .search(
                input.query,
                ServiceSurface::mcp_with_request_id(Some(next_request_id())),
            )
            .await
        {
            Ok(value) => structured(value.response),
            Err(error) => tool_error(error.code()),
        }
    }

    #[tool(description = "Run bounded AMATL Deep enrichment with stricter MCP limits")]
    async fn deep_search(&self, Parameters(input): Parameters<QueryInput>) -> CallToolResult {
        if !valid_query(&input.query) {
            return tool_error(ErrorCode::InvalidQuery);
        }
        match self
            .service
            .deep(
                input.query,
                ServiceSurface::mcp_with_request_id(Some(next_request_id())),
            )
            .await
        {
            Ok(value) => structured(value),
            Err(error) => tool_error(error.code()),
        }
    }

    #[tool(description = "Fetch one public HTTP(S) URL with SSRF, redirect, byte and time limits")]
    async fn fetch(&self, Parameters(input): Parameters<FetchInput>) -> CallToolResult {
        let Ok(url) = Url::parse(&input.url) else {
            return tool_error(ErrorCode::InvalidUrl);
        };
        let result = self
            .service
            .fetch_public(FetchRequest {
                url,
                timeout_ms: 3_000,
                max_bytes: 256 * 1024,
                max_redirects: 2,
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
    async fn providers(&self) -> CallToolResult {
        match self.service.provider_summaries() {
            Ok(providers) => CallToolResult::structured(json!({
                "schema_version": SCHEMA_VERSION,
                "providers": providers
            })),
            Err(error) => tool_error(error.code()),
        }
    }
}

#[tool_handler(
    name = "amatl",
    instructions = "AMATL search tools use bounded, deterministic multi-source retrieval. MCP budgets are stricter than local CLI budgets."
)]
impl ServerHandler for McpSurface {
    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(&[ProtocolVersion::V_2026_07_28])
    }
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
