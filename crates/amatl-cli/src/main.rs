use amatl_core::{
    parse_query, run_builtin_benchmark, run_operational_benchmark, validate_provider_canary,
    AmatlService, Config, DocumentCache, DocumentCachePolicy, InMemoryTelemetry, LocalIngestor,
    ProviderSearchCache, ProviderSearchCachePolicy, ProviderSurfaceStatus, SearchResponse,
    ServiceSurface, SqliteStorage,
};
use anyhow::Context;
use clap::{Parser, Subcommand};
use serde_json::{Map, Value};
use std::fmt;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::fmt::format::{FormatEvent, FormatFields, Writer};
use tracing_subscriber::fmt::{FmtContext, FormattedFields};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;

struct JsonEventFormatter;

impl<S, N> FormatEvent<S, N> for JsonEventFormatter
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
    N: for<'writer> FormatFields<'writer> + 'static,
{
    fn format_event(
        &self,
        context: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let mut visitor = JsonFieldVisitor::default();
        event.record(&mut visitor);
        let message = visitor
            .fields
            .remove("message")
            .unwrap_or_else(|| Value::String(event.metadata().name().into()));
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let mut root = Map::new();
        root.insert(
            "ts".into(),
            Value::String(format!(
                "{}.{:03}Z",
                elapsed.as_secs(),
                elapsed.subsec_millis()
            )),
        );
        root.insert(
            "level".into(),
            Value::String(event.metadata().level().as_str().into()),
        );
        root.insert(
            "target".into(),
            Value::String(event.metadata().target().into()),
        );
        root.insert("msg".into(), message);
        root.insert("context".into(), Value::Object(visitor.fields));
        if let Some(scope) = context.event_scope() {
            let spans = scope
                .from_root()
                .filter(|span| trusted_log_target(span.metadata().target()))
                .map(|span| {
                    let fields = {
                        let extensions = span.extensions();
                        extensions
                            .get::<FormattedFields<N>>()
                            .map(|fields| fields.fields.clone())
                            .unwrap_or_default()
                    };
                    serde_json::json!({
                        "name": span.metadata().name(),
                        "fields": fields,
                    })
                })
                .collect::<Vec<_>>();
            if !spans.is_empty() {
                root.insert("spans".into(), Value::Array(spans));
            }
        }
        writeln!(writer, "{}", Value::Object(root))
    }
}

#[derive(Default)]
struct JsonFieldVisitor {
    fields: Map<String, Value>,
}

impl JsonFieldVisitor {
    fn record_value(&mut self, field: &Field, value: Value) {
        let value = if sensitive_log_field(field.name()) {
            Value::String("[redacted]".into())
        } else {
            value
        };
        self.fields.insert(field.name().into(), value);
    }
}

fn sensitive_log_field(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "api_key" | "authorization" | "cookie" | "key" | "password" | "secret" | "token"
    )
}

fn trusted_log_target(target: &str) -> bool {
    target == "amatl" || target.starts_with("amatl::") || target.starts_with("amatl_")
}

fn log_filter() -> tracing_subscriber::EnvFilter {
    tracing_subscriber::EnvFilter::from_default_env().add_directive(
        "amatl::http=info"
            .parse()
            .expect("static request-correlation log directive is valid"),
    )
}

impl Visit for JsonFieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.record_value(field, Value::String(format!("{value:?}")));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.record_value(field, Value::String(value.into()));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.record_value(field, value.into());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.record_value(field, value.into());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.record_value(field, value.into());
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.record_value(
            field,
            serde_json::Number::from_f64(value).map_or(Value::Null, Value::Number),
        );
    }
}

