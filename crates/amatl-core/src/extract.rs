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

    pub async fn probe_version(&self) -> Result<String, ExtractError> {
        let operation = async {
            let mut child = Command::new(&self.executable)
                .arg("--version")
                .stdin(Stdio::null())
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
            let stdout = child.stdout.take().ok_or(ExtractError::Failed)?;
            let mut limited = stdout.take(257);
            let mut output = Vec::new();
            limited
                .read_to_end(&mut output)
                .await
                .map_err(|_| ExtractError::Failed)?;
            let status = child.wait().await.map_err(|_| ExtractError::Failed)?;
            if !status.success() || output.len() > 256 {
                return Err(ExtractError::InvalidOutput);
            }
            let version = String::from_utf8(output)
                .map_err(|_| ExtractError::InvalidOutput)?
                .trim()
                .to_owned();
            if version.is_empty() || version.chars().any(char::is_control) {
                return Err(ExtractError::InvalidOutput);
            }
            Ok(version)
        };
        tokio::time::timeout(Duration::from_millis(self.timeout_ms.min(2_000)), operation)
            .await
            .map_err(|_| ExtractError::Timeout)?
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
                .args(["--json", "--with-metadata", "--no-comments", "--no-tables"])
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
            let write = async move {
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
        let metadata = [
            "hostname",
            "language",
            "license",
            "pagetype",
            "source",
            "source-hostname",
            "excerpt",
            "categories",
            "tags",
        ]
        .into_iter()
        .filter_map(|name| {
            value
                .get(name)
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| {
                    (
                        format!("trafilatura_{name}"),
                        value.chars().take(512).collect(),
                    )
                })
        })
        .collect();
        Ok(ExtractionResult {
            content,
            format: "text".into(),
            title: string("title"),
            author: string("author"),
            published_at: string("date"),
            metadata,
            extractor_used: self.version.clone(),
            status: "success".into(),
        })
    }
}

/// Native, dependency-free HTML extractor used as a fallback when the optional
/// Trafilatura CLI is unavailable.
///
/// It is deliberately conservative and deterministic: it reads the title,
/// author and publication date from standard metadata, then collects the main
/// readable text from `<article>`/`<main>`/`<body>` while dropping executable,
/// navigational and decorative elements. It never spawns a process, so it works
/// under `isolated`/`deny` data policies and cannot be controlled by hostile
/// HTML the way an external extractor could.
#[derive(Clone, Debug)]
pub struct NativeHtmlExtractor {
    max_output_bytes: u64,
}

impl NativeHtmlExtractor {
    pub fn new(max_output_bytes: u64) -> Self {
        Self { max_output_bytes }
    }
}

/// Stable identity of the native extractor, used in cache keys.
pub const NATIVE_EXTRACTOR_VERSION: &str = "native-html-v1";

#[async_trait]
impl Extractor for NativeHtmlExtractor {
    fn name(&self) -> &str {
        "native-html"
    }
    fn version(&self) -> &str {
        NATIVE_EXTRACTOR_VERSION
    }

    async fn extract(&self, html: &[u8]) -> Result<ExtractionResult, ExtractError> {
        let input = std::str::from_utf8(html).map_err(|_| ExtractError::InvalidOutput)?;
        let document = scraper::Html::parse_document(input);
        let title = meta_property(&document, "og:title")
            .or_else(|| meta_name(&document, "title"))
            .or_else(|| {
                scraper::Selector::parse("title")
                    .ok()
                    .and_then(|selector| document.select(&selector).next())
                    .map(|element| element.text().collect::<String>())
            })
            .map(|value| value.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|value| !value.is_empty());
        let author = meta_name(&document, "author")
            .or_else(|| meta_property(&document, "article:author"))
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        let published_at = meta_property(&document, "article:published_time")
            .or_else(|| meta_name(&document, "date"))
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        let content = extract_readable_text(&document);
        if content.is_empty() {
            return Err(ExtractError::InvalidOutput);
        }
        if content.len() as u64 > self.max_output_bytes {
            return Err(ExtractError::OutputLimit);
        }
        let mut metadata = BTreeMap::new();
        if let Some(language) = html_lang(&document) {
            metadata.insert("native_language".into(), language);
        }
        Ok(ExtractionResult {
            content,
            format: "text".into(),
            title,
            author,
            published_at,
            metadata,
            extractor_used: self.version().into(),
            status: "success".into(),
        })
    }
}

/// Extractor that tries a primary backend (Trafilatura) and transparently falls
/// back to the native HTML extractor when the primary is unavailable or fails.
///
/// This keeps Deep from degrading to a superficial document whenever the
/// optional CLI is missing: the native extractor always provides readable text.
#[derive(Clone, Debug)]
pub struct FallbackExtractor {
    primary: TrafilaturaExtractor,
    native: NativeHtmlExtractor,
    /// Composite identity covering both members; see [`Extractor::version`].
    version: String,
}

