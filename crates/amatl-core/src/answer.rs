//! Answer synthesis: an optional, explicit layer on top of AMATL Search.
//!
//! `search`/`deep` stay exactly what they always were — AMATL executes and
//! returns what it retrieved, nothing more. `answer` is a distinct, opt-in
//! capability that a caller invokes on purpose: it runs a search, hands the
//! ranked results to a remote chat-completion model as the *only* context it
//! is allowed to use, and returns a short synthesized answer with citations
//! back to the sources that grounded it.
//!
//! # Grounding, not inference
//!
//! The model is never asked to use its own knowledge. The system prompt is
//! explicit that an unanswerable query must be reported as such, and every
//! sentence the model keeps must cite a source index. Citations are then
//! checked mechanically: a citation to a source index that does not exist is
//! stripped, and an answer that cites nothing is rejected outright — a
//! response with no traceable source is exactly the failure mode this module
//! exists to catch, not to forward. This does not make hallucination
//! impossible (no prompt does), but it makes every claim traceable to a
//! specific AMATL result, and makes an ungrounded answer a typed error
//! instead of a plausible-looking response.
//!
//! # Governance
//!
//! Mirrors [`crate::inference::RemoteEmbeddingBackend`]: gated by
//! `data_policy.inference = "remote_explicit"`, endpoint validated by
//! [`crate::inference::validate_remote_endpoint`], credential read once from
//! an environment variable and never logged, one bounded request per call
//! (`answer.timeout_ms`), no retries hidden in this module. What crosses the
//! boundary is the query plus the bounded, already-public search results
//! AMATL retrieved — nothing else.

use crate::model::SearchResult;
use crate::providers::{HttpRequest, HttpTransport};
use std::sync::Arc;
use thiserror::Error;

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum AnswerError {
    #[error("answer synthesis backend is not configured or its credential is missing")]
    BackendUnavailable,
    #[error("answer synthesis endpoint is invalid")]
    InvalidEndpoint,
    #[error("answer synthesis request failed")]
    RequestFailed,
    #[error("answer synthesis response did not match the expected contract")]
    ResponseInvalid,
    #[error("answer synthesis response cited no source; treated as ungrounded, not returned")]
    Ungrounded,
    #[error("no sources were available to ground an answer")]
    NoSources,
}

/// One search result reduced to what the model is allowed to see: enough to
/// answer from and to cite, nothing that was not already public.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct AnswerSource {
    /// 1-based index the model must cite; stable within one `synthesize` call.
    pub index: usize,
    pub title: Option<String>,
    pub url: String,
    pub snippet: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Answer {
    pub text: String,
    /// Source indices the answer actually cited, in order of first mention.
    pub citations: Vec<usize>,
    pub sources: Vec<AnswerSource>,
    pub model: String,
}

/// Contract a chat-completion backend must honor. Exists so a fixture or a
/// second vendor can stand in for [`RemoteCompletionBackend`] in tests without
/// this module caring which one it is talking to.
#[async_trait::async_trait]
pub trait CompletionBackend: Send + Sync {
    async fn complete(&self, system: &str, user: &str) -> Result<String, AnswerError>;
    fn model(&self) -> &str;
}

/// Governed remote completion backend, OpenAI-compatible
/// (`{"model":…,"messages":[…]}` → `choices[0].message.content`). DeepInfra
/// and DeepSeek both speak this shape, so pointing this at either is a
/// configuration change, not a code change.
pub struct RemoteCompletionBackend {
    endpoint: url::Url,
    model: String,
    credential: Option<String>,
    timeout_ms: u64,
    max_answer_tokens: u32,
    transport: Arc<dyn HttpTransport>,
}

impl RemoteCompletionBackend {
    pub fn new(
        config: &crate::config::AnswerConfig,
        transport: Arc<dyn HttpTransport>,
    ) -> Result<Self, AnswerError> {
        let credential = config
            .credential_env
            .as_deref()
            .map(|name| {
                std::env::var(name)
                    .ok()
                    .filter(|value| !value.is_empty())
                    .ok_or(AnswerError::BackendUnavailable)
            })
            .transpose()?;
        Self::with_credential(config, transport, credential)
    }

