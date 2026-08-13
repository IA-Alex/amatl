use async_trait::async_trait;
use std::collections::BTreeMap;
use std::time::Duration;
use url::Url;

pub struct HttpRequest {
    pub(crate) url: Url,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) timeout_ms: u64,
}

impl HttpRequest {
    pub fn sanitized_url(&self) -> Url {
        let mut url = self.url.clone();
        let sensitive = ["api_key", "key", "token"];
        let pairs = url
            .query_pairs()
            .map(|(key, value)| {
                let value: String = if sensitive.contains(&key.as_ref()) {
                    "[redacted]".into()
                } else {
                    value.into_owned()
                };
                (key.into_owned(), value)
            })
            .collect::<Vec<_>>();
        url.query_pairs_mut().clear().extend_pairs(pairs);
        url
    }
}

#[derive(Clone, Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

#[async_trait]
pub trait HttpTransport: Send + Sync {
    async fn execute(&self, request: HttpRequest) -> Result<HttpResponse, String>;
}

pub struct ReqwestTransport {
    client: reqwest::Client,
    max_response_bytes: usize,
}

impl ReqwestTransport {
    pub fn new(max_response_bytes: usize) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "unable to initialize HTTP client".to_string())?;
        Ok(Self {
            client,
            max_response_bytes,
        })
    }
}

#[async_trait]
impl HttpTransport for ReqwestTransport {
    async fn execute(&self, request: HttpRequest) -> Result<HttpResponse, String> {
        let mut builder = self
            .client
            .get(request.url)
            .timeout(Duration::from_millis(request.timeout_ms));
        for (name, value) in request.headers {
            builder = builder.header(&name, &value);
        }
        let mut response = builder
            .send()
            .await
            .map_err(|_| "provider network request failed".to_string())?;
        if response.content_length().unwrap_or(0) > self.max_response_bytes as u64 {
            return Err("provider response exceeded byte limit".into());
        }
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.as_str().to_ascii_lowercase(), value.to_string()))
            })
            .collect();
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "provider response body failed".to_string())?
        {
            if body.len().saturating_add(chunk.len()) > self.max_response_bytes {
                return Err("provider response exceeded byte limit".into());
            }
            body.extend_from_slice(&chunk);
        }
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn sanitizes_secret_query_parameters() {
        let request = HttpRequest {
            url: Url::parse("https://example.com/?api_key=secret&q=rust").unwrap(),
            headers: vec![],
            timeout_ms: 10,
        };
        let visible = request.sanitized_url().to_string();
        assert!(!visible.contains("secret"));
        assert!(visible.contains("rust"));
    }

    #[tokio::test]
    async fn stops_chunked_provider_response_during_download() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).await;
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n6\r\nabcdef\r\n0\r\n\r\n",
                )
                .await
                .unwrap();
        });
        let transport = ReqwestTransport::new(5).unwrap();
        let error = transport
            .execute(HttpRequest {
                url: Url::parse(&format!("http://{address}/")).unwrap(),
                headers: vec![],
                timeout_ms: 1_000,
            })
            .await
            .unwrap_err();
        assert_eq!(error, "provider response exceeded byte limit");
        server.await.unwrap();
    }
}
