//! Advisory floors for the locked dependency graph.
//!
//! `cargo audit` and `cargo deny check advisories` are the authoritative
//! advisory gates, but they need the RustSec database and only run in the Linux
//! `contract-gate` job. These assertions read the workspace `Cargo.lock` — the
//! same file CI installs with `--locked` — so a locked release below an
//! advisory's fixed version fails `cargo test --workspace` on every host,
//! offline, at the exact boundary the advisory states.
//!
//! This is a floor, not a pin: any newer compatible release passes. Each entry
//! records the advisories it closes, so a reviewer re-derives the number from
//! the advisory text instead of trusting the constant.
//!
//! Only remediated advisories belong here. An advisory whose fix no compatible
//! release can reach is recorded in `docs/security/supply-chain.md` instead:
//! asserting its floor would redden the suite on every host without remediating
//! anything. `zip 7.2.0` (RUSTSEC-2026-0367/-0368/-0369) was that case until the
//! Candle stack that pulled it in was quarantined out of the workspace into
//! `experiments/local-embeddings-boundary`; the lockfile read below no longer
//! contains it, so there is no floor to assert.

use std::cmp::Ordering;
use std::path::PathBuf;

/// `(crate, minimum fixed version, advisories that version closes)`.
const ADVISORY_FLOORS: &[(&str, &str, &str)] = &[
    (
        "quinn-proto",
        "0.11.18",
        "RUSTSEC-2026-0339 (unbounded growth of pending connection ID \
         retirements), RUSTSEC-2026-0341 (the stream reassembly \
         memory-exhaustion guard can be bypassed) and RUSTSEC-2026-0342 \
         (zero-length DATAGRAM frames bypass the receive buffer limit), fixed in \
         0.11.17; RUSTSEC-2026-0338 (a remote peer panics the crate with \
         zero-length STREAM frames during unordered reads) and RUSTSEC-2026-0340 \
         (a peer minimum ACK delay panics when ACK_FREQUENCY transmission is \
         enabled), fixed in 0.11.18. Optional HTTP/3 dependency of `reqwest` \
         that this workspace never enables, so it is locked but never compiled.",
    ),
    (
        "rustls",
        "0.23.46",
        "RUSTSEC-2026-0285 (TLS 1.3 handshake messages accepted across encryption \
     level boundaries; fixed in 0.23.45), RUSTSEC-2026-0346 (`Debug` on client \
     session values exposes TLS secrets) and RUSTSEC-2026-0349 (key-exchange \
     downgrade recorded before the server is authenticated), both fixed in \
     0.23.46. RUSTSEC-2026-0348 (Brotli certificate decompression can allocate \
     up to 1 GiB before authentication; fixed in 0.23.46) requires the \
     non-default `brotli` feature, which this graph does not enable: the locked \
     feature set is ring,std,tls12.",
    ),
    (
        "rustls-webpki",
        "0.103.16",
        "RUSTSEC-2026-0382 (certificate verification can accept a path through \
     distrusted emSign intermediates whose private keys are now well known; \
     fixed in 0.103.16), published 2026-10-20. In the primary TLS path: \
     `rustls 0.23.46` -> `axum-server` for the listener and `hyper-rustls` -> \
     `reqwest` for outbound HTTPS, so it is compiled into both the server and \
     the client builds.",
    ),
    (
        "tokio",
        "1.53.3",
        "RUSTSEC-2026-0386 (the safe `AsyncFd::new`, `with_interest`, `try_new` \
     and `try_with_interest` constructors accept any `T: AsRawFd`, so a \
     duplicated-then-closed descriptor can be deregistered and freed while \
     `epoll` still holds it - a use-after-free when readiness fires; fixed in \
     1.53.3 on this line, and in 1.51.6 on the 1.51 line), published \
     2026-10-13. The async runtime of every crate in the workspace. The \
     workspace never calls `AsyncFd`, so the deprecation the fix introduces \
     reaches no call site here.",
    ),
    (
        "webpki-roots",
        "1.0.10",
        "RUSTSEC-2026-0381 (the bundled trust store still contains `emSign ECC \
     Root CA - G3`, so certificate paths through distrusted emSign \
     intermediates are trusted; fixed in 1.0.10, which removes the root), \
     published 2026-10-19. Trust store for `hyper-rustls` -> `reqwest`. The \
     advisory marks the 0.26.11 compatibility package unaffected because it \
     holds no trust store of its own and re-exports the roots from its \
     separately resolved 1.x parent, so the floor has to guard that parent.",
    ),
    (
        "rmcp",
        "3.9.0",
        "RUSTSEC-2026-0457 (the OAuth client SSRF filter checks host strings \
     instead of the resolved addresses, fixed in 3.7.0), RUSTSEC-2026-0461 \
     (the Streamable HTTP server transport caches tool schemas without a \
     bound, fixed in 3.6.0) and RUSTSEC-2026-0462 (the Streamable HTTP \
     clients buffer an unbounded response body before parsing, fixed in \
     3.9.0), all published 2026-10-20 and 2026-10-22. The MCP transport of \
     `amatl-server`, so it is compiled into the shipped server build. The \
     floor is the highest fix, 3.9.0; any newer compatible release passes.",
    ),
    (
        "rustix",
        "1.1.5",
        "RUSTSEC-2026-0486 (the xattr functions mark unwritten bytes as \
     initialized when given a zero-length buffer, so a caller can read \
     uninitialized memory; `cargo deny check` classifies it `unsound`, an \
     error, while `cargo audit` reports it as a warning), published \
     2026-10-27. Reached through `tokio`'s Unix signal and fs paths; the \
     advisory's fixed release is 1.1.5 and the lockfile carries 1.1.6.",
    ),
    (
        "quinn-udp",
        "0.5.16",
        "RUSTSEC-2026-0538 (the receive loop can spin after a truncated \
     datagram), published 2026-10-27. An optional dependency reached through \
     `quinn`/`reqwest`'s HTTP/3 stack, which this workspace locks but never \
     enables; the advisory's fixed releases are `>=0.5.16, <0.6.0` or \
     `>=0.6.3`, and the lockfile carries 0.5.16, so the 0.5.x floor is the \
     one that applies to the locked line.",
    ),
];

