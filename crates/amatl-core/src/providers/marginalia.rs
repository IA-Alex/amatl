//! Marginalia Search API provider — scaffold for future implementation.
//!
//! Marginalia is an independent search engine focused on non-commercial content
//! and text-heavy websites. It offers a public API at
//! <https://api.marginalia.nu/>.
//!
//! # Status
//!
//! This is a **scaffold** — the factory is registered and the governance record
//! exists, but the provider always returns `Unavailable`. To activate it:
//!
//! 1. Obtain a Marginalia API key and set `MARGINALIA_API_KEY` in the
//!    environment.
//! 2. Complete the governance record in `config.rs` (reviewer, reviewed_at,
//!    terms_version_or_date, rate_limit, cost_model, data_handling_notes,
//!    operational_risk).
//! 3. Implement the `search()` method: construct the API request, parse the
//!    JSON response, and map results to `ProviderItem`.
//! 4. Add contract tests for filters, errors, rate limits, and invalid
//!    responses.
//! 5. Set `approval_status = "approved"` and add `"marginalia"` to
//!    `providers.enabled`.
//!
//! # API (reference)
//!
//! ```text
//! GET https://api.marginalia.nu/search?q=<query>&count=<n>
//! Authorization: Bearer <api_key>
//! ```
//!
//! Response shape (approximate):
//! ```json
//! {
//!   "results": [
//!     { "title": "...", "url": "...", "description": "...", "domain": "..." }
//!   ]
//! }
//! ```

use super::{
    HttpTransport, Provider, ProviderAvailability, ProviderContext,
};
use crate::model::{
    ProviderCapabilities, ProviderError, ProviderErrorKind, ProviderResult, SearchPlan,
    SCHEMA_VERSION,
};
use async_trait::async_trait;
use std::sync::Arc;

/// Placeholder endpoint — update when the actual API URL is confirmed.
const ENDPOINT: &str = "https://api.marginalia.nu/search";

pub struct MarginaliaProvider {
    api_key: Option<String>,
    enabled: bool,
    approved: bool,
    transport: Arc<dyn HttpTransport>,
}

impl MarginaliaProvider {
    #[allow(dead_code)]
    pub fn new(
        api_key: Option<String>,
        enabled: bool,
        approved: bool,
        transport: Arc<dyn HttpTransport>,
    ) -> Self {
        Self {
            api_key,
            enabled,
            approved,
            transport,
        }
    }

    /// Always returns an error — scaffold until the adapter is implemented.
    fn not_implemented() -> ProviderError {
        ProviderError {
            schema_version: SCHEMA_VERSION.into(),
            provider: "marginalia".into(),
            kind: ProviderErrorKind::Unavailable,
            message: "Marginalia adapter is not yet implemented".into(),
            retry_after_ms: None,
        }
    }
}

#[async_trait]
impl Provider for MarginaliaProvider {
    fn name(&self) -> &str {
        "marginalia"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            schema_version: SCHEMA_VERSION.into(),
            pagination: true,
            language: false,
            region: false,
            time_range: false,
            site_filter: true,
            file_filter: false,
            news: false,
            code: false,
            docs: false,
            academic: false,
            authentication: true,
            estimated_cost: None, // TBD — requires plan verification
        }
    }

    fn availability(&self) -> ProviderAvailability {
        if !self.enabled {
            ProviderAvailability::Unavailable {
                code: "provider_disabled".into(),
                message: "Marginalia is not enabled in the configuration".into(),
            }
        } else if !self.approved {
            ProviderAvailability::Unavailable {
                code: "provider_not_approved".into(),
                message: "Marginalia governance record is incomplete or expired".into(),
            }
        } else if self.api_key.is_none() {
            ProviderAvailability::Unavailable {
                code: "credential_missing".into(),
                message: "MARGINALIA_API_KEY is not set".into(),
            }
        } else {
            // Scaffold: always unavailable until search() is implemented.
            ProviderAvailability::Unavailable {
                code: "provider_pending_explicit_approval".into(),
                message: "Marginalia adapter is a scaffold; search() is not implemented".into(),
            }
        }
    }

    async fn search(
        &self,
        _plan: &SearchPlan,
        _context: &ProviderContext,
    ) -> Result<ProviderResult, ProviderError> {
        Err(Self::not_implemented())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn availability_is_unavailable_as_scaffold() {
        let provider = MarginaliaProvider::new(
            Some("test-key".into()),
            true,
            true,
            Arc::new(super::super::http::ReqwestTransport::new(1024).unwrap()),
        );
        assert!(!matches!(
            provider.availability(),
            ProviderAvailability::Available
        ));
    }
}