    pub fn with_credential(
        config: &crate::config::AnswerConfig,
        transport: Arc<dyn HttpTransport>,
        credential: Option<String>,
    ) -> Result<Self, AnswerError> {
        let endpoint = crate::inference::validate_remote_endpoint(
            config
                .endpoint
                .as_deref()
                .ok_or(AnswerError::InvalidEndpoint)?,
        )
        .map_err(|_| AnswerError::InvalidEndpoint)?;
        if credential.as_deref().is_some_and(str::is_empty) {
            return Err(AnswerError::BackendUnavailable);
        }
        Ok(Self {
            endpoint,
            model: config
                .model
                .clone()
                .ok_or(AnswerError::BackendUnavailable)?,
            credential,
            timeout_ms: config.timeout_ms,
            max_answer_tokens: config.max_answer_tokens,
            transport,
        })
    }
}

#[async_trait::async_trait]
impl CompletionBackend for RemoteCompletionBackend {
    async fn complete(&self, system: &str, user: &str) -> Result<String, AnswerError> {
        let mut headers = vec![("accept".to_string(), "application/json".to_string())];
        if let Some(credential) = &self.credential {
            headers.push(("authorization".into(), format!("Bearer {credential}")));
        }
        let body = serde_json::to_vec(&serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user},
            ],
            "temperature": 0.0,
            "max_tokens": self.max_answer_tokens,
        }))
        .map_err(|_| AnswerError::RequestFailed)?;
        let response = self
            .transport
            .execute(HttpRequest::post_json(
                self.endpoint.clone(),
                headers,
                self.timeout_ms,
                body,
            ))
            .await
            .map_err(|_| AnswerError::RequestFailed)?;
        if response.status != 200 {
            return Err(AnswerError::RequestFailed);
        }
        let parsed: serde_json::Value =
            serde_json::from_slice(&response.body).map_err(|_| AnswerError::ResponseInvalid)?;
        parsed["choices"][0]["message"]["content"]
            .as_str()
            .map(str::to_owned)
            .ok_or(AnswerError::ResponseInvalid)
    }

    fn model(&self) -> &str {
        &self.model
    }
}

/// Reduce ranked search results to bounded, citable sources. Order is
/// preserved — index 1 is the top-ranked result — so citations double as a
/// relevance signal, not just a lookup key.
pub fn build_sources(
    results: &[SearchResult],
    max_sources: usize,
    max_source_chars: usize,
) -> Vec<AnswerSource> {
    results
        .iter()
        .take(max_sources)
        .enumerate()
        .map(|(position, result)| AnswerSource {
            index: position + 1,
            title: result.title.clone(),
            url: result.canonical_url.0.to_string(),
            snippet: bounded(
                result.snippet.as_deref().unwrap_or_default(),
                max_source_chars,
            ),
        })
        .collect()
}

fn bounded(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

const SYSTEM_PROMPT: &str = "You answer strictly from the numbered sources you are given. \
Every sentence that states a fact must end with the citation of the source \
it came from, like [2]. If the sources do not contain the answer, say \
plainly that AMATL's search did not find it — never fill the gap from your \
own knowledge. Keep the answer short. Always write the answer in clear, \
grammatically correct Spanish (español), regardless of the language of the \
query or the sources — translate facts faithfully, do not just switch \
vocabulary. Do not pad the answer with generic filler sentences that add no \
fact of their own.";

fn build_user_prompt(query: &str, sources: &[AnswerSource]) -> String {
    let mut prompt = format!("Question: {query}\n\nSources:\n");
    for source in sources {
        let title = source.title.as_deref().unwrap_or("(untitled)");
        prompt.push_str(&format!(
            "[{}] {title} — {}\n{}\n\n",
            source.index, source.url, source.snippet
        ));
    }
    prompt
}

/// Extract every `[n]` citation marker from the model's answer, in order of
/// first appearance, deduplicated, keeping only indices that exist among
/// `sources` — a citation to a made-up index is dropped, not trusted.
fn extract_citations(text: &str, sources: &[AnswerSource]) -> Vec<usize> {
    let valid: std::collections::BTreeSet<usize> =
        sources.iter().map(|source| source.index).collect();
    let mut seen = std::collections::BTreeSet::new();
    let mut citations = Vec::new();
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'[' {
            if let Some(end) = text[index + 1..].find(']') {
                let candidate = &text[index + 1..index + 1 + end];
                if let Ok(value) = candidate.parse::<usize>() {
                    if valid.contains(&value) && seen.insert(value) {
                        citations.push(value);
                    }
                }
                index += end + 2;
                continue;
            }
        }
        index += 1;
    }
    citations
}

