use crate::config::RendererConfig;
use crate::model::FinalUrl;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
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
}