#[cfg(test)]
mod logging_tests {
    use super::*;
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct LogWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for LogWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn json_logs_escape_newlines_and_redact_sensitive_fields() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let sink = captured.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .without_time()
            .event_format(JsonEventFormatter)
            .with_writer(move || LogWriter(sink.clone()))
            .finish();
        let hostile = "first line\nforged second event";
        let secret = "never-log-this-api-key";
        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("http_request", request_id = "request-123");
            let _entered = span.enter();
            let dependency_span = tracing::error_span!(
                target: "rmcp::service",
                "serve_inner",
                arguments = secret
            );
            let _dependency_entered = dependency_span.enter();
            tracing::warn!(
                target: "amatl::logging_test",
                message = hostile,
                api_key = secret,
                authorization = secret
            );
        });

        let output = String::from_utf8(
            captured
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone(),
        )
        .unwrap();
        assert_eq!(output.lines().count(), 1, "{output:?}");
        assert!(!output.contains(secret));
        let value: Value = serde_json::from_str(output.trim()).unwrap();
        assert_eq!(value["msg"], hostile);
        assert_eq!(value["context"]["api_key"], "[redacted]");
        assert_eq!(value["context"]["authorization"], "[redacted]");
        assert_eq!(value["spans"][0]["name"], "http_request");
        assert_eq!(value["spans"].as_array().unwrap().len(), 1);
        assert!(value["spans"][0]["fields"]
            .as_str()
            .unwrap()
            .contains("request-123"));
        assert!(trusted_log_target("amatl::security"));
        assert!(trusted_log_target("amatl_core::service"));
        assert!(!trusted_log_target("rmcp::service"));
        assert!(!trusted_log_target("hyper::proto"));
    }

    #[test]
    fn compact_logs_drop_dependency_events_and_spans() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let sink = captured.clone();
        let target_filter =
            tracing_subscriber::filter::filter_fn(|metadata| trusted_log_target(metadata.target()));
        let subscriber = tracing_subscriber::registry()
            .with(tracing_subscriber::EnvFilter::new("trace"))
            .with(
                tracing_subscriber::fmt::layer()
                    .without_time()
                    .with_ansi(false)
                    .with_writer(move || LogWriter(sink.clone()))
                    .compact()
                    .with_filter(target_filter),
            );
        let secret = "dependency-must-not-log-this";
        tracing::subscriber::with_default(subscriber, || {
            let request = tracing::error_span!("http_request", request_id = "request-456");
            let _request_entered = request.enter();
            let dependency = tracing::error_span!(
                target: "rmcp::service",
                "serve_inner",
                arguments = secret
            );
            let _dependency_entered = dependency.enter();
            tracing::warn!(target: "rmcp::service", message = secret);
            tracing::warn!(target: "amatl::security", security_event = "test_event");
        });

        let output = String::from_utf8(
            captured
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone(),
        )
        .unwrap();
        assert!(output.contains("test_event"), "{output}");
        assert!(output.contains("request-456"), "{output}");
        assert!(!output.contains("rmcp"), "{output}");
        assert!(!output.contains(secret), "{output}");
    }
}

#[derive(Parser)]
#[command(
    name = "amatl",
    version,
    about = "AMATL generalist multi-source search"
)]
struct Cli {
    #[arg(long, default_value = "amatl.toml", global = true)]
    config_file: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Search {
        query: String,
        #[arg(long)]
        json: bool,
        #[arg(long, hide = true)]
        mock: bool,
    },
    Deep {
        query: String,
        #[arg(long)]
        json: bool,
        #[arg(long, hide = true)]
        mock: bool,
    },
    Ingest {
        path: PathBuf,
        #[arg(long)]
        query: Option<String>,
        #[arg(long)]
        json: bool,
    },
    Providers,
    ProviderCanary {
        provider: String,
        query: String,
        #[arg(long)]
        json: bool,
    },
    Config,
    Cache {
        #[arg(long)]
        purge: bool,
    },
    Doctor,
    Benchmark {
        component: String,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 64)]
        iterations: usize,
        #[arg(long, default_value_t = 8)]
        concurrency: usize,
    },
    Serve {
        #[arg(long, hide = true)]
        mock: bool,
    },
    Mcp {
        #[command(subcommand)]
        command: McpCommand,
    },
}