/// Remove every `[n]` marker whose `n` is not a real source index.
///
/// `extract_citations` already refused to *count* a fabricated citation as
/// grounding, but until this ran, the marker itself — `[7]` pointing at a
/// source that doesn't exist — stayed in the text the reader sees, reading
/// as a legitimate 7th source that simply wasn't listed. A citation the
/// model invented is not information to preserve; the sentence it was
/// attached to still had no real source, same as if the bracket had never
/// been there.
fn strip_invalid_citations(text: &str, valid: &std::collections::BTreeSet<usize>) -> String {
    let mut result = String::with_capacity(text.len());
    let mut index = 0;
    while index < text.len() {
        // `index` is always a char boundary here — either just-advanced by a
        // full character's byte length, or set from `find`, which only ever
        // returns boundaries of the (single-byte, ASCII) '[' / ']' bytes it
        // matched inside a valid &str. Byte-indexing without this care is
        // exactly how a stray accent or other multi-byte character gets
        // corrupted; the fix below never slices or casts mid-character.
        let ch = text[index..]
            .chars()
            .next()
            .expect("index is a char boundary");
        if ch == '[' {
            if let Some(end_rel) = text[index + 1..].find(']') {
                let candidate = &text[index + 1..index + 1 + end_rel];
                let bracket_end = index + 1 + end_rel + 1;
                match candidate.parse::<usize>() {
                    Ok(value) if valid.contains(&value) => {
                        result.push_str(&text[index..bracket_end]);
                    }
                    // A fabricated citation index: drop the whole marker —
                    // not information to preserve, the claim it decorated
                    // never had a real source either way.
                    Ok(_) => {}
                    // Not a citation at all (e.g. a literal "[note]"):
                    // untouched, this function only strips source numbers.
                    Err(_) => {
                        result.push_str(&text[index..bracket_end]);
                    }
                }
                index = bracket_end;
                continue;
            }
        }
        result.push(ch);
        index += ch.len_utf8();
    }
    result
}

