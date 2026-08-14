//! Optional JavaScript rendering for Deep.
//!
//! The renderer executes scripts over markup that has **already been
//! retrieved**; it is never given a URL and never performs network I/O. That
//! is the whole safety argument, and it is enforced by the signature of
//! [`Renderer::render`] rather than by convention: [`crate::SafeFetcher`]
//! remains the single owner of network egress, so a browser cannot be turned
//! into a second, unaudited fetch path.
//!
//! Chromium runs exclusively through the `amatl-chromium-sandbox` harness,
//! which confines it with `bwrap --unshare-all` (an empty network namespace,
//! so loopback and the internet are both unreachable) under a `systemd-run`
//! scope bounding memory, task count and wall-clock time. The harness is
//! verified independently by the `chromium-isolation` workflow.

use crate::config::RendererConfig;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, TryAcquireError};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenderResult {
    /// Serialized DOM after scripts have run.
    pub dom: Vec<u8>,
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
    /// Execute scripts over `html` and return the resulting DOM.
    ///
    /// Takes bytes, never a URL: the renderer must not be able to reach the
    /// network, and a signature that cannot express a navigation makes that
    /// structural instead of a rule someone has to remember.
    async fn render(&self, html: &[u8]) -> Result<RenderResult, RenderError>;
}

/// Chromium driven through the `amatl-chromium-sandbox` isolation harness.
///
/// Availability is *proved*, not assumed: the harness, `bwrap`, `systemd-run`
/// and a Chromium binary must all be present, and the platform must be Linux.
/// Anything missing leaves the renderer unavailable with a reason, and Deep
/// keeps the superficial document rather than falling back to an unconfined
/// browser.
#[derive(Clone, Debug)]
pub struct ChromiumRenderer {
    available: bool,
    reason: String,
    sandbox: std::path::PathBuf,
    timeout_ms: u64,
    shutdown_grace_ms: u64,
    max_memory_mb: u64,
    max_dom_bytes: u64,
}

impl ChromiumRenderer {
    pub fn detect(config: &RendererConfig) -> Self {
        let unavailable = |reason: String| Self {
            available: false,
            reason,
            sandbox: std::path::PathBuf::new(),
            timeout_ms: config.timeout_ms,
            shutdown_grace_ms: config.shutdown_grace_ms,
            max_memory_mb: config.max_memory_mb,
            max_dom_bytes: config.max_dom_bytes,
        };

        if !config.enabled {
            return unavailable("disabled by default".into());
        }
        if !cfg!(target_os = "linux") {
            // The confinement relies on user namespaces and systemd scopes.
            return unavailable("the isolation harness requires Linux".into());
        }
        let Some(sandbox) = resolve_executable(&config.sandbox_path) else {
            return unavailable(format!(
                "isolation harness not found: {}",
                config.sandbox_path
            ));
        };
        for tool in ["bwrap", "systemd-run"] {
            if resolve_executable(tool).is_none() {
                return unavailable(format!("required isolation tool is unavailable: {tool}"));
            }
        }
        if !["chromium", "chromium-browser", "google-chrome"]
            .iter()
            .any(|candidate| resolve_executable(candidate).is_some())
            && std::env::var_os("AMATL_CHROMIUM_BIN").is_none()
        {
            return unavailable("Chromium executable is unavailable".into());
        }

        Self {
            available: true,
            reason: String::new(),
            sandbox,
            timeout_ms: config.timeout_ms,
            shutdown_grace_ms: config.shutdown_grace_ms,
            max_memory_mb: config.max_memory_mb,
            max_dom_bytes: config.max_dom_bytes,
        }
    }

    pub fn unavailable_reason(&self) -> &str {
        &self.reason
    }
}

/// A private directory removed on drop, used to hand markup to the harness.
///
/// Created with `0700` on Unix so the staged page and the rendered DOM are not
/// readable by other users on a shared host.
struct ScratchDirectory(std::path::PathBuf);

impl ScratchDirectory {
    fn create() -> Option<Self> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("amatl-render-{}-{nanos}", std::process::id()));
        std::fs::create_dir(&path).ok()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700));
        }
        Some(Self(path))
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for ScratchDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Resolve `name` to an executable path, honoring `PATH` for bare names.
fn resolve_executable(name: &str) -> Option<std::path::PathBuf> {
    let candidate = std::path::Path::new(name);
    if candidate.is_absolute() || name.contains(std::path::MAIN_SEPARATOR) {
        return candidate.is_file().then(|| candidate.to_path_buf());
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|directory| directory.join(name))
            .find(|path| path.is_file())
    })
}

#[async_trait]
impl Renderer for ChromiumRenderer {
    fn available(&self) -> bool {
        self.available
    }

