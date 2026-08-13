# Release and Linux packaging

The canonical repository is <https://github.com/IA-Alex/amatl>. A release
candidate is published only from an annotated tag that exactly matches the
workspace version, for example `v0.1.0-rc.1`.

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

- `amatl_<version>_amd64.deb` for Debian/Ubuntu;
- `amatl-<version>.x86_64.rpm` for Fedora-compatible systems;
- `amatl-<version>-1-x86_64.pkg.tar.zst` for Arch Linux.

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
