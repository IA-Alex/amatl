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
import acceptance_points

from Function function
where
  is_acceptance_point(function) and
  not anchors(function)
select function,
  "this acceptance point no longer validates the configuration against the effective listener"