// Advisories whose fix no compatible release in the locked graph can reach are
// NOT floors (asserting one would redden every host without remediating
// anything). Two are that case today:
//
// - `libsqlite3-sys` RUSTSEC-2026-0459: `sqlx` 0.8.6 caps it at `^0.30` and
//   even `sqlx 0.9.0` caps it at `>=0.30.1, <0.38.0`, while the fix is
//   `>=0.39.0`, so no released `sqlx` reaches it. See `ADR-017`.
// - `chrono` RUSTSEC-2026-0543 (unmaintained, every version): reached only
//   through `rmcp` and `schemars`, which expose no `chrono` use to AMATL code.
//   The advisory names no fixed release, so there is nothing to upgrade to.
//   See `ADR-018`.
//
// Both are recorded in `docs/security/supply-chain.md` and the authoritative
// `cargo audit`/`cargo deny check` gates stay red until each is remediated,
// removed from the graph, or waived by a written ADR. Because they cannot be
// floors, the nearest thing a hermetic guard can do is make sure a
// plain `cargo update` cannot resolve to a `sqlx` line whose SQLite feature set
// differs from the audited one without a reviewer seeing the manifest change:
// `sqlx_is_pinned_to_the_audited_line` below.

/// The workspace manifest, or `None` outside a workspace checkout (same
/// reasoning as [`workspace_lockfile`]).
fn workspace_manifest() -> Option<String> {
    std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml")).ok()
}

/// The string value of `key` inside the `[workspace.dependencies]` table.
fn workspace_dependency(manifest: &str, key: &str) -> Option<String> {
    let parsed: toml::Value =
        toml::from_str(manifest).expect("the workspace manifest is valid TOML");
    let value = parsed
        .get("workspace")
        .and_then(|workspace| workspace.get("dependencies"))
        .and_then(|dependencies| dependencies.get(key))?
        .clone();
    Some(match value {
        toml::Value::String(version) => version,
        toml::Value::Table(table) => table
            .get("version")
            .and_then(|version| version.as_str())
            .unwrap_or_default()
            .to_owned(),
        _ => String::new(),
    })
}

/// Splits a requirement into its leading operator and the rest, so a pin can be
/// told apart from the caret form the workspace used before 2026-11-15.
fn requirement_parts(requirement: &str) -> (&str, &str) {
    let requirement = requirement.trim();
    for operator in ["=", "^", "~", ">", "<"] {
        if let Some(version) = requirement.strip_prefix(operator) {
            return (operator, version.trim());
        }
    }
    ("", requirement)
}

