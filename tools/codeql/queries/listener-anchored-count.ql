/**
 * @name AMATL: listener-anchored validation count is unchanged
 * @description A-01 regression guard, anti-vacuity companion to
 *              `listener-anchored-acceptance.ql`. That query reports an
 *              acceptance point that stopped anchoring; this one fails when the
 *              number of acceptance points, or of listener-anchored validations
 *              in `amatl-server`, is no longer the expected two. The first count
 *              is what keeps the pair non-vacuous: the sibling query identifies
 *              `AppState::reload` partly by the loader it calls, so renaming the
 *              loader leaves it with nothing to find and it passes. Counting the
 *              same identification here turns that silence into a failure. The
 *              second count catches an anchor that was removed, and a *new*
 *              acceptance point that nobody decided about. A deliberate change
 *              therefore has to update these numbers as part of the change.
 * @kind problem
 * @problem.severity error
 * @id amatl/listener-anchored-count
 * @tags security
 */

import rust
import acceptance_points

from File file, int points, int found
where
  file.getRelativePath() = "crates/amatl-server/src/lib.rs" and
  points = count(Function function | is_acceptance_point(function)) and
  found =
    count(Call call |
      call.getTargetName() = "validate_server_for_listener" and
      call.getFile() = file
    ) and
  (points != 2 or found != 2)
select file,
  "expected exactly two acceptance points and two listener-anchored validations in amatl-server (the reload and the `[server]` patch); found " +
    points.toString() + " acceptance point(s) and " + found.toString() +
    " anchored validation(s)"
