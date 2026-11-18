//! Provider registry: the extension point for search sources.
//!
//! A source is declared in configuration (`[providers.<name>]`, the governance
//! record) and implemented by a [`ProviderFactory`] registered here. Adding a
//! source therefore means shipping a factory and a governance record — no
//! `match` arm inside the service, no new field in the configuration struct.

use super::{HttpTransport, MarginaliaProvider, Provider};
use crate::config::ProviderRuntimeConfig;
use std::collections::BTreeMap;
use std::sync::Arc;
use zeroize::Zeroizing;

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
    /// Credential resolved from an operator-injected in-memory value or from
    /// `credential_env`, when present and non-empty. `Zeroizing` so the copy a
    /// provider keeps is wiped from the heap when the provider is dropped.
    pub credential: Option<Zeroizing<String>>,
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
    ///
    /// Exactly one source is registered: the approved, key-gated Marginalia.
    /// No other source ships an adapter or a factory, so any other name
    /// resolves to `provider_not_registered` — it can never be selected or send
    /// traffic. Configuration validation rejects unknown names outright
    /// (`config/mod.rs`), so the registry is the second of two closed doors.
    pub fn builtin() -> Self {
        Self::empty().with(Arc::new(MarginaliaFactory))
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

// ---------------------------------------------------------------------------
// Marginalia factory
// ---------------------------------------------------------------------------

pub struct MarginaliaFactory;

impl ProviderFactory for MarginaliaFactory {
    fn name(&self) -> &str {
        "marginalia"
    }

    fn requires_credential(&self) -> bool {
        true
    }

    fn build(&self, context: &ProviderBuildContext<'_>) -> Arc<dyn Provider> {
        Arc::new(MarginaliaProvider::new(
            context.credential.clone(),
            context.enabled,
            context.approved,
            context.transport.clone(),
        ))
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
        // Only the one sanctioned source ships; nothing else is registered or
        // declared, so nothing else can be selected.
        assert_eq!(registry.names(), vec!["marginalia"]);
        assert!(registry.get("marginalia").is_some());
        assert!(registry.get("marginalia").unwrap().requires_credential());
    }

    #[test]
    fn third_party_sources_register_without_touching_the_builtin_set() {
        let registry = ProviderRegistry::builtin().with(Arc::new(CustomFactory));
        assert!(registry.contains("custom_archive"));
        assert!(ProviderRegistry::empty().names().is_empty());

        // A source can also be withdrawn without rebuilding the registry.
        let mut registry = registry;
        assert!(registry.unregister("custom_archive").is_some());
        assert!(!registry.contains("custom_archive"));
        assert!(registry.unregister("custom_archive").is_none());
    }
}