impl FallbackExtractor {
    pub fn new(primary: TrafilaturaExtractor, native: NativeHtmlExtractor) -> Self {
        let version = format!(
            "fallback-v1({}+{})",
            primary.version(),
            NATIVE_EXTRACTOR_VERSION
        );
        Self {
            primary,
            native,
            version,
        }
    }
}

#[async_trait]
impl Extractor for FallbackExtractor {
    fn name(&self) -> &str {
        "fallback"
    }

    /// Identity of *this* extractor, not of its primary.
    ///
    /// Deep keys the document cache on this string. Returning the primary's
    /// version made natively extracted content indistinguishable from
    /// Trafilatura output: installing or removing the optional CLI would then
    /// serve documents produced by one algorithm under the other's key.
    fn version(&self) -> &str {
        &self.version
    }

    async fn extract(&self, html: &[u8]) -> Result<ExtractionResult, ExtractError> {
        match self.primary.extract(html).await {
            Ok(result) => Ok(result),
            Err(ExtractError::Unavailable | ExtractError::Failed | ExtractError::Timeout) => {
                self.native.extract(html).await
            }
            Err(error) => Err(error),
        }
    }
}

fn meta_name(document: &scraper::Html, name: &str) -> Option<String> {
    let selector = scraper::Selector::parse(&format!("meta[name=\"{name}\"]")).ok()?;
    document
        .select(&selector)
        .next()
        .and_then(|element| element.value().attr("content").map(str::to_owned))
}

fn meta_property(document: &scraper::Html, property: &str) -> Option<String> {
    let selector = scraper::Selector::parse(&format!("meta[property=\"{property}\"]")).ok()?;
    document
        .select(&selector)
        .next()
        .and_then(|element| element.value().attr("content").map(str::to_owned))
}

fn html_lang(document: &scraper::Html) -> Option<String> {
    let selector = scraper::Selector::parse("html").ok()?;
    document
        .select(&selector)
        .next()
        .and_then(|element| element.value().attr("lang").map(str::to_owned))
}

fn extract_readable_text(document: &scraper::Html) -> String {
    let mut parts = Vec::new();
    for tag in ["article", "main", "body"] {
        if let Ok(selector) = scraper::Selector::parse(tag) {
            if let Some(root) = document.select(&selector).next() {
                collect_readable_text(root, &mut parts);
                break;
            }
        }
    }
    parts.join("\n")
}

/// Deepest DOM nesting the native extractor descends into.
///
/// html5ever imposes no nesting limit of its own, so hostile markup — and
/// `"<div>".repeat(n)` is only five bytes per level, well inside the fetch size
/// budget — would drive a recursive walk past the stack guard. That aborts the
/// process outright rather than unwinding, so it cannot be caught. Content
/// nested deeper than this bound is skipped instead.
const MAX_EXTRACTION_DEPTH: usize = 256;

