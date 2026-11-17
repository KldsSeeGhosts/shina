# Execution lease integration on published main

This root-owned checkout preserves original candidate `be0e3f2` and the bounded
review corrections at `b630449`. It merges published main `6eff805`. The original
worker and review checkouts remain unchanged.

## Durable input composition

Lease assignment and target input intent now share the accepted peer-command
transaction. Fresh launch no longer connects a provider before that intent is
staged. Stops retain the exact execution and attempt checks, then stage the
durable target stop. Native stop effects run through the committed input driver.
A transport stop receipt does not settle the lease or prove provider exit.
Observed provider terminal evidence or a separately verified executor fence is
still required before replacement.

Published migrations 008 and 009 remain unchanged. Execution state stays in the
additive index-version-5 object. The tentative lease protocol number is 48,
after the separately reserved scheduler 46 and repository catalog 47. The
mobile hello golden and exact-capability gate use the same number.

## Reproduced reply defects

Three new regressions failed before the reply correction:

- A returned lease could change immutable ownership fields while retaining its
  lease id.
- A stop reply could select another controller lease through `allow_rebase`.
- A reply rejected by the lease helper could still replace launch or wait
  progress because the caller continued processing it.

The helper now compares the complete controller binding, schema, request key,
queue flag, prepared time and known executor-local aliases. The first physical
alias may be learned only for the saved controller assignment. Unsolicited
leases are refused. Neither transport receipts nor missing progress fields can
manufacture provider acceptance or release uncertainty.

All launch, send, stop, wait and completion replies pass the same validation
before changing task progress or consuming owed controls. Completion collection
retains the original wait's execution and input evidence with its result-id
acknowledgement and trusted transport source. Remote assignment also carries
the saved request key.

Exact terminal evidence can reconcile an uncertain lease. A fence report needs
nonempty quiescence evidence. A phase number alone does not supply that proof.

## Verification and remaining gates

The combined tree passes 66 remote-task tests, ten lease-book tests and 32
runtime tests. The five new reply tests are included in that count. They cover
changed immutable fields, cross-lease stop replies, refusal without progress
mutation across all five reply routes, one-time physical alias learning and
terminal/fence reconciliation.

Two authenticated paired-server tests pass, as do the exact lease-capability
gate and live mobile hello golden. The all-target build, formatting and diff
checks passed at the first focused checkpoint. The subsequent full-run result
and corrections are recorded below.

## Combined-suite findings and corrections

The first full serial run at `5843526` passed all 2,080 unit tests with 13
ignored, but only 109 of 122 integration tests passed. The 13 failures block
publication. They were not attributed to environmental flakiness.

The failed live snapshots and follow-up regressions exposed these composition
defects:

- Database-backed peer waits loaded thread rows without the root execution
  book, so strict progress validation received no lease. The scoped read now
  includes only the current and next leases named by that thread. A regression
  checks both inclusion and exclusion of unrelated owner records.
- The first peer injection generated a new run UUID instead of consuming its
  controller-issued binding. That prompted an executor-local replacement lease.
  The first injection now consumes the assigned run. A failing-then-passing
  regression checks the actual adapter command's run id.
- Follow-up admission read the runtime's stale in-memory peer list before its
  next metadata poll. It now uses the committed peer table, like launch
  admission. A regression with an empty in-memory list and a committed peer
  failed before the correction and passes now.
- The stop route allowed an unknown lease when its attempt matched. The
  regression proved that it staged a durable stop. Unknown leases now fail
  before input or origin mutation.
- Quiescence ignored pending injections, native queued prompts, in-flight
  injection commands and held recovery. Both local and peer fences now check
  those states plus unsettled durable input/stop intents. The regression proved
  an invalid fence before the fix and covers seven pending-state variants
  afterward.

A real owned child-process regression also proves that disconnecting the
session handle does not release quiescence while its worker still owns the
child. It requires explicit cleanup and reaping before the owner becomes free.
Only the fixture-owned process is terminated.

The four safety/scoped-state tests, initial-run test and committed-discovery
test pass. The authenticated two-server stale-question/restart test, which
failed in the combined run, passes after these corrections. A fresh full
integration run is in progress. The original full run is terminal and was not
restarted.

Issue #27 remains open. Queued generations still need distinct current-run and
queued-run control semantics. Executor quiescence still needs pending-input and
child-process review. Moving the same logical run between executors remains
unproven. Scheduler/catalog composition, exact-head CI, serialized publication
and downloaded release verification are separate gates. Issue #35 still owns
conversation and checkout content transfer.