#[derive(Subcommand)]
enum McpCommand {
    Serve {
        #[arg(long, hide = true)]
        mock: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_logging();
    let cli = Cli::parse();
    let config = Config::load_optional(&cli.config_file).context("configuration failed")?;
    match cli.command {
        Command::Search { query, json, mock } => search(query, json, mock, &config).await,
        Command::Deep { query, json, mock } => deep(query, json, mock, &config).await,
        Command::Ingest { path, query, json } => ingest(path, query, json, &config).await,
        Command::Providers => {
            print_providers(&config).await?;
            Ok(())
        }
        Command::ProviderCanary {
            provider,
            query,
            json,
        } => provider_canary(provider, query, json, &config).await,
        Command::Config => {
            println!("config_file = {}", cli.config_file.display());
            print_data_policy(&config);
            println!("providers.enabled = {:?}", config.providers.enabled);
            println!("timeouts.provider_ms = {}", config.timeouts.provider_ms);
            println!("timeouts.global_ms = {}", config.timeouts.global_ms);
            println!(
                "budget.max_provider_calls = {}",
                config.budget.max_provider_calls
            );
            println!("ranking_policy.version = {}", config.ranking_policy.version);
            println!(
                "diversity_policy.version = {}",
                config.diversity_policy.version
            );
            println!("search_policy.version = {}", config.search_policy.version);
            println!(
                "deep.ranking_v2.enabled = {}",
                config.deep.ranking_v2.enabled
            );
            println!("deep.gaps.enabled = {}", config.deep.gaps.enabled);
            println!("server.bind = {}", config.server.bind);
            println!("server.port = {}", config.server.port);
            println!("server.no_auth = {}", config.server.no_auth);
            println!("server.tls = {}", config.server.tls.cert_path.is_some());
            Ok(())
        }
        Command::Cache { purge } => {
            cache_command(&config, purge).await;
            Ok(())
        }
        Command::Doctor => {
            println!("core: ok");
            print_data_policy(&config);
            print_providers(&config).await?;
            doctor_persistence(&config).await;
            doctor_server(&config);
            Ok(())
        }
        Command::Benchmark {
            component,
            json,
            iterations,
            concurrency,
        } => benchmark_command(&component, json, iterations, concurrency, &config).await,
        Command::Serve { mock } => {
            amatl_server::serve(AmatlService::new(config, mock).await).await?;
            Ok(())
        }
        Command::Mcp {
            command: McpCommand::Serve { mock },
        } => {
            amatl_server::serve(AmatlService::new(config, mock).await).await?;
            Ok(())
        }
    }
}

fn print_data_policy(config: &Config) {
    println!(
        "data_policy.profile = {}",
        config.data_policy.profile.as_str()
    );
    println!(
        "data_policy.egress = {}",
        config.data_policy.egress.as_str()
    );
    println!(
        "data_policy.inference = {}",
        config.data_policy.inference.as_str()
    );
    println!(
        "data_policy.network_egress_allowed = {}",
        config.data_policy.allows_network_egress()
    );
    println!(
        "data_policy.remote_inference_allowed = {}",
        config.data_policy.allows_remote_inference()
    );
}

async fn provider_canary(
    provider: String,
    query: String,
    json: bool,
    config: &Config,
) -> anyhow::Result<()> {
    validate_provider_canary(config, &provider)?;
    let mut isolated = config.clone();
    isolated.providers.enabled = vec![provider.clone()];
    let execution = AmatlService::new(isolated, false)
        .await
        .search(query, ServiceSurface::Cli)
        .await?;
    if execution.response.status == amatl_core::SearchStatus::Failure
        || !execution.response.providers_used.contains(&provider)
    {
        anyhow::bail!("provider canary failed without a usable {provider} response");
    }
    print_search(execution.response, json)
}

fn init_logging() {
    if std::io::stderr().is_terminal() {
        let target_filter =
            tracing_subscriber::filter::filter_fn(|metadata| trusted_log_target(metadata.target()));
        let _ = tracing_subscriber::registry()
            .with(log_filter())
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(std::io::stderr)
                    .with_target(true)
                    .compact()
                    .with_filter(target_filter),
            )
            .try_init();
    } else {
        let target_filter =
            tracing_subscriber::filter::filter_fn(|metadata| trusted_log_target(metadata.target()));
        let _ = tracing_subscriber::registry()
            .with(log_filter())
            .with(
                tracing_subscriber::fmt::layer()
                    .event_format(JsonEventFormatter)
                    .with_writer(std::io::stderr)
                    .with_filter(target_filter),
            )
            .try_init();
    }
}

fn doctor_server(config: &Config) {
    let token = std::env::var(&config.server.token_env)
        .ok()
        .is_some_and(|value| value.len() >= 32);
    println!(
        "server: {}:{} auth={} tls={}",
        config.server.bind,
        config.server.port,
        if config.server.no_auth {
            "development-no-auth"
        } else if token {
            "token-ready"
        } else {
            "token-missing"
        },
        if config.server.tls.cert_path.is_some() {
            "configured"
        } else {
            "disabled"
        }
    );
}

