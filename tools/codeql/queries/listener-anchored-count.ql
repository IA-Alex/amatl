/**
 * @name AMATL: listener-anchored validation count is unchanged
 * @description A-01 regression guard, anti-vacuity companion to
 *              `listener-anchored-acceptance.ql`. That query reports an
 *              acceptance point that stopped anchoring; this one fails when the
 *              number of listener-anchored validations in `amatl-server` is no
 *              longer the expected two, which is what happens if an anchor is
 *              removed, if the callee is renamed (both leaving the sibling query
 *              with nothing to find), or if a *new* acceptance point is added
 *              without deciding whether it must anchor too. A deliberate change
 *              therefore has to update this number as part of the change.
 * @kind problem
 * @problem.severity error
 * @id amatl/listener-anchored-count
 * @tags security
 */

import rust

from File file, int found
where
  file.getRelativePath() = "crates/amatl-server/src/lib.rs" and
  found =
    count(Call call |
      call.getTargetName() = "validate_server_for_listener" and
      call.getFile() = file
    ) and
  found != 2
select file,
  "expected exactly two listener-anchored validations in amatl-server (the reload and the `[server]` patch); found " +
    found.toString()