fn collect_readable_text(root: scraper::ElementRef, parts: &mut Vec<String>) {
    // Iterative depth-first walk over an explicit stack. Children are pushed in
    // reverse so they pop in document order, matching the previous recursion.
    let mut stack = Vec::new();
    for child in root.children().rev() {
        stack.push((child, 1_usize));
    }
    while let Some((node, depth)) = stack.pop() {
        let Some(element) = scraper::ElementRef::wrap(node) else {
            if let Some(text) = node.value().as_text() {
                let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
                if !normalized.is_empty() {
                    parts.push(normalized);
                }
            }
            continue;
        };
        if matches!(
            element.value().name(),
            "script"
                | "style"
                | "noscript"
                | "template"
                | "nav"
                | "footer"
                | "header"
                | "aside"
                | "form"
                | "iframe"
                | "svg"
        ) {
            continue;
        }
        if depth >= MAX_EXTRACTION_DEPTH {
            continue;
        }
        for child in element.children().rev() {
            stack.push((child, depth + 1));
        }
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
    async fn fallback_version_is_distinct_from_its_primary() {
        // Deep keys the document cache on `version()`. If the fallback reported
        // the primary's version, natively extracted documents would be served
        // later as if Trafilatura had produced them.
        let primary = TrafilaturaExtractor::new("trafilatura".into(), "v2.2.0".into(), 100, 1024);
        let fallback = FallbackExtractor::new(primary.clone(), NativeHtmlExtractor::new(1024));
        assert_ne!(fallback.version(), primary.version());
        assert_ne!(fallback.version(), NATIVE_EXTRACTOR_VERSION);
        assert!(fallback.version().contains(primary.version()));
        assert!(fallback.version().contains(NATIVE_EXTRACTOR_VERSION));

        // Changing the primary's version must change the composite identity,
        // so a cache built under one configuration is not reused under another.
        let other = FallbackExtractor::new(
            TrafilaturaExtractor::new("trafilatura".into(), "v2.3.0".into(), 100, 1024),
            NativeHtmlExtractor::new(1024),
        );
        assert_ne!(fallback.version(), other.version());
    }

    #[tokio::test]
    async fn deeply_nested_html_does_not_overflow_the_stack() {
        // A recursive walk aborts the process here (stack overflow is not an
        // unwinding panic), so reaching the assertion at all is the point.
        // Deep enough that a recursive walk would exhaust a test thread's 2 MiB
        // stack, but shallow enough that parsing stays cheap in CI.
        let depth = 12_000;
        let mut html = String::from("<html><body>");
        html.push_str(&"<div>".repeat(depth));
        html.push_str("deep text");
        html.push_str(&"</div>".repeat(depth));
        html.push_str("</body></html>");

        let extractor = NativeHtmlExtractor::new(1024 * 1024);
        let result = extractor.extract(html.as_bytes()).await;
        assert!(
            result.is_ok() || result == Err(ExtractError::InvalidOutput),
            "unexpected outcome: {result:?}"
        );
    }

    #[tokio::test]
    async fn readable_text_keeps_document_order() {
        let extractor = NativeHtmlExtractor::new(1024 * 1024);
        let extracted = extractor
            .extract(b"<html><body><p>first</p><div><p>second</p><p>third</p></div><p>fourth</p></body></html>")
            .await
            .unwrap();
        let positions: Vec<usize> = ["first", "second", "third", "fourth"]
            .iter()
            .map(|needle| extracted.content.find(needle).expect("token present"))
            .collect();
        let mut sorted = positions.clone();
        sorted.sort_unstable();
        assert_eq!(positions, sorted, "traversal must preserve document order");
    }

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

    #[cfg(unix)]
    #[tokio::test]
    async fn fixed_cli_contract_requests_metadata_without_network_arguments() {
        let path = std::env::temp_dir().join(format!(
            "amatl-recording-extractor-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(
            &path,
            br###"#!/bin/sh
test "$1" = "--json"
test "$2" = "--with-metadata"
test "$3" = "--no-comments"
test "$4" = "--no-tables"
test "$#" = "4"
cat >/dev/null
printf '%s' '{"text":"main text","title":"Title","hostname":"example.com"}'
"###,
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let extractor = TrafilaturaExtractor::new(
            path.to_string_lossy().into_owned(),
            "test".into(),
            500,
            4096,
        );
        let result = extractor.extract(b"<main>main text</main>").await.unwrap();
        let _ = std::fs::remove_file(path);
        assert_eq!(result.content, "main text");
        assert_eq!(
            result
                .metadata
                .get("trafilatura_hostname")
                .map(String::as_str),
            Some("example.com")
        );
    }

    #[tokio::test]
    async fn native_extractor_reads_metadata_and_main_content() {
        let extractor = NativeHtmlExtractor::new(4096);
        let html = r#"
            <!doctype html>
            <html lang="es">
              <head>
                <title>   Título   de prueba  </title>
                <meta property="og:title" content="Título de prueba">
                <meta name="author" content="Ana García">
                <meta property="article:published_time" content="2024-05-01T10:00:00Z">
              </head>
              <body>
                <nav><a href="/">Home</a></nav>
                <article>
                  <h1>Encabezado</h1>
                  <p>Primer   párrafo   con   espacios.</p>
                  <script>alert("no");</script>
                  <p>Segundo párrafo.</p>
                </article>
                <footer>Pie de página</footer>
              </body>
            </html>
        "#;
        let result = extractor.extract(html.as_bytes()).await.unwrap();
        assert_eq!(result.title.as_deref(), Some("Título de prueba"));
        assert_eq!(result.author.as_deref(), Some("Ana García"));
        assert_eq!(result.published_at.as_deref(), Some("2024-05-01T10:00:00Z"));
        assert_eq!(
            result.metadata.get("native_language").map(String::as_str),
            Some("es")
        );
        assert!(result.content.contains("Primer párrafo con espacios."));
        assert!(result.content.contains("Segundo párrafo."));
        assert!(!result.content.contains("alert"));
        assert!(!result.content.contains("Pie de página"));
        assert!(!result.content.contains("Home"));
    }

    #[tokio::test]
    async fn native_extractor_rejects_empty_or_oversized_documents() {
        let extractor = NativeHtmlExtractor::new(64);
        assert_eq!(
            extractor
                .extract(b"<html><body><script>x</script></body></html>")
                .await,
            Err(ExtractError::InvalidOutput)
        );
        let big = format!("<p>{}</p>", "x".repeat(128));
        assert_eq!(
            extractor.extract(big.as_bytes()).await,
            Err(ExtractError::OutputLimit)
        );
    }

    #[tokio::test]
    async fn fallback_extractor_uses_native_when_primary_is_unavailable() {
        let primary = TrafilaturaExtractor::new(
            "amatl-certainly-missing-trafilatura".into(),
            "v1".into(),
            100,
            4096,
        );
        let fallback = FallbackExtractor::new(primary, NativeHtmlExtractor::new(4096));
        let html = b"<html><head><title>Fallback</title></head><body><article><p>Texto nativo</p></article></body></html>";
        let result = fallback.extract(html).await.unwrap();
        assert_eq!(result.title.as_deref(), Some("Fallback"));
        assert!(result.content.contains("Texto nativo"));
        assert_eq!(result.extractor_used, "native-html-v1");
    }
}
