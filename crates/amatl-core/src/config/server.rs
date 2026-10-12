use super::*;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ServerConfig {
    pub bind: String,
    pub port: u16,
    /// Environment variable holding the single default credential.
    ///
    /// Kept for deployments with one operator: the resulting client is called
    /// `default` and carries every scope and every MCP tool. Declare
    /// `[[server.clients]]` instead to split capability between callers.
    pub token_env: String,
    pub no_auth: bool,
    /// Named credentials, each with its own scopes, MCP tool allowlist and
    /// optional expiry. A request is authorized as exactly one of them.
    pub clients: Vec<ServerClient>,
    pub allowed_hosts: Vec<String>,
    pub allowed_origins: Vec<String>,
    /// Maximum request body size accepted by the HTTP/MCP transport. This must
    /// be at least `persistence.saved_document_max_bytes` (plus JSON envelope
    /// overhead) or `POST /saved` will reject a payload the service itself
    /// considers valid with `413 body_too_large`.
    pub max_body_bytes: usize,
    pub max_header_bytes: usize,
    pub request_timeout_ms: u64,
    pub idle_timeout_ms: u64,
    pub rate_limit_per_minute: u32,
    pub max_connections: usize,
    pub tls: TlsConfig,
}

/// One named credential accepted by the HTTP and MCP surfaces.
///
/// The secret never lives in configuration: declare the environment variable
/// that holds it, or the SHA-256 digest of the token. Both forms are compared
/// as digests in constant time, so a configuration file leak does not leak a
/// usable credential.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ServerClient {
    /// Stable identity recorded in audit events. Never a secret.
    pub id: String,
    /// Environment variable holding this client's bearer token.
    pub token_env: Option<String>,
    /// Lowercase hex SHA-256 digest of this client's bearer token.
    pub token_sha256: Option<String>,
    /// ISO `YYYY-MM-DD` after which the credential stops being accepted.
    pub expires_at: Option<String>,
    /// HTTP capability granted to this client.
    pub scopes: Vec<Scope>,
    /// MCP tools this client may call. Empty means no MCP access at all: tool
    /// capability is granted explicitly, never inherited.
    pub tools: Vec<String>,
}

/// Capability a route requires from the calling client.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// `/search`.
    Search,
    /// `/deep`.
    Deep,
    /// Read-only operator surfaces: `/providers`, `/status`, and listing
    /// history and saved documents.
    Read,
    /// Mutating local state: saving documents, deleting or purging history.
    Write,
    /// `/reload` and any other administrative action.
    Admin,
    /// The MCP surface itself. The per-tool allowlist narrows it further.
    Mcp,
}

impl Scope {
    pub const ALL: [Scope; 6] = [
        Scope::Search,
        Scope::Deep,
        Scope::Read,
        Scope::Write,
        Scope::Admin,
        Scope::Mcp,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Search => "search",
            Self::Deep => "deep",
            Self::Read => "read",
            Self::Write => "write",
            Self::Admin => "admin",
            Self::Mcp => "mcp",
        }
    }
}