async fn search(raw_query: String, json: bool, mock: bool, config: &Config) -> anyhow::Result<()> {
    let execution = AmatlService::new(config.clone(), mock)
        .await
        .search(raw_query, ServiceSurface::Cli)
        .await?;
    let failed = execution.response.status == amatl_core::SearchStatus::Failure;
    print_search(execution.response, json)?;
    if failed {
        anyhow::bail!("search failed");
    }
    Ok(())
}

fn print_search(response: SearchResponse, json: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        println!(
            "status: {} ({} ms)",
            response.status.as_str(),
            response.elapsed_ms
        );
        for degradation in &response.degradations {
            println!(
                "degradation: {} ({})",
                degradation.message, degradation.component
            );
        }
        for result in response.results {
            println!(
                "{}. {}\n   {}",
                result.rank.get(),
                result.title.as_deref().unwrap_or(&result.domain),
                result.canonical_url.0
            );
        }
    }
    Ok(())
}

async fn deep(raw_query: String, json: bool, mock: bool, config: &Config) -> anyhow::Result<()> {
    let deep = AmatlService::new(config.clone(), mock)
        .await
        .deep(raw_query, ServiceSurface::Cli)
        .await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&deep)?);
    } else {
        println!(
            "deep: {} documents ({} ms)",
            deep.documents.len(),
            deep.elapsed_ms
        );
        for degradation in deep.degradations {
            println!(
                "degradation: {} ({})",
                degradation.message, degradation.code
            );
        }
        for document in deep.documents {
            println!("{:?}\t{}", document.status, document.final_url);
        }
    }
    Ok(())
}

async fn ingest(
    path: PathBuf,
    raw_query: Option<String>,
    json: bool,
    config: &Config,
) -> anyhow::Result<()> {
    let query = raw_query
        .map(parse_query)
        .transpose()
        .context("query parsing failed")?;
    let response = LocalIngestor::new(&config.data_policy)
        .ingest(&path, query.as_ref())
        .await
        .context("local ingestion failed")?;
    if json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        println!(
            "ingested: {} ({} source bytes, {} ms)",
            response.document_type.as_str(),
            response.document.size,
            response.elapsed_ms
        );
        println!("source: {}", response.document.final_url);
        println!(
            "extractor: {}",
            response
                .document
                .extractor_used
                .as_deref()
                .unwrap_or("none")
        );
        println!(
            "evidence fragments: {}",
            response.evidence_v2.fragments.len()
        );
    }
    Ok(())
}

async fn benchmark_command(
    component: &str,
    json: bool,
    iterations: usize,
    concurrency: usize,
    config: &Config,
) -> anyhow::Result<()> {
    match component {
        "ranking-v2" => {
            let report = run_builtin_benchmark(&config.deep.ranking_v2.policy);
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("benchmark: {}", report.benchmark_id);
                println!("queries: {}", report.query_count);
                println!("baseline_ndcg@3: {:.6}", report.baseline_ndcg_at_3);
                println!("candidate_ndcg@3: {:.6}", report.candidate_ndcg_at_3);
                println!("ndcg_delta: {:.6}", report.ndcg_delta);
                println!("baseline_mrr: {:.6}", report.baseline_mrr);
                println!("candidate_mrr: {:.6}", report.candidate_mrr);
                println!("passed: {}", report.passed);
            }
            if !report.passed {
                anyhow::bail!("Ranking v2 did not pass its quality gate");
            }
        }
        "operational" => {
            let report = run_operational_benchmark(iterations, concurrency).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!(
                    "operational: {} iterations, concurrency {}",
                    report.iterations, report.concurrency
                );
                println!(
                    "search latency ms p50={:.3} p95={:.3} p99={:.3}",
                    report.search.latency.p50_ms,
                    report.search.latency.p95_ms,
                    report.search.latency.p99_ms
                );
                println!(
                    "search throughput={:.2} req/s partial_rate={:.3} failure_rate={:.3}",
                    report.search.throughput_requests_per_second,
                    report.search.partial_rate,
                    report.search.failure_rate
                );
                println!(
                    "deep latency ms p50={:.3} p95={:.3} p99={:.3}",
                    report.deep_latency.p50_ms,
                    report.deep_latency.p95_ms,
                    report.deep_latency.p99_ms
                );
                println!(
                    "sqlite cold p95={:.3} ms warm p95={:.3} ms hit_rate={:.3} write_success={:.3}",
                    report.sqlite.cold_write_latency.p95_ms,
                    report.sqlite.warm_read_latency.p95_ms,
                    report.sqlite.warm_hit_rate,
                    report.sqlite.write_success_rate
                );
                if let Some(bytes) = report.peak_rss_bytes {
                    println!("peak_rss_bytes={bytes}");
                }
            }
        }
        _ => anyhow::bail!("unsupported benchmark component: {component}"),
    }
    Ok(())
}

