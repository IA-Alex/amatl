/**
 * @name AMATL: configuration acceptance points are listener-anchored
 * @description A-01 regression guard, companion to
 *              `declared-bind-validation.ql`. The two places that accept a
 *              configuration into a *running* process — `AppState::reload` and
 *              `update_server_pending_config` — must each validate it against
 *              the address the process actually listens on, or a document that
 *              declares loopback can weaken the listener that is serving.
 * @kind problem
 * @problem.severity error
 * @id amatl/listener-anchored-acceptance
 * @tags security
 */

import rust

/**
 * The functions that accept a configuration into a *running* process:
 * `AppState::reload` (which re-reads the document and swaps the service) and
 * `update_server_pending_config` (which writes a `[server]` patch that the next
 * start applies). The reload branch also requires the `Config::load_optional`
 * call, because two other functions in the same file are called `reload` and
 * neither accepts a configuration: `ReloadHandle::reload` forwards to
 * `AppState::reload`, and the HTTP handler forwards to the handle.
 */
predicate is_acceptance_point(Function function) {
  exists(function.getName()) and
  (
    function.getName().getText() = "update_server_pending_config"
    or
    function.getName().getText() = "reload" and
    exists(Call load |
      load.getEnclosingCallable() = function and
      load.getTargetName() = "load_optional"
    )
  )
}

/** Whether `function` validates a configuration against the effective listener. */
predicate anchors(Function function) {
  exists(Call anchored |
    anchored.getEnclosingCallable() = function and
    anchored.getTargetName() = "validate_server_for_listener"
  )
}

from Function function
where
  is_acceptance_point(function) and
  not anchors(function)
select function,
  "this acceptance point no longer validates the configuration against the effective listener"