impl ServerClient {
    /// Whether the credential is still valid on `today` (`YYYY-MM-DD`).
    pub fn unexpired_on(&self, today: &str) -> bool {
        match self.expires_at.as_deref() {
            None => true,
            Some(expiry) => match (day_from_iso_date(expiry), day_from_iso_date(today)) {
                (Some(expiry), Some(today)) => today <= expiry,
                // An unparseable expiry fails closed.
                _ => false,
            },
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct TlsConfig {
    pub cert_path: Option<String>,
    pub key_path: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1".into(),
            port: 8080,
            token_env: "AMATL_SERVER_TOKEN".into(),
            no_auth: false,
            clients: vec![],
            allowed_hosts: vec!["127.0.0.1".into(), "localhost".into(), "[::1]".into()],
            allowed_origins: vec![],
            max_body_bytes: 64 * 1024,
            max_header_bytes: 16 * 1024,
            request_timeout_ms: 30_000,
            idle_timeout_ms: 30_000,
            rate_limit_per_minute: 60,
            max_connections: 64,
            tls: TlsConfig::default(),
        }
    }
}

/// Every [`ServerConfig`] field an admin-scoped caller may update in one
/// request, except `clients` — that has its own dedicated surface
/// ([`Config::upsert_server_client`]/[`Config::remove_server_client`], and
/// `/server/clients*` in `amatl-server`) because credentials need
/// create/rotate/revoke semantics no generic field patch should also try to
/// express.
///
/// Unlike every other patch type in this module, applying this one does
/// *not* mean every field takes effect once the file is written and the
/// process reloads: several of these are read once at process startup (see
/// [`ReloadKind`]) and only take full effect on restart. This patch does
/// not hide that — it is the caller's job (see `PATCH
/// /server/pending-config` in `amatl-server`) to report, per field it
/// wrote, whether [`ReloadKind::of`] says `Hot` or `Cold`.
///
/// Same "empty string clears" convention as [`InferenceConfigPatch`] on
/// `tls_cert_path`/`tls_key_path` (clearing either drops back to plaintext
/// HTTP — `Config::validate` still enforces that a remote bind requires
/// both or neither).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct ServerConfigPatch {
    pub bind: Option<String>,
    pub port: Option<u16>,
    pub token_env: Option<String>,
    pub no_auth: Option<bool>,
    pub allowed_hosts: Option<Vec<String>>,
    pub allowed_origins: Option<Vec<String>>,
    pub max_body_bytes: Option<usize>,
    pub max_header_bytes: Option<usize>,
    pub request_timeout_ms: Option<u64>,
    pub idle_timeout_ms: Option<u64>,
    pub rate_limit_per_minute: Option<u32>,
    pub max_connections: Option<usize>,
    pub tls_cert_path: Option<String>,
    pub tls_key_path: Option<String>,
}

impl ServerConfigPatch {
    /// Apply every field this patch sets to `config`, leaving `clients` and
    /// every unset field untouched. Mirrors [`Config::set_server_fields`]
    /// exactly.
    pub fn apply(&self, config: &mut ServerConfig) {
        if let Some(value) = &self.bind {
            config.bind = value.clone();
        }
        if let Some(value) = self.port {
            config.port = value;
        }
        if let Some(value) = &self.token_env {
            config.token_env = value.clone();
        }
        if let Some(value) = self.no_auth {
            config.no_auth = value;
        }
        if let Some(value) = &self.allowed_hosts {
            config.allowed_hosts = value.clone();
        }
        if let Some(value) = &self.allowed_origins {
            config.allowed_origins = value.clone();
        }
        if let Some(value) = self.max_body_bytes {
            config.max_body_bytes = value;
        }
        if let Some(value) = self.max_header_bytes {
            config.max_header_bytes = value;
        }
        if let Some(value) = self.request_timeout_ms {
            config.request_timeout_ms = value;
        }
        if let Some(value) = self.idle_timeout_ms {
            config.idle_timeout_ms = value;
        }
        if let Some(value) = self.rate_limit_per_minute {
            config.rate_limit_per_minute = value;
        }
        if let Some(value) = self.max_connections {
            config.max_connections = value;
        }
        if let Some(value) = &self.tls_cert_path {
            config.tls.cert_path = clearable(value);
        }
        if let Some(value) = &self.tls_key_path {
            config.tls.key_path = clearable(value);
        }
    }

    /// `[section, key]` pairs for every field this patch actually sets, in
    /// the shape [`ReloadKind::of`] expects — `tls_cert_path`/`tls_key_path`
    /// report as `("server.tls", "cert_path"/"key_path")`, everything else
    /// as `("server", <field name>)`. Lets a caller classify exactly what it
    /// wrote without re-deriving the field list or the `tls_*` renaming.
    pub fn changed_fields(&self) -> Vec<(&'static str, &'static str)> {
        let mut fields = Vec::new();
        macro_rules! push_if_set {
            ($field:ident, $section:expr, $key:expr) => {
                if self.$field.is_some() {
                    fields.push(($section, $key));
                }
            };
        }
        push_if_set!(bind, "server", "bind");
        push_if_set!(port, "server", "port");
        push_if_set!(token_env, "server", "token_env");
        push_if_set!(no_auth, "server", "no_auth");
        push_if_set!(allowed_hosts, "server", "allowed_hosts");
        push_if_set!(allowed_origins, "server", "allowed_origins");
        push_if_set!(max_body_bytes, "server", "max_body_bytes");
        push_if_set!(max_header_bytes, "server", "max_header_bytes");
        push_if_set!(request_timeout_ms, "server", "request_timeout_ms");
        push_if_set!(idle_timeout_ms, "server", "idle_timeout_ms");
        push_if_set!(rate_limit_per_minute, "server", "rate_limit_per_minute");
        push_if_set!(max_connections, "server", "max_connections");
        push_if_set!(tls_cert_path, "server.tls", "cert_path");
        push_if_set!(tls_key_path, "server.tls", "key_path");
        fields
    }
}

