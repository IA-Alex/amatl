use amatl_core::{
    parse_query, run_builtin_benchmark, run_operational_benchmark, validate_provider_canary,
    AmatlService, Config, DocumentCache, DocumentCachePolicy, ErrorCode, InMemoryTelemetry,
    LocalIngestor, ProviderSearchCache, ProviderSearchCachePolicy, ProviderSurfaceStatus,
    SearchResponse, ServiceSurface, SqliteStorage, TrafilaturaExtractor,
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
    /// Local search history recorded by this machine.
    History {
        #[command(subcommand)]
        command: HistoryCommand,
    },
    /// Documents saved for cross-session reuse.
    Saved {
        #[command(subcommand)]
        command: SavedCommand,
    },
    /// SQLite maintenance: health, backups, restore, downgrade and breakers.
    Db {
        #[command(subcommand)]
        command: DbCommand,
    },
    /// Serve the UI, REST API and MCP surface on one listener.
    Serve {
        #[command(flatten)]
        listener: ListenerArgs,
        #[arg(long, hide = true)]
        mock: bool,
    },
    /// MCP surface commands.
    Mcp {
        #[command(subcommand)]
        command: McpCommand,
    },
}

/// Listener overrides shared by `serve` and `mcp serve`.
///
/// They override the configuration file for this process only; nothing is
/// written back, so a temporary port never becomes permanent state.
#[derive(clap::Args, Clone, Default)]
struct ListenerArgs {
    /// Bind address override, for example 127.0.0.1.
    #[arg(long)]
    bind: Option<String>,
    /// TCP port override.
    #[arg(long)]
    port: Option<u16>,
    /// Print one JSON object describing the listener before serving.
    #[arg(long)]
    json: bool,
}

#[derive(Subcommand)]
enum McpCommand {
    /// Serve MCP over Streamable HTTP on the shared listener.
    Serve {
        #[command(flatten)]
        listener: ListenerArgs,
        #[arg(long, hide = true)]
        mock: bool,
    },
}

#[derive(Subcommand)]
enum HistoryCommand {
    /// List recorded searches, newest first.
    List {
        #[arg(long, default_value_t = 20)]
        limit: u32,
        #[arg(long, default_value_t = 0)]
        offset: u32,
        #[arg(long)]
        json: bool,
    },
    /// Delete one entry by id.
    Delete { id: i64 },
    /// Delete every recorded search.
    Purge,
}

#[derive(Subcommand)]
enum SavedCommand {
    /// List saved documents, newest first.
    List {
        #[arg(long, default_value_t = 20)]
        limit: u32,
        #[arg(long, default_value_t = 0)]
        offset: u32,
        #[arg(long)]
        json: bool,
    },
    /// Print the stored payload of one saved document.
    Show { id: i64 },
    /// Delete one saved document by id.
    Delete { id: i64 },
}

#[derive(Subcommand)]
enum DbCommand {
    /// Report journal mode, migration version and pool size.
    Health {
        #[arg(long)]
        json: bool,
    },
    /// List the automatic backups taken before migrations and downgrades.
    Backups,
    /// Replace the database with a backup file. The caller must stop other
    /// AMATL processes first.
    Restore {
        /// Backup file produced by a migration, downgrade or restore.
        backup: PathBuf,
    },
    /// Roll the schema back to an older migration version.
    Downgrade {
        /// Target `user_version`; must be lower than the current one.
        #[arg(long)]
        to: i64,
    },
    /// Show provider circuit breaker state, optionally closing every circuit.
    Circuits {
        #[arg(long)]
        reset: bool,
        #[arg(long)]
        json: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_logging();
    let outcome = run().await;
    if let Err(error) = &outcome {
        report_error_code(error);
    }
    outcome
}

/// Print the shared catalog code for a failure, when the cause carries one.
///
/// The CLI keeps its human message, but an operator or a script comparing
/// behavior across surfaces gets the same stable identifier the API and MCP
/// return, on stderr and separate from JSON output on stdout.
fn report_error_code(error: &anyhow::Error) {
    let code = error
        .downcast_ref::<amatl_core::ServiceError>()
        .map(amatl_core::ServiceError::code)
        .or_else(|| {
            error
                .downcast_ref::<amatl_core::ProviderCanaryError>()
                .map(amatl_core::ProviderCanaryError::code)
        })
        .or_else(|| {
            error
                .downcast_ref::<amatl_core::ConfigError>()
                .map(|_| ErrorCode::ConfigurationInvalid)
        })
        .or_else(|| {
            error
                .downcast_ref::<amatl_core::StorageError>()
                .map(|_| ErrorCode::StorageUnavailable)
        });
    if let Some(code) = code {
        eprintln!("error_code={} message={}", code.as_str(), code.message());
    }
}

async fn run() -> anyhow::Result<()> {
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
            println!("providers.declared = {:?}", config.providers.names());
            println!("providers.enabled = {:?}", config.providers.enabled);
            println!("inference.backend = {}", config.inference.backend);
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
            println!(
                "circuit_breaker.enabled = {}",
                config.circuit_breaker.enabled
            );
            println!(
                "persistence.enabled = {} (history={})",
                config.persistence.enabled, config.persistence.history_enabled
            );
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
            doctor_inference(&config).await;
            doctor_persistence(&config).await;
            doctor_extractor(&config).await;
            doctor_server(&config);
            Ok(())
        }
        Command::Benchmark {
            component,
            json,
            iterations,
            concurrency,
        } => benchmark_command(&component, json, iterations, concurrency, &config).await,
        Command::History { command } => history_command(command, &config).await,
        Command::Saved { command } => saved_command(command, &config).await,
        Command::Db { command } => db_command(command, &config).await,
        Command::Serve { listener, mock } => {
            serve_command(config, listener, mock, cli.config_file).await
        }
        Command::Mcp {
            command: McpCommand::Serve { listener, mock },
        } => serve_command(config, listener, mock, cli.config_file).await,
    }
}

