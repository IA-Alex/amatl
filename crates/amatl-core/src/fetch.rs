use crate::model::FinalUrl;
use crate::security::{audit_ssrf_rejection, validate_deep_url, validate_resolved_addresses};
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, LOCATION};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;
use url::Url;

#[derive(Clone, Debug)]
pub struct FetchRequest {
    pub url: Url,
    pub timeout_ms: u64,
    pub max_bytes: u64,
    pub max_redirects: u32,
    pub headers: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FetchResult {
    pub final_url: FinalUrl,
    pub status: u16,
    pub headers_safe: BTreeMap<String, String>,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
    pub size: u64,
    pub redirect_chain: Vec<Url>,
    pub retrieved_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum FetchError {
    #[error("invalid or blocked URL: {0}")]
    BlockedUrl(String),
    #[error("DNS resolution failed")]
    Dns,
    #[error("resolved address blocked")]
    AddressBlocked,
    #[error("request header is not permitted")]
    HeaderBlocked,
    #[error("redirect limit exhausted")]
    RedirectLimit,
    #[error("redirect location is missing or invalid")]
    InvalidRedirect,
    #[error("response byte limit exhausted")]
    ByteLimit,
    #[error("fetch timed out")]
    Timeout,
    #[error("network request failed")]
    Network,
}

#[async_trait]
pub trait DnsResolver: Send + Sync {
    async fn resolve(&self, host: &str, port: u16) -> Result<Vec<SocketAddr>, FetchError>;
}

#[async_trait]
pub trait Fetcher: Send + Sync {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResult, FetchError>;
}

#[derive(Clone, Default)]
pub struct SystemDnsResolver;

#[async_trait]
impl DnsResolver for SystemDnsResolver {
    async fn resolve(&self, host: &str, port: u16) -> Result<Vec<SocketAddr>, FetchError> {
        tokio::net::lookup_host((host, port))
            .await
            .map(|values| values.collect())
            .map_err(|_| FetchError::Dns)
    }
}

#[derive(Clone)]
pub struct SafeFetcher<R = SystemDnsResolver> {
    resolver: R,
    clients: Arc<Mutex<BTreeMap<String, reqwest::Client>>>,
}

impl Default for SafeFetcher<SystemDnsResolver> {
    fn default() -> Self {
        Self {
            resolver: SystemDnsResolver,
            clients: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }
}

impl<R: DnsResolver> SafeFetcher<R> {
    pub fn new(resolver: R) -> Self {
        Self {
            resolver,
            clients: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub async fn fetch(&self, request: FetchRequest) -> Result<FetchResult, FetchError> {
        validate_headers(&request.headers)?;
        let deadline = tokio::time::Instant::now() + Duration::from_millis(request.timeout_ms);
        let mut current = request.url;
        let mut redirects = Vec::new();
        loop {
            let url_stage = if redirects.is_empty() {
                "initial_url"
            } else {
                "redirect_url"
            };
            validate_deep_url(&current).map_err(|code| blocked_url(url_stage, code))?;
            let host = current
                .host_str()
                .ok_or_else(|| blocked_url(url_stage, "missing_host"))?;
            let port = current
                .port_or_known_default()
                .ok_or_else(|| blocked_url(url_stage, "missing_port"))?;
            let addresses = tokio::time::timeout_at(deadline, self.resolver.resolve(host, port))
                .await
                .map_err(|_| FetchError::Timeout)??;
            let ips: Vec<IpAddr> = addresses.iter().map(|value| value.ip()).collect();
            validate_resolved_addresses(&ips).map_err(|code| {
                audit_ssrf_rejection("dns_answer", code);
                FetchError::AddressBlocked
            })?;
            let client = self.client_for(host, &addresses)?;

            let response = tokio::time::timeout_at(
                deadline,
                execute_pinned(&client, &current, &request.headers, request.max_bytes),
            )
            .await
            .map_err(|_| FetchError::Timeout)??;

            if is_redirect(response.status) {
                if redirects.len() as u32 >= request.max_redirects {
                    return Err(FetchError::RedirectLimit);
                }
                let location = response.location.ok_or(FetchError::InvalidRedirect)?;
                let next = validate_redirect(&current, &location)?;
                redirects.push(current);
                current = next;
                continue;
            }
            return Ok(FetchResult {
                final_url: FinalUrl(current),
                status: response.status,
                headers_safe: response.headers_safe,
                content_type: response.content_type,
                size: response.body.len() as u64,
                body: response.body,
                redirect_chain: redirects,
                retrieved_at: now_rfc3339(),
            });
        }
    }

    fn client_for(
        &self,
        host: &str,
        addresses: &[SocketAddr],
    ) -> Result<reqwest::Client, FetchError> {
        let mut sorted_addresses = addresses.to_vec();
        sorted_addresses.sort_unstable();
        sorted_addresses.dedup();
        let key = format!("{host}|{sorted_addresses:?}");
        if let Some(client) = self
            .clients
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&key)
            .cloned()
        {
            return Ok(client);
        }
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .min_tls_version(reqwest::tls::Version::TLS_1_2)
            .no_proxy()
            .referer(false)
            .resolve_to_addrs(host, &sorted_addresses)
            .build()
            .map_err(|_| FetchError::Network)?;
        let mut clients = self
            .clients
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if clients.len() >= 64 {
            clients.clear();
        }
        clients.insert(key, client.clone());
        Ok(client)
    }
}

#[async_trait]
impl<R: DnsResolver> Fetcher for SafeFetcher<R> {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResult, FetchError> {
        SafeFetcher::fetch(self, request).await
    }
}

struct RawResponse {
    status: u16,
    location: Option<String>,
    headers_safe: BTreeMap<String, String>,
    content_type: Option<String>,
    body: Vec<u8>,
}

async fn execute_pinned(
    client: &reqwest::Client,
    url: &Url,
    allowed_headers: &BTreeMap<String, String>,
    max_bytes: u64,
) -> Result<RawResponse, FetchError> {
    let mut headers = HeaderMap::new();
    for (name, value) in allowed_headers {
        headers.insert(
            HeaderName::from_bytes(name.as_bytes()).map_err(|_| FetchError::HeaderBlocked)?,
            HeaderValue::from_str(value).map_err(|_| FetchError::HeaderBlocked)?,
        );
    }
    let mut response = client
        .get(url.clone())
        .headers(headers)
        .send()
        .await
        .map_err(|_| FetchError::Network)?;
    let status = response.status().as_u16();
    let location = response
        .headers()
        .get(LOCATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let headers_safe = safe_response_headers(response.headers());
    let content_type = headers_safe.get("content-type").cloned();
    if is_redirect(status) {
        return Ok(RawResponse {
            status,
            location,
            headers_safe,
            content_type,
            body: vec![],
        });
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| FetchError::Network)? {
        if body.len() as u64 + chunk.len() as u64 > max_bytes {
            return Err(FetchError::ByteLimit);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(RawResponse {
        status,
        location,
        headers_safe,
        content_type,
        body,
    })
}

fn validate_headers(headers: &BTreeMap<String, String>) -> Result<(), FetchError> {
    const ALLOWED: [&str; 5] = [
        "accept",
        "accept-language",
        "if-modified-since",
        "if-none-match",
        "user-agent",
    ];
    if headers
        .keys()
        .all(|name| ALLOWED.contains(&name.to_ascii_lowercase().as_str()))
    {
        Ok(())
    } else {
        Err(FetchError::HeaderBlocked)
    }
}

fn safe_response_headers(headers: &HeaderMap) -> BTreeMap<String, String> {
    const SAFE: [&str; 5] = [
        "content-type",
        "content-length",
        "last-modified",
        "etag",
        "cache-control",
    ];
    SAFE.into_iter()
        .filter_map(|name| {
            headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(|value| (name.into(), value.into()))
        })
        .collect()
}

fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

fn validate_redirect(current: &Url, location: &str) -> Result<Url, FetchError> {
    let next = current.join(location).map_err(|_| {
        audit_ssrf_rejection("redirect_location", "invalid_redirect");
        FetchError::InvalidRedirect
    })?;
    validate_deep_url(&next).map_err(|code| blocked_url("redirect_url", code))?;
    Ok(next)
}

fn blocked_url(stage: &'static str, code: &'static str) -> FetchError {
    audit_ssrf_rejection(stage, code);
    FetchError::BlockedUrl(code.to_string())
}

fn now_rfc3339() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |value| value.as_secs() as i64);
    time::OffsetDateTime::from_unix_timestamp(seconds)
        .ok()
        .and_then(|value| {
            value
                .format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .unwrap_or_else(|| seconds.to_string())
}

#[cfg(test)]
mod client_cache_tests {
    use super::*;

    #[test]
    fn reuses_pinned_client_for_the_same_host_and_dns_answer() {
        let fetcher = SafeFetcher::default();
        let addresses = ["93.184.216.34:443".parse().unwrap()];
        fetcher.client_for("example.com", &addresses).unwrap();
        fetcher.client_for("example.com", &addresses).unwrap();
        assert_eq!(
            fetcher
                .clients
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .len(),
            1
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct FixedResolver(Vec<SocketAddr>);

    #[async_trait]
    impl DnsResolver for FixedResolver {
        async fn resolve(&self, _: &str, _: u16) -> Result<Vec<SocketAddr>, FetchError> {
            Ok(self.0.clone())
        }
    }

    fn request(url: &str) -> FetchRequest {
        FetchRequest {
            url: Url::parse(url).unwrap(),
            timeout_ms: 100,
            max_bytes: 100,
            max_redirects: 1,
            headers: BTreeMap::new(),
        }
    }

    #[tokio::test]
    async fn rejects_private_dns_answer_before_connecting() {
        let fetcher = SafeFetcher::new(FixedResolver(vec!["127.0.0.1:80".parse().unwrap()]));
        assert_eq!(
            fetcher.fetch(request("http://example.com")).await,
            Err(FetchError::AddressBlocked)
        );
    }

    #[tokio::test]
    async fn rejects_blocked_url_before_dns() {
        let fetcher = SafeFetcher::new(FixedResolver(vec![]));
        assert!(matches!(
            fetcher.fetch(request("http://localhost")).await,
            Err(FetchError::BlockedUrl(_))
        ));
    }

    #[test]
    fn rejects_sensitive_request_headers() {
        let mut headers = BTreeMap::new();
        headers.insert("authorization".into(), "secret".into());
        assert_eq!(validate_headers(&headers), Err(FetchError::HeaderBlocked));
    }

    #[test]
    fn permits_only_safe_conditional_revalidation_headers() {
        let headers = BTreeMap::from([
            ("if-none-match".into(), "\"content-v1\"".into()),
            (
                "if-modified-since".into(),
                "Wed, 21 Oct 2015 07:28:00 GMT".into(),
            ),
        ]);
        assert_eq!(validate_headers(&headers), Ok(()));
    }

    #[test]
    fn rejects_redirect_to_private_address_before_second_connection() {
        let current = Url::parse("https://example.com/start").unwrap();
        assert!(matches!(
            validate_redirect(&current, "http://127.0.0.1/admin"),
            Err(FetchError::BlockedUrl(_))
        ));
    }
}
