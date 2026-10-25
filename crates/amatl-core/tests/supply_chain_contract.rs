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
//! release can reach — `zip 7.2.0` under the optional
//! `experimental-local-embeddings` stack is the current case — is recorded as
//! open in `docs/security/supply-chain.md`: asserting its floor would redden
//! the suite on every host without remediating anything.

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
];

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