/// Start the shared listener that serves UI, REST and MCP.
async fn serve_command(
    mut config: Config,
    listener: ListenerArgs,
    mock: bool,
    config_file: PathBuf,
) -> anyhow::Result<()> {
    if let Some(bind) = listener.bind {
        config.server.bind = bind;
    }
    if let Some(port) = listener.port {
        config.server.port = port;
    }
    config.validate().context("configuration failed")?;
    if listener.json {
        println!(
            "{}",
            serde_json::json!({
                "schema_version": amatl_core::SCHEMA_VERSION,
                "event": "listening",
                "bind": config.server.bind,
                "port": config.server.port,
                "tls": config.server.tls.cert_path.is_some(),
                "authenticated": !config.server.no_auth,
                "config_file": config_file.display().to_string(),
                "reload": ["POST /reload", "SIGHUP"],
                "surfaces": ["ui", "rest", "mcp"],
            })
        );
    }
    amatl_server::serve_with_config_path(AmatlService::new(config, mock).await, Some(config_file))
        .await?;
    Ok(())
}

/// Service handle for the local domain commands, which need persistence.
async fn domain_service(config: &Config) -> anyhow::Result<AmatlService> {
    anyhow::ensure!(
        config.persistence.enabled,
        "this command requires [persistence] enabled = true"
    );
    Ok(AmatlService::new(config.clone(), false).await)
}

async fn history_command(command: HistoryCommand, config: &Config) -> anyhow::Result<()> {
    let service = domain_service(config).await?;
    match command {
        HistoryCommand::List {
            limit,
            offset,
            json,
        } => {
            let entries = service.history(limit, offset).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&entries)?);
            } else if entries.is_empty() {
                println!("history: empty");
            } else {
                for entry in entries {
                    println!(
                        "{}\t{}\t{} results\t{}",
                        entry.id, entry.surface, entry.total_results, entry.raw_query
                    );
                }
            }
        }
        HistoryCommand::Delete { id } => {
            anyhow::ensure!(
                service.delete_history_entry(id).await?,
                "no history entry with id {id}"
            );
            println!("history entry {id} deleted");
        }
        HistoryCommand::Purge => {
            println!("history purged: {} entries", service.purge_history().await?);
        }
    }
    Ok(())
}

async fn saved_command(command: SavedCommand, config: &Config) -> anyhow::Result<()> {
    let service = domain_service(config).await?;
    match command {
        SavedCommand::List {
            limit,
            offset,
            json,
        } => {
            let documents = service.saved_documents(limit, offset).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&documents)?);
            } else if documents.is_empty() {
                println!("saved documents: none");
            } else {
                for document in documents {
                    println!(
                        "{}\t{} bytes\t{}",
                        document.id, document.size_bytes, document.canonical_url
                    );
                }
            }
        }
        SavedCommand::Show { id } => {
            let document = service
                .saved_documents(200, 0)
                .await?
                .into_iter()
                .find(|document| document.id == id)
                .ok_or_else(|| anyhow::anyhow!("no saved document with id {id}"))?;
            println!("{}", document.payload);
        }
        SavedCommand::Delete { id } => {
            anyhow::ensure!(
                service.delete_saved_document(id).await?,
                "no saved document with id {id}"
            );
            println!("saved document {id} deleted");
        }
    }
    Ok(())
}

