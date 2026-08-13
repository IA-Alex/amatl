use crate::model::{
    ProviderCapabilities, ProviderError, ProviderErrorKind, ProviderExecutionStatus, ProviderItem,
    ProviderResult, SearchPlan, SCHEMA_VERSION,
};
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

mod brave;
mod duckduckgo;
mod http;
mod mojeek;

pub use brave::BraveProvider;
pub use duckduckgo::DuckDuckGoHtmlProvider;
pub use http::{HttpRequest, HttpResponse, HttpTransport, ReqwestTransport};
pub use mojeek::MojeekProvider;

#[derive(Clone, Debug)]
pub struct ProviderContext {
    pub timeout_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderAvailability {
    Available,
    Unavailable { code: String, message: String },
}

#[async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    fn capabilities(&self) -> ProviderCapabilities;
    fn availability(&self) -> ProviderAvailability {
        ProviderAvailability::Available
    }
    async fn search(
        &self,
        plan: &SearchPlan,
        context: &ProviderContext,
    ) -> Result<ProviderResult, ProviderError>;
}

#[derive(Clone, Debug)]
pub enum MockBehavior {
    Success(Vec<ProviderItem>),
    Partial(Vec<ProviderItem>, ProviderErrorKind),
    Failure(ProviderErrorKind),
    Delayed(Vec<ProviderItem>, u64),
}

pub struct MockProvider {
    name: String,
    behavior: MockBehavior,
    attempts: Arc<AtomicUsize>,
}

impl MockProvider {
    pub fn new(name: impl Into<String>, behavior: MockBehavior) -> Self {
        Self {
            name: name.into(),
            behavior,
            attempts: Arc::new(AtomicUsize::new(0)),
        }
    }
    pub fn success(name: impl Into<String>, results: Vec<ProviderItem>) -> Self {
        Self::new(name, MockBehavior::Success(results))
    }
    fn error(&self, kind: ProviderErrorKind) -> ProviderError {
        ProviderError {
            schema_version: SCHEMA_VERSION.into(),
            provider: self.name.clone(),
            kind,
            message: "mock provider failure".into(),
            retry_after_ms: None,
        }
    }
    pub fn attempts(&self) -> usize {
        self.attempts.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Provider for MockProvider {
    fn name(&self) -> &str {
        &self.name
    }
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            schema_version: SCHEMA_VERSION.into(),
            pagination: true,
            language: true,
            region: true,
            time_range: true,
            site_filter: true,
            file_filter: true,
            news: true,
            code: true,
            docs: true,
            academic: true,
            authentication: false,
            estimated_cost: Some(0),
        }
    }
    async fn search(
        &self,
        _plan: &SearchPlan,
        _context: &ProviderContext,
    ) -> Result<ProviderResult, ProviderError> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        let (status, results, errors) = match &self.behavior {
            MockBehavior::Success(results) => {
                (ProviderExecutionStatus::Success, results.clone(), vec![])
            }
            MockBehavior::Partial(results, kind) => (
                ProviderExecutionStatus::Partial,
                results.clone(),
                vec![self.error(kind.clone())],
            ),
            MockBehavior::Failure(kind) => return Err(self.error(kind.clone())),
            MockBehavior::Delayed(results, delay_ms) => {
                tokio::time::sleep(Duration::from_millis(*delay_ms)).await;
                (ProviderExecutionStatus::Success, results.clone(), vec![])
            }
        };
        Ok(ProviderResult {
            schema_version: SCHEMA_VERSION.into(),
            provider: self.name.clone(),
            status,
            results,
            accepted_filters: vec![],
            ignored_filters: vec![],
            approximated_filters: vec![],
            errors,
        })
    }
}
