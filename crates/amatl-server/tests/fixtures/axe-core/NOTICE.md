# axe-core (vendored test fixture)

- **Version:** 4.13.0
- **Source:** `https://unpkg.com/axe-core@4.13.0/axe.min.js`
- **SHA-256:** `c24f097bd2f451d4f933e8bc7d8d539f8672a2ebcb5cc9f9f3eec8ca9470a0c1`
- **License:** Mozilla Public License 2.0 (see the header comment in
  `axe.min.js`). © Deque Systems, Inc.
- **Why vendored, not fetched at test time:** the test suite must not depend on
  network access, and `browser_e2e.rs` is already inert without a WebDriver
  endpoint.
- **Why not npm/Node:** AMATL is Linux-first Rust with no Node toolchain
  dependency (`plan_amatl.md` §3); pulling in `npm` only to run axe would add
  exactly the dependency the project avoids. `execute_async` over the existing
  WebDriver connection (`fantoccini`) injects the library instead.
- **Scope:** test-only. This file is never served by `amatl-ui` or shipped in
  the product; it is read by `crates/amatl-server/tests/browser_e2e.rs` via
  `include_str!` and injected into a headless browser page for the duration of
  one accessibility assertion.
- **Upgrading:** replace `axe.min.js`, update the version/hash above, and rerun
  `AMATL_BROWSER_E2E=1 cargo test -p amatl-server --test browser_e2e -- --test-threads=1`
  against a live WebDriver endpoint.
