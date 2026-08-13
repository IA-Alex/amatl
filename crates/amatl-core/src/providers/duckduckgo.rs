use super::{Provider, ProviderAvailability, ProviderContext};
use crate::model::{
    ProviderCapabilities, ProviderError, ProviderErrorKind, ProviderResult, SearchPlan,
    SCHEMA_VERSION,
};
use async_trait::async_trait;

pub struct DuckDuckGoHtmlProvider;

impl DuckDuckGoHtmlProvider {
    pub fn blocked() -> Self {
        Self
    }
}

#[async_trait]
impl Provider for DuckDuckGoHtmlProvider {
    fn name(&self) -> &str {
        "duckduckgo_html"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            schema_version: SCHEMA_VERSION.into(),
            pagination: false,
            language: false,
            region: false,
            time_range: false,
            site_filter: false,
            file_filter: false,
            news: false,
            code: false,
            docs: false,
            academic: false,
            authentication: false,
            estimated_cost: Some(0),
        }
    }

    fn availability(&self) -> ProviderAvailability {
        ProviderAvailability::Unavailable {
            code: "provider_pending_explicit_approval".into(),
            message: "DuckDuckGo HTML remains disabled pending verifiable authorization".into(),
        }
    }

    async fn search(
        &self,
        _plan: &SearchPlan,
        _context: &ProviderContext,
    ) -> Result<ProviderResult, ProviderError> {
        Err(ProviderError {
            schema_version: SCHEMA_VERSION.into(),
            provider: self.name().into(),
            kind: ProviderErrorKind::Unavailable,
            message: "provider is blocked pending explicit approval".into(),
            retry_after_ms: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remains_blocked_without_explicit_authorization() {
        assert!(matches!(
            DuckDuckGoHtmlProvider::blocked().availability(),
            ProviderAvailability::Unavailable { .. }
        ));
    }
}