impl Config {
    /// Insert a new `[[server.clients]]` entry, or replace the one with the
    /// same `id`, and nothing else.
    ///
    /// Same guarantees as `Config::set_scalar_field`: `toml_edit`, so every
    /// comment survives; the caller is expected to have already validated a
    /// candidate config with this entry applied (in particular, that `id` is
    /// a valid, unique client key — see `Config::validate_clients`). The
    /// credential itself is never handled here: `client.token_sha256` (or
    /// `token_env`) is written exactly as given, so a caller minting a new
    /// client must hash the raw token *before* calling this and hand the raw
    /// value to the operator exactly once — it is never reconstructible from
    /// what gets written to disk.
    pub fn upsert_server_client(path: &Path, client: &ServerClient) -> Result<(), ConfigError> {
        let text = std::fs::read_to_string(path)?;
        let mut document = text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| ConfigError::Policy(format!("invalid TOML: {error}")))?;
        if document.get("server").is_none() {
            document["server"] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        let server_table = document["server"]
            .as_table_mut()
            .ok_or_else(|| ConfigError::Policy("server is not a table".into()))?;
        if server_table.get("clients").is_none() {
            server_table.insert(
                "clients",
                toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new()),
            );
        }
        let clients = server_table["clients"]
            .as_array_of_tables_mut()
            .ok_or_else(|| {
                ConfigError::Policy("server.clients is not an array of tables".into())
            })?;
        // Replacing in place at the discovered index keeps a stable read
        // order for an operator diffing the file; client order carries no
        // runtime meaning either way, since `id` is the only identity
        // `validate_clients` enforces unique.
        let table = server_client_to_table(client);
        let existing_index = clients.iter().position(|table| {
            table.get("id").and_then(|item| item.as_str()) == Some(client.id.as_str())
        });
        match existing_index.and_then(|index| clients.get_mut(index)) {
            Some(existing) => *existing = table,
            None => clients.push(table),
        }
        write_config_document(path, &document)?;
        Ok(())
    }

    /// Remove one `[[server.clients]]` entry by id, and nothing else.
    ///
    /// A no-op, not an error, when `id` is not declared — the caller already
    /// knows whether it expected a removal from the config it read before
    /// calling this.
    pub fn remove_server_client(path: &Path, id: &str) -> Result<(), ConfigError> {
        let text = std::fs::read_to_string(path)?;
        let mut document = text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| ConfigError::Policy(format!("invalid TOML: {error}")))?;
        if let Some(clients) = document
            .get_mut("server")
            .and_then(|server| server.get_mut("clients"))
            .and_then(|item| item.as_array_of_tables_mut())
        {
            clients.retain(|table| table.get("id").and_then(|item| item.as_str()) != Some(id));
        }
        write_config_document(path, &document)?;
        Ok(())
    }
    /// Write every field `patch` sets to `[server]` (and `[server.tls]` for
    /// the two TLS fields), and nothing else — `clients` is never touched
    /// here (see [`ServerConfigPatch`]). Same guarantees as
    /// [`Config::set_inference_fields`]: the caller validates a candidate
    /// first; this only performs the write. Writing does **not** mean every
    /// field takes effect without a restart — see [`ServerConfigPatch`] and
    /// [`ReloadKind`].
    pub fn set_server_fields(path: &Path, patch: &ServerConfigPatch) -> Result<(), ConfigError> {
        let text = std::fs::read_to_string(path)?;
        let mut document = text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| ConfigError::Policy(format!("invalid TOML: {error}")))?;
        {
            let table = Self::table_at(&mut document, &["server"])?;
            if let Some(value) = &patch.bind {
                table.insert("bind", toml_edit::value(value.as_str()));
            }
            if let Some(value) = patch.port {
                table.insert("port", toml_edit::value(i64::from(value)));
            }
            if let Some(value) = &patch.token_env {
                table.insert("token_env", toml_edit::value(value.as_str()));
            }
            if let Some(value) = patch.no_auth {
                table.insert("no_auth", toml_edit::value(value));
            }
            if let Some(value) = &patch.allowed_hosts {
                let mut hosts = toml_edit::Array::new();
                for host in value {
                    hosts.push(host.as_str());
                }
                table.insert("allowed_hosts", toml_edit::value(hosts));
            }
            if let Some(value) = &patch.allowed_origins {
                let mut origins = toml_edit::Array::new();
                for origin in value {
                    origins.push(origin.as_str());
                }
                table.insert("allowed_origins", toml_edit::value(origins));
            }
            if let Some(value) = patch.max_body_bytes {
                table.insert(
                    "max_body_bytes",
                    toml_edit::value(signed_integer(value, "server.max_body_bytes")?),
                );
            }
            if let Some(value) = patch.max_header_bytes {
                table.insert(
                    "max_header_bytes",
                    toml_edit::value(signed_integer(value, "server.max_header_bytes")?),
                );
            }
            if let Some(value) = patch.request_timeout_ms {
                table.insert(
                    "request_timeout_ms",
                    toml_edit::value(signed_integer(value, "server.request_timeout_ms")?),
                );
            }
            if let Some(value) = patch.idle_timeout_ms {
                table.insert(
                    "idle_timeout_ms",
                    toml_edit::value(signed_integer(value, "server.idle_timeout_ms")?),
                );
            }
            if let Some(value) = patch.rate_limit_per_minute {
                table.insert(
                    "rate_limit_per_minute",
                    toml_edit::value(signed_integer(value, "server.rate_limit_per_minute")?),
                );
            }
            if let Some(value) = patch.max_connections {
                table.insert(
                    "max_connections",
                    toml_edit::value(signed_integer(value, "server.max_connections")?),
                );
            }
        }
        if patch.tls_cert_path.is_some() || patch.tls_key_path.is_some() {
            let table = Self::table_at(&mut document, &["server", "tls"])?;
            set_clearable_string(table, "cert_path", patch.tls_cert_path.as_deref());
            set_clearable_string(table, "key_path", patch.tls_key_path.as_deref());
        }
        write_config_document(path, &document)?;
        Ok(())
    }
    /// HTTP server safety limits, TLS pairing, the named-client set (via
    /// [`Self::validate_clients`]) and the host/origin allowlists. `bind` is
    /// the already-parsed `server.bind`, passed in by [`Self::validate`].
    pub(super) fn validate_server(&self, bind: std::net::IpAddr) -> Result<(), ConfigError> {
        let tls_complete =
            self.server.tls.cert_path.is_some() && self.server.tls.key_path.is_some();
        let tls_partial = self.server.tls.cert_path.is_some() != self.server.tls.key_path.is_some();
        if self.server.port == 0
            || self.server.token_env.trim().is_empty()
            || self.server.allowed_hosts.is_empty()
            || self.server.max_body_bytes == 0
            || self.server.max_body_bytes > 1024 * 1024
            || self.server.max_header_bytes == 0
            || self.server.max_header_bytes > 64 * 1024
            || self.server.request_timeout_ms == 0
            || self.server.request_timeout_ms > 86_400_000
            || self.server.idle_timeout_ms == 0
            || self.server.idle_timeout_ms > 86_400_000
            || self.server.rate_limit_per_minute == 0
            || self.server.rate_limit_per_minute > 1_000_000
            || self.server.max_connections == 0
            || self.server.max_connections > 10_000
            || tls_partial
        {
            return Err(ConfigError::Policy(
                "invalid HTTP server safety limit".into(),
            ));
        }
        self.validate_clients()?;
        if !bind.is_loopback() && (self.server.no_auth || !tls_complete) {
            return Err(ConfigError::Policy(
                "remote HTTP bind requires authentication and TLS".into(),
            ));
        }
        if self.server.no_auth && !bind.is_loopback() {
            return Err(ConfigError::Policy(
                "no-auth development mode is loopback-only".into(),
            ));
        }
        if self.server.allowed_hosts.iter().any(|host| {
            host.trim().is_empty()
                || host.contains('*')
                || host.parse::<http::uri::Authority>().is_err()
        }) {
            return Err(ConfigError::Policy(
                "server allowed_hosts must contain exact HTTP authorities".into(),
            ));
        }
        if self.server.allowed_origins.iter().any(|origin| {
            url::Url::parse(origin).map_or(true, |url| {
                !matches!(url.scheme(), "http" | "https")
                    || url.host_str().is_none()
                    || url.path() != "/"
                    || url.query().is_some()
                    || url.fragment().is_some()
            })
        }) {
            return Err(ConfigError::Policy(
                "server allowed_origins must contain exact HTTP origins".into(),
            ));
        }
        Ok(())
    }
    /// Inference limits, and the policy that must back the ranking weights.
    ///
    /// Semantic and reranker weights are only accepted when an inference mode
    /// with an available backend is selected, so an operator never believes a
    /// semantic ranking is running while it silently falls back to lexical.
    /// Named credentials must be unambiguous, secret-free and capability-bound.
    fn validate_clients(&self) -> Result<(), ConfigError> {
        let mut seen = std::collections::BTreeSet::new();
        for client in &self.server.clients {
            if client.id.trim().is_empty() || !is_provider_key(&client.id) {
                return Err(ConfigError::Policy(format!(
                    "invalid server client id: {}; use lowercase ascii, digits and underscore",
                    client.id
                )));
            }
            if !seen.insert(client.id.as_str()) {
                return Err(ConfigError::Policy(format!(
                    "duplicate server client id: {}",
                    client.id
                )));
            }
            match (&client.token_env, &client.token_sha256) {
                (Some(name), None) if !name.trim().is_empty() => {}
                (None, Some(digest)) if is_sha256_hex(digest) => {}
                (None, Some(_)) => {
                    return Err(ConfigError::Policy(format!(
                        "client {} token_sha256 must be a lowercase hex SHA-256 digest",
                        client.id
                    )))
                }
                _ => {
                    return Err(ConfigError::Policy(format!(
                        "client {} must declare exactly one of token_env or token_sha256",
                        client.id
                    )))
                }
            }
            if client.scopes.is_empty() {
                return Err(ConfigError::Policy(format!(
                    "client {} declares no scope; a credential without capability is never useful",
                    client.id
                )));
            }
            if let Some(expiry) = client.expires_at.as_deref() {
                if day_from_iso_date(expiry).is_none() {
                    return Err(ConfigError::Policy(format!(
                        "client {} has an invalid expires_at; use YYYY-MM-DD",
                        client.id
                    )));
                }
            }
            if let Some(tool) = client
                .tools
                .iter()
                .find(|tool| !MCP_TOOLS.contains(&tool.as_str()))
            {
                return Err(ConfigError::Policy(format!(
                    "client {} allows unknown MCP tool: {tool}",
                    client.id
                )));
            }
            if !client.tools.is_empty() && !client.scopes.contains(&Scope::Mcp) {
                return Err(ConfigError::Policy(format!(
                    "client {} allows MCP tools without the mcp scope",
                    client.id
                )));
            }
        }
        Ok(())
    }
}

