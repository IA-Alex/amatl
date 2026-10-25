/**
 * Shared identification of AMATL's configuration acceptance points.
 *
 * A library rather than a query so that both listener-anchored checks use one
 * copy: `listener-anchored-acceptance.ql` reports an acceptance point that
 * stopped anchoring, and `listener-anchored-count.ql` notices when this
 * identification itself stopped matching anything. With two copies, the second
 * check would keep counting by the old spelling after the first was edited and
 * the pair would go quiet exactly when it matters.
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
 *
 * The loader name is part of the identification, so renaming the loader empties
 * this predicate. `listener-anchored-count.ql` asserts its size for exactly
 * that reason: an empty identification has to fail the gate, not pass it.
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