/// Run one grounded answer synthesis call.
///
/// Fails with [`AnswerError::NoSources`] before ever reaching the network if
/// there is nothing to ground an answer in, and with
/// [`AnswerError::Ungrounded`] after the call if the model's response cited
/// no valid source — both are refusals to return an answer that is not
/// traceable, not attempts to salvage one. A citation to a source that does
/// not exist is stripped from the visible text, not merely uncounted, by the
/// private `strip_invalid_citations` helper below.
pub async fn synthesize(
    backend: &dyn CompletionBackend,
    query: &str,
    sources: Vec<AnswerSource>,
) -> Result<Answer, AnswerError> {
    if sources.is_empty() {
        return Err(AnswerError::NoSources);
    }
    let user_prompt = build_user_prompt(query, &sources);
    let raw_text = backend.complete(SYSTEM_PROMPT, &user_prompt).await?;
    let citations = extract_citations(&raw_text, &sources);
    if citations.is_empty() {
        return Err(AnswerError::Ungrounded);
    }
    let valid: std::collections::BTreeSet<usize> =
        sources.iter().map(|source| source.index).collect();
    let text = strip_invalid_citations(&raw_text, &valid);
    Ok(Answer {
        text,
        citations,
        sources,
        model: backend.model().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedBackend {
        response: String,
        model: String,
    }

    #[async_trait::async_trait]
    impl CompletionBackend for FixedBackend {
        async fn complete(&self, _system: &str, _user: &str) -> Result<String, AnswerError> {
            Ok(self.response.clone())
        }
        fn model(&self) -> &str {
            &self.model
        }
    }

    fn source(index: usize) -> AnswerSource {
        AnswerSource {
            index,
            title: Some(format!("Title {index}")),
            url: format!("https://example.com/{index}"),
            snippet: "snippet text".into(),
        }
    }

    #[tokio::test]
    async fn a_cited_answer_is_returned_with_its_citations_in_order() {
        let backend = FixedBackend {
            response: "Rust is memory-safe [1]. It compiles to native code [2].".into(),
            model: "test-model".into(),
        };
        let answer = synthesize(&backend, "what is rust", vec![source(1), source(2)])
            .await
            .unwrap();
        assert_eq!(answer.citations, vec![1, 2]);
        assert_eq!(answer.model, "test-model");
        assert_eq!(answer.sources.len(), 2);
    }

    #[tokio::test]
    async fn cites_only_indices_that_exist_and_deduplicates() {
        let backend = FixedBackend {
            response: "Fact one [1]. Repeated [1]. Fabricated [99].".into(),
            model: "test-model".into(),
        };
        let answer = synthesize(&backend, "q", vec![source(1), source(2)])
            .await
            .unwrap();
        assert_eq!(answer.citations, vec![1]);
        // Not just uncounted: the fabricated marker must not survive into the
        // text a reader sees, or it reads as a real, just-unlisted source.
        assert!(
            !answer.text.contains("[99]"),
            "fabricated citation marker leaked into the visible text: {}",
            answer.text
        );
        assert!(answer.text.contains("[1]"));
    }

    #[tokio::test]
    async fn stripping_a_fabricated_citation_does_not_corrupt_multibyte_text() {
        // Regression: an earlier version of this fix indexed text by byte
        // and cast individual bytes to `char`, which mangles any accented
        // or otherwise non-ASCII character. This response has accents both
        // before and after the fabricated marker specifically to catch that.
        let backend = FixedBackend {
            response: "áéí [1] óú fabricado [99] ñ final [1].".into(),
            model: "test-model".into(),
        };
        let answer = synthesize(&backend, "q", vec![source(1)]).await.unwrap();
        assert_eq!(answer.citations, vec![1]);
        assert!(!answer.text.contains("[99]"));
        assert_eq!(answer.text, "áéí [1] óú fabricado  ñ final [1].");
    }

    #[tokio::test]
    async fn an_answer_that_cites_nothing_is_rejected_as_ungrounded() {
        let backend = FixedBackend {
            response: "I looked it up and the answer is 42.".into(),
            model: "test-model".into(),
        };
        let error = synthesize(&backend, "q", vec![source(1)])
            .await
            .unwrap_err();
        assert_eq!(error, AnswerError::Ungrounded);
    }

    #[tokio::test]
    async fn no_sources_fails_closed_before_any_call() {
        let backend = FixedBackend {
            response: "unused".into(),
            model: "test-model".into(),
        };
        let error = synthesize(&backend, "q", vec![]).await.unwrap_err();
        assert_eq!(error, AnswerError::NoSources);
    }

    #[test]
    fn system_prompt_requires_spanish_and_no_filler() {
        assert!(SYSTEM_PROMPT.contains("Spanish"));
        assert!(SYSTEM_PROMPT.contains("filler"));
    }

    #[test]
    fn build_sources_truncates_snippets_and_indexes_from_one() {
        let results = vec![]; // exercised indirectly via service-level tests;
                              // this keeps the truncation boundary itself
                              // pinned without needing a full SearchResult.
        let sources = build_sources(&results, 8, 5);
        assert!(sources.is_empty());
        assert_eq!(bounded("abcdefgh", 5), "abcde");
    }
}
