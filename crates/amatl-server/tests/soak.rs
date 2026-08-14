// Soak / load test: sustained concurrent MCP + HTTP traffic.
//
// Exercises the full Axum stack (TCP, routing, middleware, rate limiting,
// MCP Streamable HTTP) under controlled concurrency for a fixed duration.
// Reports p50/p95/p99 latency, throughput, error rate and peak RSS.
//
// Run with:
//   cargo test --locked -p amatl-server --test soak -- --nocapture --ignored
//
// This test is #[ignore] by default (~30 s, many TCP connections).

use amatl_core::AmatlService;
use amatl_server::serve;
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;

/// MCP protocol version this surface speaks; must match `mcp::McpSurface`.
const MCP_PROTOCOL_VERSION: &str = "2026-07-28";
const DURATION_SECS: u64 = 15;
const CONCURRENCY: usize = 16;
const WARMUP_SECS: u64 = 2;

#[derive(Debug, Default)]
struct Stats {
    total: u64,
    ok: u64,
    errors: u64,
    latencies_us: Vec<u64>,
}

impl Stats {
    fn record(&mut self, ok: bool, latency_us: u64) {
        self.total += 1;
        if ok {
            self.ok += 1;
        } else {
            self.errors += 1;
        }
        self.latencies_us.push(latency_us);
    }

    fn p50(&self) -> u64 {
        percentile(&self.latencies_us, 50)
    }
    fn p95(&self) -> u64 {
        percentile(&self.latencies_us, 95)
    }
    fn p99(&self) -> u64 {
        percentile(&self.latencies_us, 99)
    }
    fn max(&self) -> u64 {
        self.latencies_us.iter().copied().max().unwrap_or(0)
    }
    fn throughput(&self, elapsed: Duration) -> f64 {
        self.total as f64 / elapsed.as_secs_f64()
    }
    fn error_rate(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.errors as f64 / self.total as f64
        }
    }
}

fn percentile(data: &[u64], p: u8) -> u64 {
    if data.is_empty() {
        return 0;
    }
    let mut sorted: Vec<u64> = data.to_vec();
    sorted.sort_unstable();
    let idx = ((p as f64 / 100.0) * (sorted.len() - 1) as f64).round() as usize;
    sorted[idx]
}

fn peak_rss() -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find(|line| line.starts_with("VmHWM:"))?
        .split_whitespace()
        .nth(1)?
        .parse::<u64>()
        .ok()
        .map(|kb| kb * 1024)
}

async fn spawn_server() -> u16 {
    let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reservation.local_addr().unwrap().port();
    drop(reservation);

    let mut config = amatl_core::Config::default();
    config.server.port = port;
    config.server.no_auth = true;
    config.server.rate_limit_per_minute = 1_000_000;

    let service = AmatlService::new(config, true).await;
    tokio::spawn(serve(service));

    for _ in 0..100 {
        let result: Result<_, std::io::Error> = TcpStream::connect(("127.0.0.1", port)).await;
        match result {
            Ok(_) => return port,
            Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
        }
    }
    panic!("server did not start on port {port}");
}

async fn http_get(port: u16, path: &str) -> (bool, u64) {
    let start = Instant::now();
    let result: Result<bool, std::io::Error> = async {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await?;
        let request =
            format!("GET {path} HTTP/1.1\r\nHost: localhost:{port}\r\nConnection: close\r\n\r\n");
        tokio::io::AsyncWriteExt::write_all(&mut stream, request.as_bytes()).await?;
        let mut response = Vec::new();
        tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut response))
            .await
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout"))??;
        let text = String::from_utf8_lossy(&response);
        Ok(text.starts_with("HTTP/1.1 200"))
    }
    .await;
    let elapsed = start.elapsed().as_micros() as u64;
    (result.unwrap_or(false), elapsed)
}

