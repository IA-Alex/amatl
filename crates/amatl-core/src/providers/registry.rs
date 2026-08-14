//! Provider registry: the extension point for search sources.
//!
//! A source is declared in configuration (`[providers.<name>]`, the governance
//! record) and implemented by a [`ProviderFactory`] registered here. Adding a
//! source therefore means shipping a factory and a governance record — no
//! `match` arm inside the service, no new field in the configuration struct.

use super::{BraveProvider, DuckDuckGoHtmlProvider, HttpTransport, MojeekProvider, Provider};
use crate::config::ProviderRuntimeConfig;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Everything a factory may use to build one provider instance.
pub struct ProviderBuildContext<'a> {
    /// Registered name, identical to the configuration key.
    pub name: &'a str,
    /// Governance record for this provider.
    pub runtime: &'a ProviderRuntimeConfig,
    /// Whether the provider is in `providers.enabled`.
    pub enabled: bool,
    /// Whether the governance record is complete and unexpired.
    pub approved: bool,
    /// Credential resolved from `credential_env`, when present and non-empty.
    pub credential: Option<String>,
    /// Shared outbound transport, already subject to the data policy.
    pub transport: Arc<dyn HttpTransport>,
}

/// Builds one kind of provider.
pub trait ProviderFactory: Send + Sync {
    /// Registry key; must match the configuration key exactly.
    fn name(&self) -> &str;
    /// Whether `provider-canary` may drive this provider over the network.
    fn supports_network_canary(&self) -> bool {
        true
    }
    /// Whether a credential is required before the provider can be used.
    fn requires_credential(&self) -> bool {
        true
    }
    /// Build an instance for the supplied context.
    fn build(&self, context: &ProviderBuildContext<'_>) -> Arc<dyn Provider>;
}

/// Name-indexed set of provider factories.
#[derive(Clone, Default)]
pub struct ProviderRegistry {
    factories: BTreeMap<String, Arc<dyn ProviderFactory>>,
}

impl ProviderRegistry {
    /// Registry with no implementations; useful for tests and embedders that
    /// want full control over the available sources.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Registry with the sources AMATL ships.
    pub fn builtin() -> Self {
        Self::empty()
            .with(Arc::new(BraveFactory))
            .with(Arc::new(MojeekFactory))
            .with(Arc::new(DuckDuckGoHtmlFactory))
    }

    /// Builder-style registration, replacing any factory with the same name.
    pub fn with(mut self, factory: Arc<dyn ProviderFactory>) -> Self {
        self.register(factory);
        self
    }

    /// Register a factory, returning the one it replaced.
    pub fn register(
        &mut self,
        factory: Arc<dyn ProviderFactory>,
    ) -> Option<Arc<dyn ProviderFactory>> {
        self.factories.insert(factory.name().to_owned(), factory)
    }

    /// Remove a factory, returning it when one was registered.
    ///
    /// Removing a factory does not disable the source on its own: the
    /// configuration still declares it, and the service reports
    /// `provider_not_registered` until a factory is registered again.
    pub fn unregister(&mut self, name: &str) -> Option<Arc<dyn ProviderFactory>> {
        self.factories.remove(name)
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn ProviderFactory>> {
        self.factories.get(name)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.factories.contains_key(name)
    }

    /// Registered names in stable alphabetical order.
    pub fn names(&self) -> Vec<&str> {
        self.factories.keys().map(String::as_str).collect()
    }
}

struct BraveFactory;

impl ProviderFactory for BraveFactory {
    fn name(&self) -> &str {
        "brave"
    }

    fn build(&self, context: &ProviderBuildContext<'_>) -> Arc<dyn Provider> {
        Arc::new(BraveProvider::new(
            context.credential.clone(),
            context.enabled,
            context.approved,
            context.transport.clone(),
        ))
    }
}

struct MojeekFactory;

impl ProviderFactory for MojeekFactory {
    fn name(&self) -> &str {
        "mojeek"
    }

    fn build(&self, context: &ProviderBuildContext<'_>) -> Arc<dyn Provider> {
        Arc::new(MojeekProvider::new(
            context.credential.clone(),
            context.enabled,
            context.approved,
            context.runtime.supported_filters.clone(),
            context.transport.clone(),
        ))
    }
}

struct DuckDuckGoHtmlFactory;

impl ProviderFactory for DuckDuckGoHtmlFactory {
    fn name(&self) -> &str {
        "duckduckgo_html"
    }

    fn supports_network_canary(&self) -> bool {
        false
    }

    fn requires_credential(&self) -> bool {
        false
    }

    fn build(&self, _context: &ProviderBuildContext<'_>) -> Arc<dyn Provider> {
        Arc::new(DuckDuckGoHtmlProvider::blocked())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        ProviderCapabilities, ProviderError, ProviderErrorKind, ProviderResult, SearchPlan,
        SCHEMA_VERSION,
    };
    use crate::providers::{ProviderAvailability, ProviderContext};
    use async_trait::async_trait;

    struct CustomProvider;

    #[async_trait]
    impl Provider for CustomProvider {
        fn name(&self) -> &str {
            "custom_archive"
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
                docs: true,
                academic: false,
                authentication: false,
                estimated_cost: Some(0),
            }
        }
        fn availability(&self) -> ProviderAvailability {
            ProviderAvailability::Available
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
                message: "test provider".into(),
                retry_after_ms: None,
            })
        }
    }

    struct CustomFactory;

    impl ProviderFactory for CustomFactory {
        fn name(&self) -> &str {
            "custom_archive"
        }
        fn build(&self, _context: &ProviderBuildContext<'_>) -> Arc<dyn Provider> {
            Arc::new(CustomProvider)
        }
    }

    #[test]
    fn builtin_registry_exposes_the_shipped_sources() {
        let registry = ProviderRegistry::builtin();
        assert_eq!(registry.names(), vec!["brave", "duckduckgo_html", "mojeek"]);
        assert!(!registry
            .get("duckduckgo_html")
            .unwrap()
            .supports_network_canary());
        assert!(registry.get("brave").unwrap().requires_credential());
    }

    #[test]
    fn third_party_sources_register_without_touching_the_builtin_set() {
        let registry = ProviderRegistry::builtin().with(Arc::new(CustomFactory));
        assert!(registry.contains("custom_archive"));
        assert!(registry.contains("brave"));
        assert!(ProviderRegistry::empty().names().is_empty());

        // A source can also be withdrawn without rebuilding the registry.
        let mut registry = registry;
        assert!(registry.unregister("custom_archive").is_some());
        assert!(!registry.contains("custom_archive"));
        assert!(registry.unregister("custom_archive").is_none());
        assert!(registry.contains("brave"));
    }
}
