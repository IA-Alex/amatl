//! Shared error-code catalog for every AMATL surface (CLI, API, MCP, UI).
//!
//! Surfaces must never invent their own wire codes: they translate a domain
//! error into an [`ErrorCode`] and render it with the catalog's stable string,
//! transport status and neutral message. This keeps the public contract
//! identical across surfaces and makes the taxonomy reviewable in one place.

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Malformed request envelope (body, parameters or encoding).
    InvalidRequest,
    /// Request envelope was valid but the query text is not usable.
    InvalidQuery,
    /// Supplied URL is not a usable public HTTP(S) URL.
    InvalidUrl,
    /// Missing or non-matching credential on a protected route.
    Unauthorized,
    /// `Host` header is absent or outside the configured allow list.
    InvalidHost,
    /// `Origin` header is outside the configured allow list.
    InvalidOrigin,
    /// Per-client request budget for the current window is exhausted.
    RateLimited,
    /// Request body exceeds the configured limit.
    BodyTooLarge,
    /// Request headers exceed the configured limit.
    HeadersTooLarge,
    /// Handler exceeded the configured request deadline.
    RequestTimeout,
    /// No route or asset matches the request path.
    NotFound,
    /// Data policy denies outbound network access for this operation.
    EgressDenied,
    /// Outbound fetch failed or was rejected by the safety controls.
    FetchFailed,
    /// Enabled provider has no governance record in the configuration.
    ProviderNotDeclared,
    /// Declared provider has no implementation in the provider registry.
    ProviderNotRegistered,
    /// Provider is declared and registered but absent from `providers.enabled`.
    ProviderNotEnabled,
    /// Provider governance approval is incomplete, expired or rejected.
    ProviderNotApproved,
    /// Provider credential environment variable is missing or empty.
    ProviderCredentialMissing,
    /// Provider has no authorized real-network canary.
    ProviderNetworkBlocked,
    /// Search planning produced no plan for the request.
    SearchPlanningFailed,
    /// Configuration is internally inconsistent for the requested operation.
    ConfigurationInvalid,
    /// Ranking policy requires an inference backend that is not available.
    InferenceUnavailable,
    /// Optional ranking backend rejected or failed the request.
    RankingBackendUnavailable,
    /// Persistent storage is unavailable; degraded operation continues.
    StorageUnavailable,
    /// Credential is valid but lacks the capability this surface requires.
    ScopeDenied,
    /// Caller cancelled the request before it completed.
    RequestCancelled,
    /// Response could not be serialized for the surface.
    SerializationFailed,
    /// Unclassified internal failure.
    InternalError,
}

/// Every code in the catalog, in declaration order.
pub const ERROR_CATALOG: [ErrorCode; 28] = [
    ErrorCode::InvalidRequest,
    ErrorCode::InvalidQuery,
    ErrorCode::InvalidUrl,
    ErrorCode::Unauthorized,
    ErrorCode::InvalidHost,
    ErrorCode::InvalidOrigin,
    ErrorCode::RateLimited,
    ErrorCode::BodyTooLarge,
    ErrorCode::HeadersTooLarge,
    ErrorCode::RequestTimeout,
    ErrorCode::NotFound,
    ErrorCode::EgressDenied,
    ErrorCode::FetchFailed,
    ErrorCode::ProviderNotDeclared,
    ErrorCode::ProviderNotRegistered,
    ErrorCode::ProviderNotEnabled,
    ErrorCode::ProviderNotApproved,
    ErrorCode::ProviderCredentialMissing,
    ErrorCode::ProviderNetworkBlocked,
    ErrorCode::SearchPlanningFailed,
    ErrorCode::ConfigurationInvalid,
    ErrorCode::InferenceUnavailable,
    ErrorCode::RankingBackendUnavailable,
    ErrorCode::StorageUnavailable,
    ErrorCode::ScopeDenied,
    ErrorCode::RequestCancelled,
    ErrorCode::SerializationFailed,
    ErrorCode::InternalError,
];