/// The workspace lockfile, or `None` outside a workspace checkout (a published
/// crate has no lockfile to guard). Inside the workspace the file always
/// exists, and CI installs it with `--locked`, so deleting it cannot be used to
/// skip this test where it matters.
fn workspace_lockfile() -> Option<String> {
    std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock")).ok()
}

/// Every locked version of `crate_name`, in lockfile order. More than one is
/// possible: `deny.toml` warns on multiple versions instead of denying them.
fn locked_versions(lockfile: &str, crate_name: &str) -> Vec<String> {
    let parsed: toml::Value = toml::from_str(lockfile).expect("Cargo.lock is valid TOML");
    parsed
        .get("package")
        .and_then(|packages| packages.as_array())
        .expect("Cargo.lock declares [[package]] entries")
        .iter()
        .filter(|package| package.get("name").and_then(|name| name.as_str()) == Some(crate_name))
        .map(|package| {
            package
                .get("version")
                .and_then(|version| version.as_str())
                .expect("a locked package declares its version")
                .to_owned()
        })
        .collect()
}

/// Splits a version into its numeric core and whether it is a prerelease.
/// Build metadata is not a prerelease and does not affect precedence.
fn numeric_core(version: &str) -> (&str, bool) {
    let without_build = version.split('+').next().unwrap_or(version);
    match without_build.split_once('-') {
        Some((core, _)) => (core, true),
        None => (without_build, false),
    }
}