    async fn render(&self, html: &[u8]) -> Result<RenderResult, RenderError> {
        if !self.available {
            return Err(RenderError::Unavailable);
        }
        if html.is_empty() {
            return Err(RenderError::Failed);
        }

        // The harness reads and writes files, so the markup is staged in a
        // private directory that is removed however this function exits.
        let workspace = ScratchDirectory::create().ok_or(RenderError::Failed)?;
        let input = workspace.path().join("input.html");
        let output = workspace.path().join("dom.html");
        tokio::fs::write(&input, html)
            .await
            .map_err(|_| RenderError::Failed)?;

        // The harness takes whole seconds and enforces the deadline itself via
        // `RuntimeMaxSec`; the outer timeout below is the backstop.
        let harness_timeout_secs = self.timeout_ms.div_ceil(1_000).max(1);
        let mut command = tokio::process::Command::new(&self.sandbox);
        command
            .arg(&input)
            .arg(&output)
            .arg(harness_timeout_secs.to_string())
            .env("AMATL_CHROMIUM_MEMORY_MB", self.max_memory_mb.to_string())
            .env(
                "AMATL_CHROMIUM_MAX_DOM_BYTES",
                self.max_dom_bytes.to_string(),
            )
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        let deadline = std::time::Duration::from_millis(
            self.timeout_ms.saturating_add(self.shutdown_grace_ms),
        );
        let outcome = tokio::time::timeout(deadline, command.output()).await;

        let output_bytes = match outcome {
            Err(_) => return Err(RenderError::Timeout),
            Ok(Err(_)) => return Err(RenderError::Failed),
            Ok(Ok(result)) => {
                if !result.status.success() {
                    // 69 is the harness's "a required capability is missing",
                    // which is a configuration problem rather than a page that
                    // failed to render.
                    return Err(match result.status.code() {
                        Some(69) => RenderError::Unavailable,
                        _ => RenderError::Failed,
                    });
                }
                tokio::fs::read(&output)
                    .await
                    .map_err(|_| RenderError::Failed)?
            }
        };

        if output_bytes.is_empty() || output_bytes.len() as u64 > self.max_dom_bytes {
            return Err(RenderError::Blocked);
        }
        Ok(RenderResult { dom: output_bytes })
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

    /// Acquire a concurrency slot and render `html`. Returns
    /// [`RenderError::AtCapacity`] when all slots are occupied.
    pub async fn render(&self, html: &[u8]) -> Result<RenderResult, RenderError> {
        let _permit = self.acquire().await?;
        self.inner.render(html).await
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

    /// Enabling the renderer must never be enough on its own: without the
    /// isolation harness present, Chromium stays unlaunched.
    #[tokio::test]
    async fn renderer_has_no_unsafe_fallback() {
        let config = RendererConfig {
            enabled: true,
            sandbox_path: "amatl-certainly-missing-sandbox-harness".into(),
            ..RendererConfig::default()
        };
        let renderer = ChromiumRenderer::detect(&config);
        assert!(
            !renderer.available(),
            "a missing harness must not yield an available renderer"
        );
        assert!(renderer.unavailable_reason().contains("harness"));
        assert_eq!(
            renderer.render(b"<html><body>x</body></html>").await,
            Err(RenderError::Unavailable)
        );
    }

    #[tokio::test]
    async fn renderer_is_disabled_by_default() {
        let renderer = ChromiumRenderer::detect(&RendererConfig::default());
        assert!(!renderer.available());
        assert_eq!(renderer.unavailable_reason(), "disabled by default");
    }

    /// The renderer's safety argument is that it cannot reach the network.
    /// `render` therefore accepts bytes and `RenderResult` carries only a DOM:
    /// there is no way to express a navigation, and no way for the browser to
    /// report a `final_url` that differs from what `SafeFetcher` resolved.
    #[test]
    fn the_render_contract_cannot_express_a_navigation() {
        let source = include_str!("render.rs");
        let contract_end = source
            .find("#[cfg(test)]")
            .expect("the test module delimits the contract");
        let contract = &source[..contract_end];
        assert!(
            !contract.contains("url::Url"),
            "the renderer must not name a URL type"
        );
        assert!(
            !contract.contains("final_url"),
            "the renderer must not produce a final_url; SafeFetcher owns it"
        );
    }

    #[tokio::test]
    async fn pool_enforces_concurrency_limit() {
        let renderer = Arc::new(ChromiumRenderer::detect(&RendererConfig::default()));
        let pool = RendererPool::new(renderer, 1);

        // First call acquires the only permit.
        let handle = {
            let pool = pool.clone();
            tokio::spawn(async move {
                let _permit = pool.acquire().await.unwrap();
                // Hold the permit indefinitely.
                std::future::pending::<()>().await;
            })
        };

        // Give the spawned task time to acquire.
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        // Second call should be rejected.
        assert_eq!(
            pool.render(b"<html></html>").await,
            Err(RenderError::AtCapacity)
        );

        handle.abort();
    }

    #[tokio::test]
    async fn pool_available_delegates_to_inner() {
        let renderer = Arc::new(ChromiumRenderer::detect(&RendererConfig::default()));
        let pool = RendererPool::new(renderer, 2);
        assert!(!pool.available());
    }
}
