use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static TEMP_ID: AtomicUsize = AtomicUsize::new(0);

#[test]
fn search_json_uses_public_search_contract() {
    let output = Command::new(env!("CARGO_BIN_EXE_amatl"))
        .args(["search", "rust async", "--json", "--mock"])
        .output()
        .expect("amatl binary should run");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("\"schema_version\": \"1\""));
    assert!(stdout.contains("\"status\": \"success\""));
    assert!(stdout.contains("\"canonical_url\": \"https://example.com/rust\""));
    assert!(!stdout.contains("final_url"));
    assert!(!stdout.contains("\"rrf\""));
    assert!(!stdout.contains("combined_score"));
    assert!(!stdout.contains("stable_order"));
    assert!(!stdout.contains("ranking_v2"));
    assert!(!stdout.contains("evidence_score"));
}

#[test]
fn failed_search_returns_exit_code_one() {
    let output = Command::new(env!("CARGO_BIN_EXE_amatl"))
        .args(["search", "ordinary query", "--json"])
        .output()
        .expect("amatl binary should run");
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("\"status\": \"failure\""));
}

#[test]
fn redirected_logs_are_structured_and_keep_stable_fields() {
    let output = Command::new(env!("CARGO_BIN_EXE_amatl"))
        .env("RUST_LOG", "amatl::routing=debug")
        .args(["search", "rust async", "--json", "--mock"])
        .output()
        .expect("amatl binary should run");
    assert!(output.status.success());
    let line = String::from_utf8(output.stderr)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_owned();
    let value: serde_json::Value = serde_json::from_str(&line).unwrap();
    for field in ["ts", "level", "target", "msg", "context"] {
        assert!(value.get(field).is_some(), "missing {field}: {value}");
    }
}

#[test]
fn required_skeleton_commands_are_available() {
    for command in ["providers", "config", "cache", "doctor"] {
        let status = Command::new(env!("CARGO_BIN_EXE_amatl"))
            .arg(command)
            .status()
            .expect("amatl command should run");
        assert!(status.success(), "{command} should succeed");
    }
}

#[test]
fn config_reports_the_effective_isolated_data_policy() {
    let id = TEMP_ID.fetch_add(1, Ordering::SeqCst);
    let base = std::env::temp_dir().join(format!("amatl-data-policy-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&base).unwrap();
    let config = base.join("amatl.toml");
    std::fs::write(
        &config,
        "[data_policy]\nprofile = \"isolated\"\negress = \"deny\"\ninference = \"local_only\"\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_amatl"))
        .arg("--config-file")
        .arg(&config)
        .arg("config")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("data_policy.profile = isolated"));
    assert!(stdout.contains("data_policy.egress = deny"));
    assert!(stdout.contains("data_policy.inference = local_only"));
    assert!(stdout.contains("data_policy.network_egress_allowed = false"));
    assert!(stdout.contains("data_policy.remote_inference_allowed = false"));
    std::fs::remove_file(config).unwrap();
    std::fs::remove_dir(base).unwrap();
}

#[test]
fn deep_command_is_exposed_without_running_network_on_help() {
    let output = Command::new(env!("CARGO_BIN_EXE_amatl"))
        .args(["deep", "--help"])
        .output()
        .expect("deep help should run");
    assert!(output.status.success());
    assert!(String::from_utf8(output.stdout)
        .unwrap()
        .contains("Usage: amatl deep"));
}

