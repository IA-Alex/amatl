/**
 * @name AMATL: server configuration validated against the declared bind
 * @description A-01 regression guard. `Config::validate_server` decides the
 *              loopback/TLS/`no_auth` matrix from the `bind` written in the
 *              document, but `--bind`/`--port` override the document at startup
 *              and are never written back to it. Only `amatl-core`'s own
 *              document validation may use that bind-blind check; every caller
 *              that accepts a configuration into a *running* process must use
 *              `Config::validate_server_for_listener`, or the bypass A-01
 *              describes re-opens.
 * @kind problem
 * @problem.severity error
 * @id amatl/declared-bind-validation
 * @tags security
 */

import rust

/**
 * The single file allowed to call the bind-blind check: `config/mod.rs` holds
 * both callers that legitimately need it — `Config::validate` (the document
 * validation, which can only see the declared `bind`) and
 * `Config::validate_server_for_listener` (which passes the effective address
 * through to the same check).
 *
 * This is the exact file, not the whole `config/` directory. A directory-wide
 * exemption admits every *new* module added beside it, so a call in
 * `config/<anything else>.rs` was never reported; the exemption has to be as
 * narrow as the code that justifies it.
 */
predicate may_use_declared_bind(File file) {
  file.getRelativePath() = "crates/amatl-core/src/config/mod.rs"
}

from Call call
where
  call.getTargetName() = "validate_server" and
  not may_use_declared_bind(call.getFile())
select call,
  "configuration accepted against the `bind` the document declares, not the address the process is listening on"