/// Tool names the MCP surface exposes, and the vocabulary a client allowlist
/// may use. Kept here so an unknown name is rejected by configuration
/// validation instead of silently granting nothing.
pub const MCP_TOOLS: [&str; 6] = [
    "search",
    "deep_search",
    "fetch",
    "providers",
    "status",
    "answer",
];
fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
/// Render one [`ServerClient`] as the `toml_edit` table
/// [`Config::upsert_server_client`] writes into `[[server.clients]]`.
///
/// Only fields that are `Some`/non-empty are written, matching how an
/// operator would hand-author the same entry: an absent `expires_at` means
/// "never expires", not an explicit null.
fn server_client_to_table(client: &ServerClient) -> toml_edit::Table {
    let mut table = toml_edit::Table::new();
    table.insert("id", toml_edit::value(client.id.as_str()));
    if let Some(token_env) = client.token_env.as_deref() {
        table.insert("token_env", toml_edit::value(token_env));
    }
    if let Some(token_sha256) = client.token_sha256.as_deref() {
        table.insert("token_sha256", toml_edit::value(token_sha256));
    }
    if let Some(expires_at) = client.expires_at.as_deref() {
        table.insert("expires_at", toml_edit::value(expires_at));
    }
    let mut scopes = toml_edit::Array::new();
    for scope in &client.scopes {
        scopes.push(scope.as_str());
    }
    table.insert("scopes", toml_edit::value(scopes));
    let mut tools = toml_edit::Array::new();
    for tool in &client.tools {
        tools.push(tool.as_str());
    }
    table.insert("tools", toml_edit::value(tools));
    table
}