#[test]
fn ingest_dispatches_local_markdown_into_traceable_evidence() {
    let id = TEMP_ID.fetch_add(1, Ordering::SeqCst);
    let path =
        std::env::temp_dir().join(format!("amatl-cli-ingest-{}-{id}.md", std::process::id()));
    std::fs::write(
        &path,
        "# Local report\nAMATL evidence reached 88 percent on 2026-08-13.",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_amatl"))
        .arg("ingest")
        .arg(&path)
        .args(["--query", "AMATL evidence", "--json"])
        .output()
        .expect("local ingest should run");
    std::fs::remove_file(path).unwrap();

    assert!(output.status.success(), "{:?}", output);
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["schema_version"], "1");
    assert_eq!(response["document_type"], "markdown");
    assert_eq!(response["document"]["fetch_method"], "local");
    assert_eq!(response["document"]["status"], "enriched");
    assert_eq!(response["evidence_v2"]["evidence_version"], "v2");
    assert!(!response["evidence_v2"]["fragments"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn ingest_rejects_unknown_binary_without_echoing_content() {
    let id = TEMP_ID.fetch_add(1, Ordering::SeqCst);
    let path =
        std::env::temp_dir().join(format!("amatl-cli-ingest-{}-{id}.bin", std::process::id()));
    let secret = "private-binary-content";
    std::fs::write(&path, format!("{secret}\0")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_amatl"))
        .arg("ingest")
        .arg(&path)
        .arg("--json")
        .output()
        .expect("local ingest should fail safely");
    std::fs::remove_file(path).unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("document type is unsupported"));
    assert!(!stderr.contains(secret));
}

#[test]
fn provider_canary_refuses_incomplete_governance_without_network() {
    let id = TEMP_ID.fetch_add(1, Ordering::SeqCst);
    let base = std::env::temp_dir().join(format!("amatl-canary-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&base).unwrap();
    let config = base.join("amatl.toml");
    std::fs::write(&config, "[providers]\nenabled = [\"brave\"]\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_amatl"))
        .arg("--config-file")
        .arg(&config)
        .args(["provider-canary", "brave", "rust"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("governance approval is incomplete or expired"));
    std::fs::remove_file(config).unwrap();
    std::fs::remove_dir(base).unwrap();
}

#[test]
fn phase_nine_server_commands_are_exposed_without_binding_on_help() {
    for arguments in [
        ["serve", "--help"].as_slice(),
        ["mcp", "serve", "--help"].as_slice(),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_amatl"))
            .args(arguments)
            .output()
            .expect("server help should run");
        assert!(output.status.success());
    }
}

#[test]
fn ranking_v2_benchmark_is_reproducible_and_passes_gate() {
    let output = Command::new(env!("CARGO_BIN_EXE_amatl"))
        .args(["benchmark", "ranking-v2", "--json"])
        .output()
        .expect("ranking benchmark should run");
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["benchmark_id"], "ranking-v2-human-labeled-v2");
    assert_eq!(report["passed"], true);
    assert!(
        report["candidate_ndcg_at_3"].as_f64().unwrap()
            > report["baseline_ndcg_at_3"].as_f64().unwrap()
    );
}

#[test]
fn operational_benchmark_reports_latency_memory_and_sqlite_contention() {
    let output = Command::new(env!("CARGO_BIN_EXE_amatl"))
        .args([
            "benchmark",
            "operational",
            "--json",
            "--iterations",
            "4",
            "--concurrency",
            "2",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["workload"], "controlled-local-v1");
    assert_eq!(report["search"]["latency"]["samples"], 4);
    assert_eq!(report["deep_latency"]["samples"], 4);
    assert_eq!(report["sqlite"]["warm_hit_rate"], 1.0);
    if cfg!(target_os = "linux") {
        assert!(report["peak_rss_bytes"].as_u64().unwrap() > 0);
    }
}

#[test]
fn unavailable_sqlite_does_not_break_search() {
    let id = TEMP_ID.fetch_add(1, Ordering::SeqCst);
    let base = std::env::temp_dir().join(format!("amatl-cli-phase3-{}-{id}", std::process::id()));
    let database_is_a_directory = base.join("database-is-a-directory");
    std::fs::create_dir_all(&database_is_a_directory).unwrap();
    let config = base.join("amatl.toml");
    std::fs::write(
        &config,
        format!(
            "[persistence]\nenabled = true\npath = {:?}\n\n[telemetry]\npersistence_enabled = true\nretention_days = 30\n",
            database_is_a_directory.display().to_string()
        ),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_amatl"))
        .arg("--config-file")
        .arg(&config)
        .args(["search", "rust", "--json", "--mock"])
        .output()
        .expect("amatl binary should run without SQLite");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("\"status\": \"partial_success\""));
    assert!(stdout.contains("\"code\": \"storage_unavailable\""));

    std::fs::remove_file(config).unwrap();
    std::fs::remove_dir(database_is_a_directory).unwrap();
    std::fs::remove_dir(base).unwrap();
}

/// Configuration directory with persistence enabled, for the local domain and
/// maintenance commands.
fn persistent_config() -> (std::path::PathBuf, std::path::PathBuf) {
    let id = TEMP_ID.fetch_add(1, Ordering::SeqCst);
    let base = std::env::temp_dir().join(format!("amatl-cli-domain-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&base).unwrap();
    let database = base.join("amatl.sqlite3");
    let config = base.join("amatl.toml");
    std::fs::write(
        &config,
        format!(
            "schema_version = \"1\"\n\n[persistence]\nenabled = true\npath = \"{}\"\n",
            database.display()
        ),
    )
    .unwrap();
    (config, database)
}

fn amatl(config: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_amatl"))
        .arg("--config-file")
        .arg(config)
        .args(args)
        .output()
        .expect("amatl binary should run")
}

#[test]
fn history_and_saved_commands_manage_local_domain_state() {
    let (config, _database) = persistent_config();
    assert!(amatl(&config, &["search", "rust async", "--mock"])
        .status
        .success());

    let listed = amatl(&config, &["history", "list"]);
    assert!(listed.status.success());
    let stdout = String::from_utf8(listed.stdout).unwrap();
    assert!(stdout.contains("rust async"), "{stdout}");
    assert!(stdout.contains("cli"), "{stdout}");
    let id = stdout.split('\t').next().unwrap().trim().to_owned();

    assert!(amatl(&config, &["history", "delete", &id]).status.success());
    // Deleting the same entry twice is an error, not a silent success.
    assert!(!amatl(&config, &["history", "delete", &id]).status.success());

    let purged = amatl(&config, &["history", "purge"]);
    assert!(purged.status.success());
    assert!(String::from_utf8(purged.stdout).unwrap().contains("purged"));

    let saved = amatl(&config, &["saved", "list"]);
    assert!(saved.status.success());
    assert!(String::from_utf8(saved.stdout).unwrap().contains("none"));
}

#[test]
fn domain_commands_require_persistence() {
    let output = Command::new(env!("CARGO_BIN_EXE_amatl"))
        .args(["history", "list"])
        .output()
        .expect("amatl binary should run");
    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("requires [persistence]"));
}

#[test]
fn db_maintenance_reports_health_and_rolls_the_schema_back() {
    let (config, database) = persistent_config();
    assert!(amatl(&config, &["search", "rust async", "--mock"])
        .status
        .success());

    let health = amatl(&config, &["db", "health", "--json"]);
    assert!(health.status.success());
    let value: serde_json::Value = serde_json::from_slice(&health.stdout).unwrap();
    assert_eq!(value["migration_version"], value["code_migration_version"]);
    assert_eq!(value["journal_mode"], "wal");

    // Downgrade takes a backup and really moves the schema version back.
    let downgraded = amatl(&config, &["db", "downgrade", "--to", "4"]);
    assert!(
        downgraded.status.success(),
        "{}",
        String::from_utf8_lossy(&downgraded.stderr)
    );
    let backups = amatl(&config, &["db", "backups"]);
    assert!(String::from_utf8(backups.stdout)
        .unwrap()
        .contains(".sqlite3"));
    assert!(database.exists());

    // Reopening migrates forward again, so the tool is not one-way.
    let health = amatl(&config, &["db", "health", "--json"]);
    let value: serde_json::Value = serde_json::from_slice(&health.stdout).unwrap();
    assert_eq!(value["migration_version"], value["code_migration_version"]);

    let circuits = amatl(&config, &["db", "circuits", "--json"]);
    assert!(circuits.status.success());
    let snapshots: serde_json::Value = serde_json::from_slice(&circuits.stdout).unwrap();
    assert!(snapshots.is_array());
    assert!(amatl(&config, &["db", "circuits", "--reset"])
        .status
        .success());
}

#[test]
fn serve_reports_the_effective_listener_before_binding() {
    let (config, _database) = persistent_config();
    // An invalid override is rejected before the listener is created.
    let output = amatl(
        &config,
        &["serve", "--bind", "not-an-address", "--port", "0", "--json"],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .to_lowercase()
        .contains("configuration"));
}

#[test]
fn cli_reports_the_same_error_catalog_codes_as_the_other_surfaces() {
    // A domain failure carries a catalog code on stderr, next to the message.
    let canary = Command::new(env!("CARGO_BIN_EXE_amatl"))
        .args(["provider-canary", "brave", "rust"])
        .output()
        .expect("amatl binary should run");
    assert!(!canary.status.success());
    let stderr = String::from_utf8(canary.stderr).unwrap();
    assert!(
        stderr.contains("error_code=provider_not_enabled"),
        "{stderr}"
    );

    // A failed Search is a contract outcome: its own composite codes are
    // reported instead of an invented one, and stdout stays clean.
    let search = Command::new(env!("CARGO_BIN_EXE_amatl"))
        .args(["search", "ordinary query", "--json"])
        .output()
        .expect("amatl binary should run");
    assert_eq!(search.status.code(), Some(1));
    let stderr = String::from_utf8(search.stderr).unwrap();
    assert!(
        stderr.contains("error_code=no_available_provider"),
        "{stderr}"
    );
    let stdout = String::from_utf8(search.stdout).unwrap();
    assert!(!stdout.contains("error_code="), "{stdout}");
}