async fn db_command(command: DbCommand, config: &Config) -> anyhow::Result<()> {
    match command {
        DbCommand::Health { json } => {
            let storage = optional_storage(config)
                .await
                .ok_or_else(|| anyhow::anyhow!("SQLite persistence is disabled or unavailable"))?;
            let health = storage.health().await?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "path": health.path.display().to_string(),
                        "journal_mode": health.journal_mode,
                        "synchronous": health.synchronous,
                        "busy_timeout_ms": health.busy_timeout_ms,
                        "migration_version": health.migration_version,
                        "code_migration_version": amatl_core::MIGRATION_VERSION,
                        "pool_size": health.pool_size,
                    })
                );
            } else {
                println!("path: {}", health.path.display());
                println!("journal_mode: {}", health.journal_mode);
                println!("synchronous: {}", health.synchronous);
                println!("busy_timeout_ms: {}", health.busy_timeout_ms);
                println!(
                    "migration_version: {} (binary expects {})",
                    health.migration_version,
                    amatl_core::MIGRATION_VERSION
                );
                println!("pool_size: {}", health.pool_size);
            }
        }
        DbCommand::Backups => {
            let backups =
                SqliteStorage::list_backups(std::path::Path::new(&config.persistence.path))?;
            if backups.is_empty() {
                println!("backups: none");
            }
            for backup in backups {
                println!("{}", backup.display());
            }
        }
        DbCommand::Restore { backup } => {
            anyhow::ensure!(backup.exists(), "backup file does not exist");
            SqliteStorage::restore_from_backup(&config.persistence.path, &backup).await?;
            println!(
                "restored {} from {}",
                config.persistence.path,
                backup.display()
            );
        }
        DbCommand::Downgrade { to } => {
            let storage = optional_storage(config)
                .await
                .ok_or_else(|| anyhow::anyhow!("SQLite persistence is disabled or unavailable"))?;
            storage.downgrade_to(to).await?;
            println!(
                "downgraded to migration version {to}; a backup was written next to the database"
            );
        }
        DbCommand::Circuits { reset, json } => {
            let service = domain_service(config).await?;
            if reset {
                service.reset_circuits().await;
                println!("provider circuits closed");
                return Ok(());
            }
            let snapshots = service.circuit_snapshots();
            if json {
                println!("{}", serde_json::to_string_pretty(&snapshots)?);
            } else if snapshots.is_empty() {
                println!("provider circuits: all closed");
            } else {
                for snapshot in snapshots {
                    println!(
                        "{}\t{}\tfailures={}",
                        snapshot.provider,
                        snapshot.state.as_str(),
                        snapshot.consecutive_failures
                    );
                }
            }
        }
    }
    Ok(())
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
        .search(query, ServiceSurface::cli())
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

async fn doctor_inference(config: &Config) {
    let ranking = &config.deep.ranking_v2.policy;
    let required = config.deep.ranking_v2.enabled
        && (ranking.weight_semantic > 0.0 || ranking.weight_reranker > 0.0);
    match AmatlService::new(config.clone(), false)
        .await
        .inference_backend()
    {
        Some(backend) => {
            let remote = config.data_policy.inference == amatl_core::InferenceMode::RemoteExplicit;
            println!("inference: ready ({backend}, required={required}, remote={remote})");
            if remote {
                println!(
                    "inference.remote_endpoint = {}",
                    config
                        .inference
                        .remote_endpoint
                        .as_deref()
                        .unwrap_or("unset")
                );
            }
        }
        None if required => println!(
            "inference: unavailable ({}); ranking would degrade to lexical signals",
            config.data_policy.inference.as_str()
        ),
        None => println!("inference: {}", config.data_policy.inference.as_str()),
    }
}

async fn doctor_extractor(config: &Config) {
    let extractor = TrafilaturaExtractor::new(
        config.deep.extractor.executable.clone(),
        config.deep.extractor.version.clone(),
        config.deep.extractor.timeout_ms,
        config.deep.extractor.max_output_bytes,
    );
    match extractor.probe_version().await {
        Ok(version) => println!("extractor: ready ({version})"),
        Err(error) => println!("extractor: unavailable ({error})"),
    }
}

async fn search(raw_query: String, json: bool, mock: bool, config: &Config) -> anyhow::Result<()> {
    let execution = AmatlService::new(config.clone(), mock)
        .await
        .search(raw_query, ServiceSurface::cli())
        .await?;
    let failed = execution.response.status == amatl_core::SearchStatus::Failure;
    // A failed response is a contract outcome, not an error type: report the
    // codes it already carries instead of inventing one.
    let codes = execution
        .response
        .errors
        .iter()
        .map(|error| error.code.clone())
        .collect::<Vec<_>>();
    print_search(execution.response, json)?;
    if failed {
        if !codes.is_empty() {
            eprintln!("error_code={}", codes.join(","));
        }
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
        .deep(raw_query, ServiceSurface::cli())
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
            stale_while_revalidate_seconds: config.cache.document.stale_while_revalidate_seconds,
            // Maintenance addresses every namespace, so no model is pinned.
            model_version: None,
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
        "circuit breaker: {} (threshold={}, cooldown={}s)",
        if config.circuit_breaker.enabled {
            "enabled"
        } else {
            "disabled"
        },
        config.circuit_breaker.failure_threshold,
        config.circuit_breaker.open_seconds
    );
    println!(
        "telemetry: in-memory{}",
        if config.telemetry.persistence_enabled {
            " + optional SQLite"
        } else {
            ""
        }
    );
    if config.telemetry.persistence_enabled {
        let telemetry =
            InMemoryTelemetry::with_storage_and_retention(storage, config.telemetry.retention_days)
                .await;
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
