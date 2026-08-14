use async_trait::async_trait;
use std::collections::BTreeMap;
use std::time::Duration;
use url::Url;

pub struct HttpRequest {
    pub(crate) url: Url,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) timeout_ms: u64,
    /// Request body. `None` is a GET; `Some` is a POST with the declared
    /// content type already present in `headers`.
    pub(crate) body: Option<Vec<u8>>,
}

impl HttpRequest {
    /// Read-only request, the shape every search provider uses.
    pub fn get(url: Url, headers: Vec<(String, String)>, timeout_ms: u64) -> Self {
        Self {
            url,
            headers,
            timeout_ms,
            body: None,
        }
    }

    /// JSON POST, used by the governed remote inference backend.
    pub fn post_json(
        url: Url,
        headers: Vec<(String, String)>,
        timeout_ms: u64,
        body: Vec<u8>,
    ) -> Self {
        let mut headers = headers;
        headers.push(("content-type".into(), "application/json".into()));
        Self {
            url,
            headers,
            timeout_ms,
            body: Some(body),
        }
    }

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
            .min_tls_version(reqwest::tls::Version::TLS_1_2)
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
        let mut builder = match request.body {
            Some(body) => self.client.post(request.url).body(body),
            None => self.client.get(request.url),
        }
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
        let request = HttpRequest::get(
            Url::parse("https://example.com/?api_key=secret&q=rust").unwrap(),
            vec![],
            10,
        );
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
            .execute(HttpRequest::get(
                Url::parse(&format!("http://{address}/")).unwrap(),
                vec![],
                1_000,
            ))
            .await
            .unwrap_err();
        assert_eq!(error, "provider response exceeded byte limit");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn rejects_untrusted_provider_certificate_without_leaking_credentials() {
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("amatl-provider-tls-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let cert_path = directory.join("cert.pem");
        let key_path = directory.join("key.pem");
        std::fs::write(&cert_path, cert.pem()).unwrap();
        std::fs::write(&key_path, signing_key.serialize_pem()).unwrap();

        let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = reservation.local_addr().unwrap().port();
        drop(reservation);
        let tls = axum_server::tls_rustls::RustlsConfig::from_pem_file(&cert_path, &key_path)
            .await
            .unwrap();
        let app = axum::Router::new().route("/", axum::routing::get(|| async { "ok" }));
        let address = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        let server =
            tokio::spawn(axum_server::bind_rustls(address, tls).serve(app.into_make_service()));
        for _ in 0..50 {
            if tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_ok()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let secret = "never-leak-provider-key";
        let error = ReqwestTransport::new(1024)
            .unwrap()
            .execute(HttpRequest::get(
                Url::parse(&format!("https://localhost:{port}/?api_key={secret}")).unwrap(),
                vec![],
                1_000,
            ))
            .await
            .unwrap_err();
        assert_eq!(error, "provider network request failed");
        assert!(!error.contains(secret));

        server.abort();
        let _ = server.await;
        std::fs::remove_file(cert_path).unwrap();
        std::fs::remove_file(key_path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