fn numeric_triple(core: &str) -> (u64, u64, u64) {
    let mut parts = core.split('.').map(|part| part.parse::<u64>().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

/// `true` when `found` reaches `floor`, comparing `major.minor.patch`
/// numerically. On equal numbers only a stable release clears the floor: a
/// prerelease of the fixed version is not the fix.
fn version_at_least(found: &str, floor: &str) -> bool {
    let (found_core, found_is_prerelease) = numeric_core(found);
    let (floor_core, floor_is_prerelease) = numeric_core(floor);
    match numeric_triple(found_core).cmp(&numeric_triple(floor_core)) {
        Ordering::Greater => true,
        Ordering::Less => false,
        Ordering::Equal => !found_is_prerelease || floor_is_prerelease,
    }
}

#[test]
fn locked_versions_reach_every_advisory_floor() {
    let Some(lockfile) = workspace_lockfile() else {
        return;
    };
    for (crate_name, floor, advisories) in ADVISORY_FLOORS.iter().copied() {
        let versions = locked_versions(&lockfile, crate_name);
        assert!(
            !versions.is_empty(),
            "{crate_name} is no longer locked: delete its advisory floor here \
             (re-checking the advisories first) instead of leaving an entry that \
             asserts nothing"
        );
        for version in &versions {
            assert!(
                version_at_least(version, floor),
                "Cargo.lock locks {crate_name} {version}, below the fixed release \
                 {floor}: {advisories}"
            );
        }
    }
}

#[test]
fn floor_comparison_handles_patch_boundaries_and_prereleases() {
    assert!(version_at_least("0.23.46", "0.23.46"));
    assert!(version_at_least("0.23.47", "0.23.46"));
    assert!(version_at_least("0.24.0", "0.23.46"));
    assert!(version_at_least("1.0.0", "0.23.46"));
    assert!(version_at_least("0.23.46+build.1", "0.23.46"));
    assert!(!version_at_least("0.23.45", "0.23.46"));
    assert!(!version_at_least("0.22.99", "0.23.46"));
    assert!(!version_at_least("0.23.46-alpha.1", "0.23.46"));
}

/// `sqlx` is pinned with `=` to the line the advisory analysis covers.
///
/// RUSTSEC-2026-0459 cannot be a floor (no compatible release fixes it), so a
/// silent `cargo update` is the only way the SQLite half of the graph could
/// change without a reviewer reading a manifest diff: the bare `0.8` requirement
/// admits every newer `0.8.x`, and each one decides which `libsqlite3-sys`
/// range — and which `sqlite` feature set — the workspace gets. Pinning keeps
/// that decision where `ADR-017` records it; moving to a line whose fix is
/// reachable is the remediation and must update this test, the ADR and the
/// advisory documentation together.
#[test]
fn sqlx_is_pinned_to_the_audited_line() {
    let Some(manifest) = workspace_manifest() else {
        return;
    };
    let requirement = workspace_dependency(&manifest, "sqlx")
        .expect("[workspace.dependencies] must keep declaring sqlx");
    let (operator, version) = requirement_parts(&requirement);
    assert_eq!(
        operator, "=",
        "sqlx must be pinned with `=` (found {requirement:?}): a caret or \
         wildcard requirement lets `cargo update` change the libsqlite3-sys \
         range behind RUSTSEC-2026-0459 and its sqlite feature set without a \
         reviewed manifest change"
    );
    assert_eq!(
        version, "0.8.6",
        "the sqlx pin moved to {version}: re-check RUSTSEC-2026-0459 against \
         that line, and if it now admits libsqlite3-sys >=0.39.0, remediate and \
         update ADR-017, docs/security/supply-chain.md and this pin together"
    );
    let Some(lockfile) = workspace_lockfile() else {
        return;
    };
    assert_eq!(
        locked_versions(&lockfile, "sqlx"),
        ["0.8.6".to_owned()],
        "Cargo.lock must lock exactly the pinned sqlx line"
    );
    assert_eq!(
        locked_versions(&lockfile, "sqlx-sqlite"),
        ["0.8.6".to_owned()],
        "the SQLite driver must move with `sqlx`: it is the crate that declares \
         the libsqlite3-sys range"
    );
}

/// The `libsqlite3-sys` version the lockfile carries is the one the advisory
/// analysis names, and the gate that would catch a change is reachable.
///
/// `sqlx` is pinned, so the range `sqlx-sqlite` declares cannot move on its own;
/// what a reviewer still has to confirm is that the *resolved* crate is the
/// audited 0.30.1 rather than a newer release that happens to sit inside the
/// same `^0.30`, and that `deny.toml` names exactly the advisories the analysis
/// covers — the `libsqlite3-sys` one above and the `chrono` unmaintained one
/// (`RUSTSEC-2026-0543`, `ADR-018`). Neither check needs the RustSec database,
/// so both run offline on every host.
#[test]
fn the_audited_sqlite_binding_and_its_suppression_are_still_the_ones_reviewed() {
    let Some(lockfile) = workspace_lockfile() else {
        return;
    };
    assert_eq!(
        locked_versions(&lockfile, "libsqlite3-sys"),
        ["0.30.1".to_owned()],
        "libsqlite3-sys moved: re-check RUSTSEC-2026-0459 against the new release \
         (the fix is >=0.39.0), then update ADR-017, \
         docs/security/supply-chain.md and this assertion together"
    );
    let deny =
        std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../deny.toml"))
            .expect("the workspace checkout carries deny.toml");
    let advisories = toml::from_str::<toml::Value>(&deny)
        .expect("deny.toml is valid TOML")
        .get("advisories")
        .cloned()
        .expect("deny.toml declares an [advisories] table");
    let ignored = advisories
        .get("ignore")
        .and_then(|value| value.as_array())
        .expect("the [advisories] table declares its ignore list, even when empty");
    assert_eq!(
        ignored
            .iter()
            .filter_map(|value| value.as_str())
            .collect::<Vec<_>>(),
        ["RUSTSEC-2026-0459", "RUSTSEC-2026-0543"],
        "the advisory suppression set changed: every entry needs an ADR and a \
         threat model in docs/security/supply-chain.md, and CI passes the same \
         IDs to scripts/security-gate.sh"
    );
}

/// The pin above is only meaningful if the manifest parser reads the shape the
/// workspace actually uses, including the quoted table form.
#[test]
fn workspace_dependency_reads_both_manifest_shapes() {
    let manifest = "[workspace.dependencies]\n\
                    sqlx = { version = \"0.8.6\", default-features = false }\n\
                    toml = \"0.8\"\n";
    assert_eq!(
        workspace_dependency(manifest, "sqlx").as_deref(),
        Some("0.8.6")
    );
    assert_eq!(
        workspace_dependency(manifest, "toml").as_deref(),
        Some("0.8")
    );
    assert_eq!(workspace_dependency(manifest, "absent"), None);
    assert_eq!(requirement_parts("0.8.6"), ("", "0.8.6"));
    assert_eq!(requirement_parts("=0.8.6"), ("=", "0.8.6"));
    assert_eq!(
        requirement_parts(">=0.39.0, <0.40"),
        (">", "=0.39.0, <0.40")
    );
}
