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

from Call call
where
  call.getTargetName() = "validate_server" and
  not call.getFile().getRelativePath().matches("crates/amatl-core/src/config/%")
select call,
  "configuration accepted against the `bind` the document declares, not the address the process is listening on"
