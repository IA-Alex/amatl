use crate::evidence::analyze_evidence_bundle_optional;
use crate::{
    CanonicalUrl, DataPolicyConfig, Document, DocumentStatus, Evidence, EvidenceV2, FetchMethod,
    FinalUrl, OriginalUrl, Query, SCHEMA_VERSION,
};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};
use thiserror::Error;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

pub const LOCAL_INGEST_MAX_INPUT_BYTES: u64 = 20 * 1024 * 1024;
pub const LOCAL_INGEST_MAX_OUTPUT_BYTES: u64 = 8 * 1024 * 1024;
pub const LOCAL_INGEST_PDF_TIMEOUT_MS: u64 = 8_000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LocalDocumentType {
    PlainText,
    Markdown,
    Html,
    Json,
    JsonLines,
    Csv,
    SourceCode,
    Pdf,
}

impl LocalDocumentType {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::PlainText => "plain_text",
            Self::Markdown => "markdown",
            Self::Html => "html",
            Self::Json => "json",
            Self::JsonLines => "json_lines",
            Self::Csv => "csv",
            Self::SourceCode => "source_code",
            Self::Pdf => "pdf",
        }
    }

    pub const fn media_type(&self) -> &'static str {
        match self {
            Self::PlainText => "text/plain",
            Self::Markdown => "text/markdown",
            Self::Html => "text/html",
            Self::Json => "application/json",
            Self::JsonLines => "application/x-ndjson",
            Self::Csv => "text/csv",
            Self::SourceCode => "text/plain",
            Self::Pdf => "application/pdf",
        }
    }

    const fn extractor_version(&self) -> &'static str {
        match self {
            Self::PlainText => "local-plain-text-v1",
            Self::Markdown => "local-markdown-v1",
            Self::Html => "local-html-v1",
            Self::Json => "local-json-v1",
            Self::JsonLines => "local-json-lines-v1",
            Self::Csv => "local-csv-v1",
            Self::SourceCode => "local-source-code-v1",
            Self::Pdf => "local-pdftotext-v1",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LocalIngestResponse {
    pub schema_version: String,
    pub query: Option<String>,
    pub document_type: LocalDocumentType,
    pub document: Document,
    pub evidence: Evidence,
    pub evidence_v2: EvidenceV2,
    pub elapsed_ms: u64,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum LocalIngestError {
    #[error("local source is unavailable")]
    SourceUnavailable,
    #[error("local source is not a regular file")]
    NotAFile,
    #[error("local source exceeded the input limit")]
    InputLimit,
    #[error("document type is unsupported")]
    UnsupportedDocumentType,
    #[error("document encoding must be UTF-8")]
    InvalidTextEncoding,
    #[error("document content does not match its type")]
    InvalidDocument,
    #[error("document contains no extractable text")]
    EmptyDocument,
    #[error("extracted document exceeded the output limit")]
    OutputLimit,
    #[error("external PDF extraction is denied by the active data policy")]
    ExternalExtractorDenied,
    #[error("PDF extractor is unavailable")]
    PdfExtractorUnavailable,
    #[error("PDF extraction timed out")]
    PdfExtractorTimeout,
    #[error("PDF extraction failed")]
    PdfExtractorFailed,
    #[error("local source cannot be represented as a file URI")]
    InvalidSourceUri,
}

#[derive(Clone, Debug)]
pub struct LocalIngestor {
    max_input_bytes: u64,
    max_output_bytes: u64,
    pdf_timeout_ms: u64,
    pdf_executable: PathBuf,
    allow_external_processes: bool,
}

impl LocalIngestor {
    pub fn new(data_policy: &DataPolicyConfig) -> Self {
        Self {
            max_input_bytes: LOCAL_INGEST_MAX_INPUT_BYTES,
            max_output_bytes: LOCAL_INGEST_MAX_OUTPUT_BYTES,
            pdf_timeout_ms: LOCAL_INGEST_PDF_TIMEOUT_MS,
            pdf_executable: PathBuf::from("pdftotext"),
            allow_external_processes: data_policy.allows_network_egress(),
        }
    }

    #[cfg(test)]
    fn with_test_limits(
        max_input_bytes: u64,
        max_output_bytes: u64,
        pdf_timeout_ms: u64,
        pdf_executable: PathBuf,
        allow_external_processes: bool,
    ) -> Self {
        Self {
            max_input_bytes,
            max_output_bytes,
            pdf_timeout_ms,
            pdf_executable,
            allow_external_processes,
        }
    }

    pub async fn ingest(
        &self,
        source: &Path,
        query: Option<&Query>,
    ) -> Result<LocalIngestResponse, LocalIngestError> {
        let started = Instant::now();
        let canonical = tokio::fs::canonicalize(source)
            .await
            .map_err(|_| LocalIngestError::SourceUnavailable)?;
        let metadata = tokio::fs::metadata(&canonical)
            .await
            .map_err(|_| LocalIngestError::SourceUnavailable)?;
        if !metadata.is_file() {
            return Err(LocalIngestError::NotAFile);
        }
        if metadata.len() > self.max_input_bytes {
            return Err(LocalIngestError::InputLimit);
        }
        let bytes = read_bounded(&canonical, self.max_input_bytes).await?;
        let document_type = detect_document_type(&canonical, &bytes)?;
        let (extracted, extraction_tag) = self.extract(&document_type, &bytes).await?;
        if extracted.len() as u64 > self.max_output_bytes {
            return Err(LocalIngestError::OutputLimit);
        }
        if extracted.trim().is_empty() {
            return Err(LocalIngestError::EmptyDocument);
        }

        let source_uri =
            url::Url::from_file_path(&canonical).map_err(|_| LocalIngestError::InvalidSourceUri)?;
        let source_hash = hex_digest(&bytes);
        let identity = format!("local-document-v1\0{}\0{source_hash}", source_uri.as_str());
        let document_id = hex_digest(identity.as_bytes());
        let retrieved_at = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .map_err(|_| LocalIngestError::InvalidDocument)?;
        let mut document_metadata = BTreeMap::from([
            ("local_document_type".into(), document_type.as_str().into()),
            ("source_size_bytes".into(), bytes.len().to_string()),
        ]);
        if let Ok(modified) = metadata.modified() {
            if let Ok(value) = OffsetDateTime::from(modified).format(&Rfc3339) {
                document_metadata.insert("filesystem_modified_at".into(), value);
            }
        }
        let file_name = canonical
            .file_name()
            .and_then(|value| value.to_str())
            .map(str::to_owned);
        let title = if document_type == LocalDocumentType::Html {
            html_title(&bytes).or(file_name)
        } else {
            file_name
        };
        let document = Document {
            schema_version: SCHEMA_VERSION.into(),
            search_result_id: document_id,
            original_url: OriginalUrl(source_uri.clone()),
            canonical_url: CanonicalUrl(source_uri.clone()),
            final_url: FinalUrl(source_uri),
            content_hash: source_hash,
            fetch_method: FetchMethod::Local,
            extractor_used: Some(
                extraction_tag
                    .unwrap_or_else(|| document_type.extractor_version())
                    .into(),
            ),
            content_type: Some(document_type.media_type().into()),
            size: bytes.len() as u64,
            retrieved_at,
            status: DocumentStatus::Enriched,
            content: Some(extracted),
            title,
            author: None,
            published_at: None,
            metadata: document_metadata,
        };
        let (mut evidence, mut evidence_v2) =
            analyze_evidence_bundle_optional(query, std::slice::from_ref(&document));
        Ok(LocalIngestResponse {
            schema_version: SCHEMA_VERSION.into(),
            query: query.map(|value| value.raw_query.clone()),
            document_type,
            document,
            evidence: evidence.remove(0),
            evidence_v2: evidence_v2.remove(0),
            elapsed_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
        })
    }

    async fn extract(
        &self,
        document_type: &LocalDocumentType,
        bytes: &[u8],
    ) -> Result<(String, Option<&'static str>), LocalIngestError> {
        match document_type {
            LocalDocumentType::Pdf => self.extract_pdf(bytes).await,
            LocalDocumentType::Html => extract_html(bytes).map(|content| (content, None)),
            LocalDocumentType::Json => extract_json(bytes).map(|content| (content, None)),
            LocalDocumentType::JsonLines => {
                extract_json_lines(bytes).map(|content| (content, None))
            }
            LocalDocumentType::PlainText
            | LocalDocumentType::Markdown
            | LocalDocumentType::Csv
            | LocalDocumentType::SourceCode => utf8_text(bytes).map(|content| (content, None)),
        }
    }

    async fn extract_pdf(
        &self,
        bytes: &[u8],
    ) -> Result<(String, Option<&'static str>), LocalIngestError> {
        // Prefer the governed external extractor when processes are allowed.
        if self.allow_external_processes {
            match self.extract_pdf_external(bytes).await {
                Ok(content) => return Ok((content, Some("local-pdftotext-v1"))),
                Err(LocalIngestError::PdfExtractorUnavailable)
                | Err(LocalIngestError::PdfExtractorTimeout)
                | Err(LocalIngestError::PdfExtractorFailed) => {
                    // Fall through to the pure-Rust fallback below.
                }
                Err(other) => return Err(other),
            }
        }
        // Pure-Rust fallback: never spawns a process, so it is safe under an
        // isolated policy. It only decodes uncompressed content streams.
        match extract_pdf_rust(bytes) {
            Ok(content) => Ok((content, Some("local-pdf-rust-v1"))),
            Err(LocalIngestError::EmptyDocument) => {
                if self.allow_external_processes {
                    Err(LocalIngestError::PdfExtractorFailed)
                } else {
                    Err(LocalIngestError::ExternalExtractorDenied)
                }
            }
            Err(other) => Err(other),
        }
    }

    async fn extract_pdf_external(&self, bytes: &[u8]) -> Result<String, LocalIngestError> {
        if !self.allow_external_processes {
            return Err(LocalIngestError::ExternalExtractorDenied);
        }
        let operation = async {
            let mut child = Command::new(&self.pdf_executable)
                .args(["-", "-"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .map_err(|error| {
                    if error.kind() == std::io::ErrorKind::NotFound {
                        LocalIngestError::PdfExtractorUnavailable
                    } else {
                        LocalIngestError::PdfExtractorFailed
                    }
                })?;
            let mut stdin = child
                .stdin
                .take()
                .ok_or(LocalIngestError::PdfExtractorFailed)?;
            let stdout = child
                .stdout
                .take()
                .ok_or(LocalIngestError::PdfExtractorFailed)?;
            let mut limited = stdout.take(self.max_output_bytes.saturating_add(1));
            let mut output = Vec::new();
            let write = async move {
                stdin
                    .write_all(bytes)
                    .await
                    .map_err(|_| LocalIngestError::PdfExtractorFailed)?;
                stdin
                    .shutdown()
                    .await
                    .map_err(|_| LocalIngestError::PdfExtractorFailed)?;
                drop(stdin);
                Ok::<_, LocalIngestError>(())
            };
            let read = async {
                limited
                    .read_to_end(&mut output)
                    .await
                    .map_err(|_| LocalIngestError::PdfExtractorFailed)
            };
            let (write_result, read_result) = tokio::join!(write, read);
            write_result?;
            read_result?;
            let status = child
                .wait()
                .await
                .map_err(|_| LocalIngestError::PdfExtractorFailed)?;
            Ok::<_, LocalIngestError>((status, output))
        };
        let (status, output) =
            tokio::time::timeout(Duration::from_millis(self.pdf_timeout_ms), operation)
                .await
                .map_err(|_| LocalIngestError::PdfExtractorTimeout)??;
        if output.len() as u64 > self.max_output_bytes {
            return Err(LocalIngestError::OutputLimit);
        }
        if !status.success() {
            return Err(LocalIngestError::PdfExtractorFailed);
        }
        let content = String::from_utf8(output)
            .map_err(|_| LocalIngestError::InvalidTextEncoding)?
            .trim()
            .to_owned();
        if content.is_empty() {
            return Err(LocalIngestError::EmptyDocument);
        }
        Ok(content)
    }
}

/// Minimal pure-Rust PDF text extraction used as a fallback when the external
/// `pdftotext` binary is unavailable or denied by policy. It scans uncompressed
/// content streams for the text-showing operators `Tj`, `TJ`, `'` and `"`,
/// decoding literal and hex string operands. Compressed (FlateDecode) streams
/// are not decoded, so such documents degrade to
/// [`LocalIngestError::EmptyDocument`].
fn extract_pdf_rust(bytes: &[u8]) -> Result<String, LocalIngestError> {
    let input = String::from_utf8_lossy(bytes);
    let hay = input.as_bytes();
    let mut parts = Vec::new();
    let mut search_from = 0;
    while let Some(start) = find_stream_keyword(hay, search_from) {
        let Some(end) = find_sub(hay, start + 6, b"endstream") else {
            break;
        };
        let mut body = &input[start + 6..end];
        // Skip the newline that typically follows the `stream` keyword.
        body = body
            .strip_prefix("\r\n")
            .or_else(|| body.strip_prefix('\n'))
            .unwrap_or(body);
        collect_pdf_text(body, &mut parts);
        search_from = end + 9;
    }
    let content = parts.join("\n");
    if content.trim().is_empty() {
        return Err(LocalIngestError::EmptyDocument);
    }
    Ok(content)
}

/// Locate the next `stream` keyword (not the `stream` inside `endstream`),
/// i.e. a `stream` token delimited by whitespace or the buffer edges.
fn find_stream_keyword(hay: &[u8], from: usize) -> Option<usize> {
    const NEEDLE: &[u8] = b"stream";
    let mut i = from;
    while i + NEEDLE.len() <= hay.len() {
        if &hay[i..i + NEEDLE.len()] == NEEDLE {
            let prev_ok = i == 0 || hay[i - 1].is_ascii_whitespace();
            let next_ok = hay
                .get(i + NEEDLE.len())
                .is_none_or(|byte| byte.is_ascii_whitespace());
            if prev_ok && next_ok {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// Locate the first occurrence of `needle` at or after `from`.
fn find_sub(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    let tail = hay.get(from..)?;
    tail.windows(needle.len())
        .position(|window| window == needle)
        .map(|offset| from + offset)
}

/// Scan a PDF content stream body for text-showing operators and collect the
/// decoded strings, one per shown line.
fn collect_pdf_text(body: &str, parts: &mut Vec<String>) {
    let bytes = body.as_bytes();
    let mut i = 0;
    let mut line = String::new();
    let flush = |line: &mut String, parts: &mut Vec<String>| {
        if !line.trim().is_empty() {
            parts.push(line.trim().to_owned());
            line.clear();
        }
    };
    while i < bytes.len() {
        match bytes[i] {
            b'(' => {
                if let Some((text, next)) = parse_literal_string(bytes, i) {
                    line.push_str(&text);
                    i = next;
                    continue;
                }
            }
            b'<' => {
                if let Some((text, next)) = parse_hex_string(bytes, i) {
                    line.push_str(&text);
                    i = next;
                    continue;
                }
            }
            b'[' => {
                if let Some((text, next)) = parse_text_array(bytes, i) {
                    line.push_str(&text);
                    i = next;
                    continue;
                }
            }
            b'T' if matches!(bytes.get(i + 1), Some(b'j') | Some(b'J')) => {
                flush(&mut line, parts);
                i += 2;
                continue;
            }
            b'\'' | b'"' => {
                flush(&mut line, parts);
            }
            _ => {}
        }
        i += 1;
    }
    flush(&mut line, parts);
}

/// Parse a PDF literal string `( ... )`, honouring backslash escapes and
/// nested parentheses. Returns the decoded text and the index just past the
/// closing parenthesis.
fn parse_literal_string(bytes: &[u8], start: usize) -> Option<(String, usize)> {
    debug_assert_eq!(bytes.get(start), Some(&b'('));
    let mut i = start + 1;
    let mut out = String::new();
    let mut depth = 1usize;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                let Some(&next) = bytes.get(i + 1) else {
                    break;
                };
                match next {
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'b' => out.push('\u{8}'),
                    b'f' => out.push('\u{c}'),
                    b'(' => out.push('('),
                    b')' => out.push(')'),
                    b'\\' => out.push('\\'),
                    b'0'..=b'7' => {
                        let mut value = 0u32;
                        let mut count = 0;
                        while count < 3 {
                            let Some(&digit) = bytes.get(i + 1 + count) else {
                                break;
                            };
                            if !(b'0'..=b'7').contains(&digit) {
                                break;
                            }
                            value = value * 8 + u32::from(digit - b'0');
                            count += 1;
                        }
                        if let Some(ch) = char::from_u32(value) {
                            out.push(ch);
                        }
                        i += count;
                    }
                    _ => out.push(next as char),
                }
                i += 2;
            }
            b'(' => {
                depth += 1;
                out.push('(');
                i += 1;
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some((out, i + 1));
                }
                out.push(')');
                i += 1;
            }
            byte => {
                out.push(byte as char);
                i += 1;
            }
        }
    }
    None
}

/// Parse a PDF hex string `< ... >`, decoding pairs of hex digits into bytes.
fn parse_hex_string(bytes: &[u8], start: usize) -> Option<(String, usize)> {
    debug_assert_eq!(bytes.get(start), Some(&b'<'));
    let mut i = start + 1;
    let mut out = String::new();
    let mut high: Option<u8> = None;
    while i < bytes.len() {
        let byte = bytes[i];
        if byte == b'>' {
            if let Some(h) = high {
                out.push((h << 4) as char);
            }
            return Some((out, i + 1));
        }
        if let Some(value) = hex_value(byte) {
            match high {
                None => high = Some(value),
                Some(h) => {
                    out.push(((h << 4) | value) as char);
                    high = None;
                }
            }
        }
        i += 1;
    }
    None
}

/// Parse a PDF text array `[ ... ]` (the operand of `TJ`), concatenating the
/// string elements it contains.
fn parse_text_array(bytes: &[u8], start: usize) -> Option<(String, usize)> {
    debug_assert_eq!(bytes.get(start), Some(&b'['));
    let mut i = start + 1;
    let mut out = String::new();
    while i < bytes.len() {
        match bytes[i] {
            b']' => return Some((out, i + 1)),
            b'(' => {
                let (text, next) = parse_literal_string(bytes, i)?;
                out.push_str(&text);
                i = next;
            }
            b'<' => {
                let (text, next) = parse_hex_string(bytes, i)?;
                out.push_str(&text);
                i = next;
            }
            _ => i += 1,
        }
    }
    None
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

async fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, LocalIngestError> {
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|_| LocalIngestError::SourceUnavailable)?;
    let mut limited = file.take(limit.saturating_add(1));
    let mut bytes = Vec::new();
    limited
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| LocalIngestError::SourceUnavailable)?;
    if bytes.len() as u64 > limit {
        return Err(LocalIngestError::InputLimit);
    }
    Ok(bytes)
}

fn detect_document_type(path: &Path, bytes: &[u8]) -> Result<LocalDocumentType, LocalIngestError> {
    if bytes.starts_with(b"%PDF-") {
        return Ok(LocalDocumentType::Pdf);
    }
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if extension == "pdf" {
        return Err(LocalIngestError::InvalidDocument);
    }
    let kind = match extension.as_str() {
        "md" | "markdown" | "mdx" => LocalDocumentType::Markdown,
        "html" | "htm" | "xhtml" => LocalDocumentType::Html,
        "json" => LocalDocumentType::Json,
        "jsonl" | "ndjson" => LocalDocumentType::JsonLines,
        "csv" | "tsv" => LocalDocumentType::Csv,
        "rs" | "py" | "js" | "jsx" | "ts" | "tsx" | "java" | "go" | "c" | "h" | "cc" | "cpp"
        | "hpp" | "cs" | "rb" | "php" | "sh" | "bash" | "zsh" | "fish" | "sql" | "toml"
        | "yaml" | "yml" => LocalDocumentType::SourceCode,
        "txt" | "text" | "log" | "rst" => LocalDocumentType::PlainText,
        _ => {
            if bytes.contains(&0) || std::str::from_utf8(bytes).is_err() {
                return Err(LocalIngestError::UnsupportedDocumentType);
            }
            if looks_like_html(bytes) {
                LocalDocumentType::Html
            } else {
                LocalDocumentType::PlainText
            }
        }
    };
    Ok(kind)
}

fn utf8_text(bytes: &[u8]) -> Result<String, LocalIngestError> {
    let value = std::str::from_utf8(bytes).map_err(|_| LocalIngestError::InvalidTextEncoding)?;
    Ok(value.strip_prefix('\u{feff}').unwrap_or(value).to_owned())
}

fn extract_json(bytes: &[u8]) -> Result<String, LocalIngestError> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| LocalIngestError::InvalidDocument)?;
    serde_json::to_string_pretty(&value).map_err(|_| LocalIngestError::InvalidDocument)
}

fn extract_json_lines(bytes: &[u8]) -> Result<String, LocalIngestError> {
    let input = utf8_text(bytes)?;
    let mut lines = Vec::new();
    for line in input.lines().filter(|line| !line.trim().is_empty()) {
        let value: serde_json::Value =
            serde_json::from_str(line).map_err(|_| LocalIngestError::InvalidDocument)?;
        lines.push(serde_json::to_string(&value).map_err(|_| LocalIngestError::InvalidDocument)?);
    }
    if lines.is_empty() {
        return Err(LocalIngestError::EmptyDocument);
    }
    Ok(lines.join("\n"))
}

fn extract_html(bytes: &[u8]) -> Result<String, LocalIngestError> {
    let input = utf8_text(bytes)?;
    let document = Html::parse_document(&input);
    let body_selector = Selector::parse("body").map_err(|_| LocalIngestError::InvalidDocument)?;
    let root = document
        .select(&body_selector)
        .next()
        .unwrap_or_else(|| document.root_element());
    let mut parts = Vec::new();
    for node in root.descendants() {
        let Some(text) = node.value().as_text() else {
            continue;
        };
        let hidden = node.ancestors().any(|ancestor| {
            ancestor.value().as_element().is_some_and(|element| {
                matches!(
                    element.name(),
                    "script" | "style" | "noscript" | "template" | "head"
                )
            })
        });
        if hidden {
            continue;
        }
        let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if !normalized.is_empty() {
            parts.push(normalized);
        }
    }
    let content = parts.join("\n");
    if content.is_empty() {
        return Err(LocalIngestError::EmptyDocument);
    }
    Ok(content)
}

fn html_title(bytes: &[u8]) -> Option<String> {
    let input = std::str::from_utf8(bytes).ok()?;
    let document = Html::parse_document(input);
    let selector = Selector::parse("title").ok()?;
    let title = document
        .select(&selector)
        .next()?
        .text()
        .flat_map(str::split_whitespace)
        .collect::<Vec<_>>()
        .join(" ");
    (!title.is_empty()).then_some(title)
}

fn looks_like_html(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).ok().is_some_and(|value| {
        let value = value.trim_start().to_ascii_lowercase();
        value.starts_with("<!doctype html") || value.starts_with("<html")
    })
}

