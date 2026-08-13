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
        let operation = async {
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
            let stdout = child.stdout.take().ok_or(ExtractError::Failed)?;
            let mut limited = stdout.take(self.max_output_bytes.saturating_add(1));
            let mut output = Vec::new();
            let write = async {
                stdin
                    .write_all(html)
                    .await
                    .map_err(|_| ExtractError::Failed)?;
                stdin.shutdown().await.map_err(|_| ExtractError::Failed)
            };
            let read = async {
                limited
                    .read_to_end(&mut output)
                    .await
                    .map_err(|_| ExtractError::Failed)
            };
            let (write_result, read_result) = tokio::join!(write, read);
            write_result?;
            read_result?;
            let status = child.wait().await.map_err(|_| ExtractError::Failed)?;
            Ok::<_, ExtractError>((status, output))
        };
        let (status, output) =
            tokio::time::timeout(Duration::from_millis(self.timeout_ms), operation)
                .await
                .map_err(|_| ExtractError::Timeout)??;
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
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::time::Instant;

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

    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_covers_a_child_that_never_drains_stdin() {
        let path = std::env::temp_dir().join(format!(
            "amatl-blocked-extractor-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, b"#!/bin/sh\nsleep 5\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let extractor =
            TrafilaturaExtractor::new(path.to_string_lossy().into_owned(), "test".into(), 30, 1024);
        let started = Instant::now();
        let result = extractor.extract(&vec![b'x'; 1024 * 1024]).await;
        let _ = std::fs::remove_file(path);
        assert_eq!(result, Err(ExtractError::Timeout));
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