async fn optional_storage(config: &Config) -> Option<SqliteStorage> {
    if !config.persistence.enabled {
        return None;
    }
    SqliteStorage::open(&config.persistence.path).await.ok()
}

async fn cache_command(config: &Config, purge: bool) {
    let Some(storage) = optional_storage(config).await else {
        println!("provider search cache: unavailable or disabled");
        return;
    };
    let cache = ProviderSearchCache::new(
        storage.clone(),
        ProviderSearchCachePolicy {
            enabled: config.cache.provider_search.enabled,
            ttl_seconds: config.cache.provider_search.ttl_seconds,
            max_entries: config.cache.provider_search.max_entries,
            max_bytes: config.cache.provider_search.max_bytes,
        },
    );
    let documents = DocumentCache::new(
        storage,
        DocumentCachePolicy {
            enabled: config.cache.document.enabled,
            ttl_seconds: config.cache.document.ttl_seconds,
            max_entries: config.cache.document.max_entries,
            max_bytes: config.cache.document.max_bytes,
            store_content: config.cache.document.store_content,
        },
    );
    if purge {
        println!(
            "provider search cache purged: {} entries",
            cache.purge().await
        );
        println!("document cache purged: {} entries", documents.purge().await);
    } else {
        let stats = cache.stats().await;
        println!(
            "provider search cache: {} entries, {} bytes",
            stats.entries, stats.size_bytes
        );
        let stats = documents.stats().await;
        println!(
            "document cache: {} entries, {} bytes",
            stats.entries, stats.size_bytes
        );
    }
}

async fn doctor_persistence(config: &Config) {
    let storage = optional_storage(config).await;
    match &storage {
        Some(storage) => match storage.health().await {
            Ok(health) => println!(
                "sqlite: ok (journal={}, synchronous={}, busy_timeout_ms={}, migration={}, pool={})",
                health.journal_mode,
                health.synchronous,
                health.busy_timeout_ms,
                health.migration_version,
                health.pool_size
            ),
            Err(error) => println!("sqlite: degraded ({error})"),
        },
        None if config.persistence.enabled => {
            println!("sqlite: unavailable; Search remains operational")
        }
        None => println!("sqlite: disabled"),
    }
    println!(
        "telemetry: in-memory{}",
        if config.telemetry.persistence_enabled {
            " + optional SQLite"
        } else {
            ""
        }
    );
    if config.telemetry.persistence_enabled {
        let telemetry = InMemoryTelemetry::with_optional_storage(storage).await;
        let snapshots = telemetry.snapshots(amatl_core::telemetry::now_unix());
        if snapshots.is_empty() {
            println!("provider health: Bootstrap (no persisted samples)");
        } else {
            for snapshot in snapshots {
                println!(
                    "provider health: {} {:?} {:?} (sample={})",
                    snapshot.provider, snapshot.health, snapshot.state, snapshot.sample
                );
            }
        }
    }
}

async fn print_providers(config: &Config) -> anyhow::Result<()> {
    let service = AmatlService::new(config.clone(), false).await;
    for provider in service.provider_summaries()? {
        match provider.status {
            ProviderSurfaceStatus::Available => println!("{}\tavailable", provider.name),
            ProviderSurfaceStatus::Unavailable => println!(
                "{}\tunavailable\t{}",
                provider.name,
                provider.code.as_deref().unwrap_or("provider_unavailable")
            ),
        }
    }
    Ok(())
}