fn hex_digest(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{parse_query, EgressPolicy, EvidenceSignal, SecurityProfile};
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    fn temp_file(extension: &str, content: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "amatl-local-ingest-{}-{}.{}",
            std::process::id(),
            OffsetDateTime::now_utc().unix_timestamp_nanos(),
            extension
        ));
        std::fs::write(&path, content).unwrap();
        path
    }

    fn in_process_ingestor() -> LocalIngestor {
        let policy = DataPolicyConfig {
            egress: EgressPolicy::Deny,
            ..DataPolicyConfig::default()
        };
        LocalIngestor::new(&policy)
    }

    #[tokio::test]
    async fn text_ingestion_produces_local_document_and_traceable_evidence() {
        let path = temp_file(
            "md",
            b"# Security report\nAMATL reached 95 percent on 2026-08-13.",
        );
        let query = parse_query("AMATL security".into()).unwrap();
        let response = in_process_ingestor()
            .ingest(&path, Some(&query))
            .await
            .unwrap();
        std::fs::remove_file(path).unwrap();

        assert_eq!(response.document_type, LocalDocumentType::Markdown);
        assert_eq!(response.document.fetch_method, FetchMethod::Local);
        assert_eq!(response.document.status, DocumentStatus::Enriched);
        assert_eq!(
            response.document.content_type.as_deref(),
            Some("text/markdown")
        );
        assert_eq!(
            response.evidence.document_id,
            response.document.search_result_id
        );
        assert_eq!(
            response.evidence_v2.document_id,
            response.document.search_result_id
        );
        assert!(response.evidence_v2.fragments.iter().any(|fragment| {
            fragment.signals.contains(&EvidenceSignal::QueryMatch)
                && fragment.text.contains("AMATL")
        }));
    }

    #[tokio::test]
    async fn html_dispatch_excludes_executable_text_and_uses_title() {
        let path = temp_file(
            "html",
            b"<html><head><title>Local report</title><style>secret-style</style></head><body><h1>Evidence</h1><script>secret-script</script><p>Visible 42.</p></body></html>",
        );
        let response = in_process_ingestor().ingest(&path, None).await.unwrap();
        std::fs::remove_file(path).unwrap();

        assert_eq!(response.document_type, LocalDocumentType::Html);
        assert_eq!(response.document.title.as_deref(), Some("Local report"));
        let content = response.document.content.unwrap();
        assert!(content.contains("Evidence") && content.contains("Visible 42."));
        assert!(!content.contains("secret-script"));
        assert!(!content.contains("secret-style"));
    }

    #[tokio::test]
    async fn structured_dispatch_validates_json_and_rejects_binary() {
        let json_path = temp_file("json", br#"{"b":2,"a":1}"#);
        let response = in_process_ingestor()
            .ingest(&json_path, None)
            .await
            .unwrap();
        std::fs::remove_file(json_path).unwrap();
        assert_eq!(response.document_type, LocalDocumentType::Json);
        assert!(response.document.content.unwrap().contains("\"a\": 1"));

        let binary_path = temp_file("bin", b"binary\0payload");
        let error = in_process_ingestor()
            .ingest(&binary_path, None)
            .await
            .unwrap_err();
        std::fs::remove_file(binary_path).unwrap();
        assert_eq!(error, LocalIngestError::UnsupportedDocumentType);
    }

    #[test]
    fn dispatch_table_covers_supported_document_families() {
        let cases = [
            (
                "report.txt",
                b"text".as_slice(),
                LocalDocumentType::PlainText,
            ),
            (
                "report.md",
                b"# text".as_slice(),
                LocalDocumentType::Markdown,
            ),
            (
                "report.html",
                b"<p>text</p>".as_slice(),
                LocalDocumentType::Html,
            ),
            (
                "report.json",
                br#"{"text":1}"#.as_slice(),
                LocalDocumentType::Json,
            ),
            (
                "report.ndjson",
                b"{\"text\":1}\n".as_slice(),
                LocalDocumentType::JsonLines,
            ),
            (
                "report.csv",
                b"name,value\na,1".as_slice(),
                LocalDocumentType::Csv,
            ),
            (
                "report.rs",
                b"fn main() {}".as_slice(),
                LocalDocumentType::SourceCode,
            ),
            (
                "renamed.bin",
                b"%PDF-1.7\n".as_slice(),
                LocalDocumentType::Pdf,
            ),
            (
                "no-extension",
                b"<!doctype html><p>x</p>".as_slice(),
                LocalDocumentType::Html,
            ),
        ];
        for (name, bytes, expected) in cases {
            assert_eq!(
                detect_document_type(Path::new(name), bytes).unwrap(),
                expected,
                "{name}"
            );
        }
        assert_eq!(
            detect_document_type(Path::new("false.pdf"), b"not a pdf"),
            Err(LocalIngestError::InvalidDocument)
        );
    }

    #[tokio::test]
    async fn both_metadata_and_stream_enforce_the_input_limit() {
        let path = temp_file("txt", b"12345");
        let ingestor = LocalIngestor::with_test_limits(4, 16, 100, "missing".into(), false);
        let error = ingestor.ingest(&path, None).await.unwrap_err();
        std::fs::remove_file(path).unwrap();
        assert_eq!(error, LocalIngestError::InputLimit);
    }

    #[tokio::test]
    async fn pdf_dispatch_is_denied_before_process_under_isolated_policy() {
        let path = temp_file("pdf", b"%PDF-1.7\nlocal");
        let policy = DataPolicyConfig {
            profile: SecurityProfile::Isolated,
            egress: EgressPolicy::Deny,
            ..DataPolicyConfig::default()
        };
        let error = LocalIngestor::new(&policy)
            .ingest(&path, None)
            .await
            .unwrap_err();
        std::fs::remove_file(path).unwrap();
        assert_eq!(error, LocalIngestError::ExternalExtractorDenied);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pdf_dispatch_uses_bounded_local_extractor() {
        let executable = temp_file(
            "sh",
            b"#!/bin/sh\ncat >/dev/null\nprintf 'PDF evidence 73'\n",
        );
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let pdf = temp_file("pdf", b"%PDF-1.7\nfixture");
        let ingestor = LocalIngestor::with_test_limits(1024, 1024, 2_000, executable.clone(), true);
        let response = ingestor.ingest(&pdf, None).await.unwrap();
        std::fs::remove_file(executable).unwrap();
        std::fs::remove_file(pdf).unwrap();

        assert_eq!(response.document_type, LocalDocumentType::Pdf);
        assert_eq!(
            response.document.content.as_deref(),
            Some("PDF evidence 73")
        );
        assert_eq!(
            response.document.extractor_used.as_deref(),
            Some("local-pdftotext-v1")
        );
    }

    #[tokio::test]
    async fn pdf_fallback_extracts_uncompressed_stream_under_isolated_policy() {
        let pdf = temp_file(
            "pdf",
            b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog >>\nendobj\n2 0 obj\n<< /Type /Page /Contents 3 0 R >>\nendobj\n3 0 obj\n<< /Length 44 >>\nstream\nBT /F1 12 Tf 72 720 Td (Hello PDF) Tj ET\nendstream\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n",
        );
        let policy = DataPolicyConfig {
            profile: SecurityProfile::Isolated,
            egress: EgressPolicy::Deny,
            ..DataPolicyConfig::default()
        };
        let response = LocalIngestor::new(&policy)
            .ingest(&pdf, None)
            .await
            .unwrap();
        std::fs::remove_file(pdf).unwrap();

        assert_eq!(response.document_type, LocalDocumentType::Pdf);
        assert_eq!(response.document.content.as_deref(), Some("Hello PDF"));
        assert_eq!(
            response.document.extractor_used.as_deref(),
            Some("local-pdf-rust-v1")
        );
    }

    #[tokio::test]
    async fn pdf_fallback_used_when_external_extractor_is_missing() {
        let pdf = temp_file(
            "pdf",
            b"%PDF-1.4\nstream\nBT (Fallback text) Tj ET\nendstream\n%%EOF\n",
        );
        // Processes are allowed but the executable does not exist, so the
        // external extractor reports unavailable and the fallback takes over.
        let ingestor = LocalIngestor::with_test_limits(
            1024,
            1024,
            2_000,
            "definitely-missing-pdftotext".into(),
            true,
        );
        let response = ingestor.ingest(&pdf, None).await.unwrap();
        std::fs::remove_file(pdf).unwrap();

        assert_eq!(response.document.content.as_deref(), Some("Fallback text"));
        assert_eq!(
            response.document.extractor_used.as_deref(),
            Some("local-pdf-rust-v1")
        );
    }

    #[test]
    fn pdf_rust_extractor_decodes_literal_hex_and_array_operators() {
        let content = extract_pdf_rust(
            b"stream\nBT (Alpha) Tj 0 -14 Td <48656C6C6F> Tj 0 -14 Td [(A)(B)(C)] TJ ET\nendstream\n",
        )
        .unwrap();
        assert_eq!(content, "Alpha\nHello\nABC");
    }

    #[test]
    fn pdf_rust_extractor_handles_escapes_and_nested_parens() {
        let content = extract_pdf_rust(
            b"stream\nBT (a\\(b\\)c) Tj 0 -14 Td (line\\nbreak) Tj ET\nendstream\n",
        )
        .unwrap();
        assert_eq!(content, "a(b)c\nline\nbreak");
    }

    #[test]
    fn pdf_rust_extractor_rejects_empty_or_compressed_streams() {
        assert_eq!(
            extract_pdf_rust(b"%PDF-1.4\nstream\nBT ET\nendstream\n%%EOF\n"),
            Err(LocalIngestError::EmptyDocument)
        );
        // A FlateDecode stream is not decoded by the fallback.
        assert_eq!(
            extract_pdf_rust(b"%PDF-1.4\nstream\n\x78\x9c\xcbH\xcd\xc9\xc9\x07\x00\x06\x2c\x02\x15\nendstream\n%%EOF\n"),
            Err(LocalIngestError::EmptyDocument)
        );
    }

    #[tokio::test]
    async fn malformed_corpus_is_rejected_gracefully() {
        let empty = temp_file("txt", b"");
        let error = in_process_ingestor()
            .ingest(&empty, None)
            .await
            .unwrap_err();
        std::fs::remove_file(empty).unwrap();
        assert_eq!(error, LocalIngestError::EmptyDocument);

        let invalid_utf8 = temp_file("txt", b"\xff\xfe\x00binary");
        let error = in_process_ingestor()
            .ingest(&invalid_utf8, None)
            .await
            .unwrap_err();
        std::fs::remove_file(invalid_utf8).unwrap();
        assert_eq!(error, LocalIngestError::InvalidTextEncoding);

        let bad_json = temp_file("json", b"{not valid json");
        let error = in_process_ingestor()
            .ingest(&bad_json, None)
            .await
            .unwrap_err();
        std::fs::remove_file(bad_json).unwrap();
        assert_eq!(error, LocalIngestError::InvalidDocument);

        let bad_jsonl = temp_file("jsonl", b"{\"a\":1}\nnot-json\n");
        let error = in_process_ingestor()
            .ingest(&bad_jsonl, None)
            .await
            .unwrap_err();
        std::fs::remove_file(bad_jsonl).unwrap();
        assert_eq!(error, LocalIngestError::InvalidDocument);
    }

    proptest::proptest! {
        #[test]
        fn html_parser_never_panics_on_arbitrary_bytes(
            bytes in proptest::collection::vec(proptest::num::u8::ANY, 0..4096)
        ) {
            let _ = extract_html(&bytes);
            let _ = html_title(&bytes);
            let _ = looks_like_html(&bytes);
        }

        #[test]
        fn pdf_fallback_never_panics_on_arbitrary_bytes(
            bytes in proptest::collection::vec(proptest::num::u8::ANY, 0..4096)
        ) {
            let _ = extract_pdf_rust(&bytes);
        }

        #[test]
        fn text_parser_never_panics_on_arbitrary_bytes(
            bytes in proptest::collection::vec(proptest::num::u8::ANY, 0..4096)
        ) {
            let _ = utf8_text(&bytes);
        }
    }
}
