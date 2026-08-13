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
