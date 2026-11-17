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
checks pass. A full combined suite has not run.

Issue #27 remains open. Queued generations still need distinct current-run and
queued-run control semantics. Executor quiescence still needs pending-input and
child-process review. Moving the same logical run between executors remains
unproven. Scheduler/catalog composition, exact-head CI, serialized publication
and downloaded release verification are separate gates. Issue #35 still owns
conversation and checkout content transfer.
