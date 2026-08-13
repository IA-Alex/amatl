use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::process::Stdio;
use std::time::Duration;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExtractionResult {
    pub content: String,
    pub format: String,
    pub title: Option<String>,
    pub author: Option<String>,
    pub published_at: Option<String>,
    pub metadata: BTreeMap<String, String>,
    pub extractor_used: String,
    pub status: String,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ExtractError {
    #[error("extractor unavailable")]
    Unavailable,
    #[error("extractor timed out")]
    Timeout,
    #[error("extractor output exceeded limit")]
    OutputLimit,
    #[error("extractor failed")]
    Failed,
    #[error("extractor returned invalid output")]
    InvalidOutput,
}

#[async_trait]
pub trait Extractor: Send + Sync {
    fn name(&self) -> &str;
    fn version(&self) -> &str;
    async fn extract(&self, html: &[u8]) -> Result<ExtractionResult, ExtractError>;
}

#[derive(Clone, Debug)]
pub struct TrafilaturaExtractor {
    executable: String,
    version: String,
    timeout_ms: u64,
    max_output_bytes: u64,
}

impl TrafilaturaExtractor {
    pub fn new(
        executable: String,
        version: String,
        timeout_ms: u64,
        max_output_bytes: u64,
    ) -> Self {
        Self {
            executable,
            version,
            timeout_ms,
            max_output_bytes,
        }
    }
}

#[async_trait]
impl Extractor for TrafilaturaExtractor {
    fn name(&self) -> &str {
        "trafilatura"
    }
    fn version(&self) -> &str {
        &self.version
    }

    async fn extract(&self, html: &[u8]) -> Result<ExtractionResult, ExtractError> {
        let mut child = Command::new(&self.executable)
            .args(["--json", "--no-comments", "--no-tables"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    ExtractError::Unavailable
                } else {
                    ExtractError::Failed
                }
            })?;
        let mut stdin = child.stdin.take().ok_or(ExtractError::Failed)?;
        stdin
            .write_all(html)
            .await
            .map_err(|_| ExtractError::Failed)?;
        drop(stdin);
        let stdout = child.stdout.take().ok_or(ExtractError::Failed)?;
        let mut limited = stdout.take(self.max_output_bytes.saturating_add(1));
        let mut output = Vec::new();
        let operation = async {
            limited
                .read_to_end(&mut output)
                .await
                .map_err(|_| ExtractError::Failed)?;
            child.wait().await.map_err(|_| ExtractError::Failed)
        };
        let status =
            match tokio::time::timeout(Duration::from_millis(self.timeout_ms), operation).await {
                Ok(result) => result?,
                Err(_) => {
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    return Err(ExtractError::Timeout);
                }
            };
        if output.len() as u64 > self.max_output_bytes {
            return Err(ExtractError::OutputLimit);
        }
        if !status.success() {
            return Err(ExtractError::Failed);
        }
        let value: serde_json::Value =
            serde_json::from_slice(&output).map_err(|_| ExtractError::InvalidOutput)?;
        let content = value
            .get("text")
            .or_else(|| value.get("raw_text"))
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .trim()
            .to_string();
        if content.is_empty() {
            return Err(ExtractError::InvalidOutput);
        }
        let string = |name: &str| {
            value
                .get(name)
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        };
        Ok(ExtractionResult {
            content,
            format: "text".into(),
            title: string("title"),
            author: string("author"),
            published_at: string("date"),
            metadata: BTreeMap::new(),
            extractor_used: self.version.clone(),
            status: "success".into(),
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct UnavailableExtractor;

#[async_trait]
impl Extractor for UnavailableExtractor {
    fn name(&self) -> &str {
        "unavailable"
    }
    fn version(&self) -> &str {
        "unavailable"
    }
    async fn extract(&self, _: &[u8]) -> Result<ExtractionResult, ExtractError> {
        Err(ExtractError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn missing_trafilatura_is_a_typed_optional_failure() {
        let extractor = TrafilaturaExtractor::new(
            "amatl-certainly-missing-trafilatura".into(),
            "v1".into(),
            100,
            1024,
        );
        assert_eq!(
            extractor.extract(b"<p>x</p>").await,
            Err(ExtractError::Unavailable)
        );
    }
}
