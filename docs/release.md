# Release and packaging

The canonical repository is <https://github.com/IA-Alex/amatl>. A release
candidate is published only from an annotated tag that exactly matches the
workspace version, for example `v0.1.0-rc.1`.

## Supported platforms

| Platform | Target | Package formats |
|---|---|---|
| Linux x86_64 | `x86_64-unknown-linux-musl` | `.tar.gz`, `.deb`, `.rpm`, `.pkg.tar.zst` |
| Linux aarch64 | `aarch64-unknown-linux-musl` | `.tar.gz` |
| macOS x86_64 | `x86_64-apple-darwin` | `.tar.gz` |
| macOS aarch64 (Apple Silicon) | `aarch64-apple-darwin` | `.tar.gz` |
| Windows x86_64 | `x86_64-pc-windows-msvc` | `.zip` |

### Support scope

The table above is the whole distribution scope; nothing else is published.

- **Tier 1 — Linux x86_64 (`x86_64-unknown-linux-musl`).** Static binary plus
  native packages, reproducible archive, SBOMs, checksums and build
  attestations. This is the target the release workflow validates end to end.
- **Tier 2 — Linux aarch64, macOS x86_64/aarch64, Windows x86_64.** Signed-off
  `.tar.gz`/`.zip` archives built from the same tag with checksums, without
  native packages and without renderer isolation.
- **Out of scope.** Any other target, and every form of self-managed
  distribution channel (Homebrew, winget, AUR, distro repositories). Building
  from source on an unlisted platform is supported as source code, not as a
  published artifact.

Every listed target is compiled and tested on each push: `contract-gate` runs
the workspace test suite on Linux, macOS and Windows, so a platform regression
fails CI instead of appearing at tag time.

The Chromium sandbox (`packaging/amatl-chromium-sandbox`) is Linux-only: it
requires bubblewrap user namespaces and is only packaged in `.deb` and `.rpm`.
On macOS and Windows the binary runs without renderer isolation; the
`AMATL_CHROMIUM_BIN` capability is unavailable on those platforms. Native
packaging (`packaging/build-linux-packages.sh`) is likewise Linux-only by
design and is never invoked from the macOS or Windows jobs.

## Owner release procedure

1. Confirm that `main` is clean and every `contract-gate` job is green.
2. Run the `release-candidate` workflow manually and inspect its private
   artifact without publishing a tag.
3. Verify the musl binary, four CycloneDX SBOMs, `SHA256SUMS`, `.deb`, `.rpm`
   and `.pkg.tar.zst` contents on a clean Linux host.
4. Create and push an annotated tag:

   ```bash
   git tag -a v0.1.0-rc.1 -m "AMATL v0.1.0-rc.1"
   git push origin v0.1.0-rc.1
   ```

5. The workflow validates tag/version agreement and publishes a GitHub
   prerelease. Never create a release manually from an unverified local binary.

The generated binary is static musl. Native packages intentionally install the
same verified binary, README, licenses and manual page; Trafilatura and Chromium
remain optional runtime capabilities and are not package dependencies.

## Native package validation

The workflow builds:

- `amatl_<version>_amd64.deb` and `amatl_<version>_arm64.deb` for Debian/Ubuntu;
- `amatl-<version>.x86_64.rpm` and `amatl-<version>.aarch64.rpm` for Fedora-compatible systems;
- `amatl-<version>-1-x86_64.pkg.tar.zst` and `amatl-<version>-1-aarch64.pkg.tar.zst` for Arch Linux.

`packaging/PKGBUILD` is the auditable source-build recipe for a future AUR
submission. Before publishing it to AUR, replace `SKIP` with the SHA-256 of the
actual tagged source archive and submit through the owner's authenticated AUR
account. The repository does not claim an AUR package until that external
submission exists.

## GitHub enforcement limitation

`@IA-Alex` is the verified owner and CODEOWNER. On 2026-08-13 GitHub returned
HTTP 403 when branch protection and repository rulesets were requested for this
private user-owned repository: the feature requires a plan upgrade or public
visibility. Until the owner makes one of those changes, green CI and CODEOWNER
review are policy evidence, not hosting-enforced controls. Once available,
require pull requests, one CODEOWNER approval, `contract-gate` and
`trafilatura-integration`, dismiss stale approvals, block force pushes and block
deletions on `main`.