impl ErrorCode {
    /// Stable wire identifier. Never change an existing string.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::InvalidQuery => "invalid_query",
            Self::InvalidUrl => "invalid_url",
            Self::Unauthorized => "unauthorized",
            Self::InvalidHost => "invalid_host",
            Self::InvalidOrigin => "invalid_origin",
            Self::RateLimited => "rate_limited",
            Self::BodyTooLarge => "body_too_large",
            Self::HeadersTooLarge => "headers_too_large",
            Self::RequestTimeout => "request_timeout",
            Self::NotFound => "not_found",
            Self::EgressDenied => "egress_denied",
            Self::FetchFailed => "fetch_failed",
            Self::ProviderNotDeclared => "provider_not_declared",
            Self::ProviderNotRegistered => "provider_not_registered",
            Self::ProviderNotEnabled => "provider_not_enabled",
            Self::ProviderNotApproved => "provider_not_approved",
            Self::ProviderCredentialMissing => "provider_credential_missing",
            Self::ProviderNetworkBlocked => "provider_network_blocked",
            Self::SearchPlanningFailed => "search_planning_failed",
            Self::ConfigurationInvalid => "configuration_invalid",
            Self::InferenceUnavailable => "inference_unavailable",
            Self::RankingBackendUnavailable => "ranking_backend_unavailable",
            Self::StorageUnavailable => "storage_unavailable",
            Self::ScopeDenied => "scope_denied",
            Self::RequestCancelled => "request_cancelled",
            Self::SerializationFailed => "serialization_failed",
            Self::InternalError => "internal_error",
        }
    }

    /// HTTP status used by transport surfaces. MCP tools ignore it.
    pub const fn http_status(self) -> u16 {
        match self {
            Self::InvalidRequest | Self::InvalidQuery | Self::InvalidUrl | Self::InvalidHost => 400,
            Self::Unauthorized => 401,
            Self::InvalidOrigin
            | Self::EgressDenied
            | Self::ProviderNetworkBlocked
            | Self::ScopeDenied => 403,
            Self::NotFound => 404,
            // 499 is the conventional "client closed request"; MCP renders it
            // as a tool error rather than a transport status.
            Self::RequestCancelled => 499,
            Self::BodyTooLarge => 413,
            Self::HeadersTooLarge => 431,
            Self::RateLimited => 429,
            Self::FetchFailed => 502,
            Self::StorageUnavailable => 503,
            Self::RequestTimeout => 504,
            Self::ProviderNotDeclared
            | Self::ProviderNotRegistered
            | Self::ProviderNotEnabled
            | Self::ProviderNotApproved
            | Self::ProviderCredentialMissing
            | Self::SearchPlanningFailed
            | Self::ConfigurationInvalid
            | Self::InferenceUnavailable
            | Self::RankingBackendUnavailable
            | Self::SerializationFailed
            | Self::InternalError => 500,
        }
    }

    /// Neutral operator-facing message. Never embeds request data.
    pub const fn message(self) -> &'static str {
        match self {
            Self::InvalidRequest => "request envelope is malformed",
            Self::InvalidQuery => "query is empty or exceeds the accepted length",
            Self::InvalidUrl => "URL is not a usable public HTTP(S) URL",
            Self::Unauthorized => "credential is missing or does not match",
            Self::InvalidHost => "Host header is not in the allow list",
            Self::InvalidOrigin => "Origin header is not in the allow list",
            Self::RateLimited => "request budget for this window is exhausted",
            Self::BodyTooLarge => "request body exceeds the configured limit",
            Self::HeadersTooLarge => "request headers exceed the configured limit",
            Self::RequestTimeout => "request exceeded the configured deadline",
            Self::NotFound => "no route or asset matches this path",
            Self::EgressDenied => "data policy denies outbound network access",
            Self::FetchFailed => "outbound fetch failed or was rejected by safety controls",
            Self::ProviderNotDeclared => "enabled provider has no governance record",
            Self::ProviderNotRegistered => "declared provider has no registered implementation",
            Self::ProviderNotEnabled => "provider is not enabled",
            Self::ProviderNotApproved => "provider governance approval is incomplete or expired",
            Self::ProviderCredentialMissing => "provider credential is missing or empty",
            Self::ProviderNetworkBlocked => "provider has no authorized real-network canary",
            Self::SearchPlanningFailed => "search planning produced no plan",
            Self::ConfigurationInvalid => "configuration is invalid for this operation",
            Self::InferenceUnavailable => "required inference backend is unavailable",
            Self::RankingBackendUnavailable => "optional ranking backend is unavailable",
            Self::StorageUnavailable => "persistent storage is unavailable",
            Self::ScopeDenied => "credential lacks the required capability",
            Self::RequestCancelled => "request was cancelled by the caller",
            Self::SerializationFailed => "response could not be serialized",
            Self::InternalError => "internal failure",
        }
    }

    /// Parse a wire identifier back into a catalog entry.
    pub fn from_wire(value: &str) -> Option<Self> {
        ERROR_CATALOG
            .into_iter()
            .find(|code| code.as_str() == value)
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn wire_identifiers_are_unique_and_snake_case() {
        let mut seen = BTreeSet::new();
        for code in ERROR_CATALOG {
            let value = code.as_str();
            assert!(seen.insert(value), "duplicated error code: {value}");
            assert!(
                value
                    .chars()
                    .all(|character| character.is_ascii_lowercase() || character == '_'),
                "non snake_case error code: {value}"
            );
            assert_eq!(ErrorCode::from_wire(value), Some(code));
        }
    }

    #[test]
    fn every_code_maps_to_a_client_or_server_status_and_a_message() {
        for code in ERROR_CATALOG {
            assert!(
                (400..=599).contains(&code.http_status()),
                "{code} has a non-error status"
            );
            assert!(!code.message().is_empty());
        }
    }

    #[test]
    fn serialization_uses_the_wire_identifier() {
        assert_eq!(
            serde_json::to_string(&ErrorCode::SearchPlanningFailed).unwrap(),
            "\"search_planning_failed\""
        );
    }
}