async fn mcp_list_tools(port: u16) -> (bool, u64) {
    let start = Instant::now();
    // The MCP surface negotiates a protocol version. Omitting the
    // `mcp-protocol-version` header and the `_meta` block makes the server
    // reject every call, which is exactly what happened while this test was
    // `#[ignore]`d and never executed: it reported a steady 33% error rate,
    // one third being precisely the share of MCP requests below.
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/list",
        "params": {
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": MCP_PROTOCOL_VERSION,
                "io.modelcontextprotocol/clientInfo": {
                    "name": "amatl-soak",
                    "version": "1"
                },
                "io.modelcontextprotocol/clientCapabilities": {}
            }
        }
    });
    let body_str = body.to_string();
    let result: Result<bool, std::io::Error> = async {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await?;
        let request = format!(
            "POST /mcp HTTP/1.1\r\n\
             Host: localhost:{port}\r\n\
             Content-Type: application/json\r\n\
             Accept: application/json, text/event-stream\r\n\
             mcp-protocol-version: {MCP_PROTOCOL_VERSION}\r\n\
             mcp-method: tools/list\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n\
             {body_str}",
            body_str.len(),
        );
        tokio::io::AsyncWriteExt::write_all(&mut stream, request.as_bytes()).await?;
        let mut response = Vec::new();
        tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut response))
            .await
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout"))??;
        let text = String::from_utf8_lossy(&response);
        Ok(text.contains("200 OK") && text.contains("\"tools\""))
    }
    .await;
    let elapsed = start.elapsed().as_micros() as u64;
    (result.unwrap_or(false), elapsed)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "soak test: run with --ignored"]
async fn sustained_concurrent_mcp_and_http_traffic() {
    let port = spawn_server().await;

    // Warmup
    let warmup_deadline = Instant::now() + Duration::from_secs(WARMUP_SECS);
    while Instant::now() < warmup_deadline {
        let _ = http_get(port, "/health").await;
        let _ = mcp_list_tools(port).await;
    }

    let stats = Arc::new(tokio::sync::Mutex::new(Stats::default()));
    let deadline = Instant::now() + Duration::from_secs(DURATION_SECS);

    let mut handles = Vec::new();
    for _ in 0..CONCURRENCY {
        let stats = stats.clone();
        let handle = tokio::spawn(async move {
            let mut counter = 0u64;
            while Instant::now() < deadline {
                let (ok, latency) = if counter.is_multiple_of(3) {
                    mcp_list_tools(port).await
                } else {
                    http_get(port, "/health").await
                };
                stats.lock().await.record(ok, latency);
                counter += 1;
                tokio::task::yield_now().await;
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.await.unwrap();
    }

    let stats = stats.lock().await;
    let elapsed = Duration::from_secs(DURATION_SECS);

    eprintln!("── soak results ──────────────────────────────");
    eprintln!("duration:       {DURATION_SECS}s");
    eprintln!("concurrency:    {CONCURRENCY}");
    eprintln!("total requests: {}", stats.total);
    eprintln!("ok:             {}", stats.ok);
    eprintln!("errors:         {}", stats.errors);
    eprintln!("error rate:     {:.4}", stats.error_rate());
    eprintln!("throughput:     {:.2} req/s", stats.throughput(elapsed));
    eprintln!("p50 latency:    {} µs", stats.p50());
    eprintln!("p95 latency:    {} µs", stats.p95());
    eprintln!("p99 latency:    {} µs", stats.p99());
    eprintln!("max latency:    {} µs", stats.max());
    if let Some(rss) = peak_rss() {
        eprintln!("peak RSS:       {rss} bytes");
    }
    eprintln!("──────────────────────────────────────────────");

    assert!(stats.total > 100, "too few requests: {}", stats.total);
    assert!(
        stats.error_rate() < 0.01,
        "error rate too high: {:.4}",
        stats.error_rate()
    );
    assert!(
        stats.p95() < 50_000,
        "p95 latency too high: {} µs",
        stats.p95()
    );
    assert!(
        stats.throughput(elapsed) > 10.0,
        "throughput too low: {:.2} req/s",
        stats.throughput(elapsed)
    );
}
