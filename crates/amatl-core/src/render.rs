use crate::config::RendererConfig;
use crate::model::FinalUrl;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, TryAcquireError};
use url::Url;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenderResult {
    pub final_url: FinalUrl,
    pub dom: Vec<u8>,
    pub redirects: u32,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum RenderError {
    #[error("renderer_unavailable")]
    Unavailable,
    #[error("renderer_timeout")]
    Timeout,
    #[error("renderer_blocked")]
    Blocked,
    #[error("renderer_failed")]
    Failed,
    #[error("renderer_at_capacity")]
    AtCapacity,
}

#[async_trait]
pub trait Renderer: Send + Sync {
    fn available(&self) -> bool;
    async fn render(&self, url: &Url) -> Result<RenderResult, RenderError>;
}

/// Safe base capability: Chromium is not launched until a CDP backend can prove
/// sandbox, network, memory, navigation and cleanup enforcement.
#[derive(Clone, Debug)]
pub struct ChromiumRenderer {
    available: bool,
    reason: String,
}

impl ChromiumRenderer {
    pub fn detect(config: &RendererConfig) -> Self {
        if !config.enabled {
            return Self {
                available: false,
                reason: "disabled by default".into(),
            };
        }
        Self {
            available: false,
            reason: "required CDP isolation and resource enforcement are not installed".into(),
        }
    }

    pub fn unavailable_reason(&self) -> &str {
        &self.reason
    }
}

#[async_trait]
impl Renderer for ChromiumRenderer {
    fn available(&self) -> bool {
        self.available
    }
    async fn render(&self, _: &Url) -> Result<RenderResult, RenderError> {
        Err(RenderError::Unavailable)
    }
}

/// Concurrency-controlled pool that wraps a single [`Renderer`] instance so it
/// is created once and reused across deep requests instead of being
/// re-detected on every call.
///
/// The pool enforces a global cap on concurrent browser renderings via a
/// [`Semaphore`]; callers that would exceed the cap receive
/// [`RenderError::AtCapacity`] immediately instead of queuing unboundedly.
#[derive(Clone)]
pub struct RendererPool {
    inner: Arc<dyn Renderer>,
    semaphore: Arc<Semaphore>,
}

impl RendererPool {
    /// Wrap `renderer` and allow at most `max_concurrent` simultaneous
    /// renderings.
    pub fn new(renderer: Arc<dyn Renderer>, max_concurrent: usize) -> Self {
        let max_concurrent = max_concurrent.max(1);
        Self {
            inner: renderer,
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
        }
    }

    /// Whether the underlying renderer reports itself as available.
    pub fn available(&self) -> bool {
        self.inner.available()
    }

    /// Acquire a concurrency slot and render `url`. Returns
    /// [`RenderError::AtCapacity`] when all slots are occupied.
    pub async fn render(&self, url: &Url) -> Result<RenderResult, RenderError> {
        let _permit = self.acquire().await?;
        self.inner.render(url).await
    }

    async fn acquire(&self) -> Result<OwnedSemaphorePermit, RenderError> {
        Arc::clone(&self.semaphore)
            .try_acquire_owned()
            .map_err(|error| match error {
                TryAcquireError::Closed => RenderError::Unavailable,
                TryAcquireError::NoPermits => RenderError::AtCapacity,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn renderer_has_no_unsafe_fallback() {
        let config = RendererConfig {
            enabled: true,
            ..RendererConfig::default()
        };
        let renderer = ChromiumRenderer::detect(&config);
        assert!(!renderer.available());
        assert_eq!(
            renderer
                .render(&Url::parse("https://example.com").unwrap())
                .await,
            Err(RenderError::Unavailable)
        );
    }

    #[tokio::test]
    async fn pool_enforces_concurrency_limit() {
        let renderer = Arc::new(ChromiumRenderer::detect(&RendererConfig::default()));
        let pool = RendererPool::new(renderer, 1);
        let url = Url::parse("https://example.com").unwrap();

        // First call acquires the only permit.
        let handle = {
            let pool = pool.clone();
            let _url = url.clone();
            tokio::spawn(async move {
                let _permit = pool.acquire().await.unwrap();
                // Hold the permit indefinitely.
                std::future::pending::<()>().await;
            })
        };

        // Give the spawned task time to acquire.
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        // Second call should be rejected.
        assert_eq!(pool.render(&url).await, Err(RenderError::AtCapacity));

        handle.abort();
    }

    #[tokio::test]
    async fn pool_available_delegates_to_inner() {
        let renderer = Arc::new(ChromiumRenderer::detect(&RendererConfig::default()));
        let pool = RendererPool::new(renderer, 2);
        assert!(!pool.available());
    }
}
