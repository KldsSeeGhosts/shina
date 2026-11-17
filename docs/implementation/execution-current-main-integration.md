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
additive index-version-5 object. The tentative lease protocol number is 47,
after the separately published scheduler 46 and before the separately reserved
repository catalog 48. The mobile hello golden and exact-capability gate use
the same number.

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
failed in the combined run, passes after these corrections. The fresh full
integration run passed 121 of 122 tests. Its only failure was
`peer_cli_delegation_recovers_home_and_delivers_one_acknowledged_remote_result`.
Both full runs are terminal.

## Restart acknowledgement and remaining cancellation case

The remaining integration test exposed two further composition defects.
A transport acknowledgement without a lease body used the remote mirror's
older phase, then tried to advance an already uncertain controller record.
The correction uses the canonical book phase. It accepts the transport receipt
without releasing uncertainty and continues collecting exact terminal evidence.
A new regression failed before the correction and passes afterward.

An authenticated valid task reply also left stale offline metadata installed.
A later run could become uncertain again solely because of that older metadata.
Valid replies now update reachability without changing the lease's certainty,
capabilities or authentication-denial state. The narrow database update does not
recreate a removed peer and never regresses its last-reached timestamp.
Unit tests cover those restrictions and continued uncertainty until exact
terminal evidence arrives.

With those corrections the live restart test proceeded further, then failed when
cancellation overtook a follow-up before the executor installed its assignment.
The resulting stop named an unknown lease and was correctly refused. A previous
same-attempt fallback hid this case by weakening lease validation. Do not restore
that fallback.

After the acknowledgement/reachability corrections, all 72 remote-task units,
ten lease-book units and the narrow database reachability regression pass.
Formatting and diff checks passed at that checkpoint.

## Pending-assignment cancellation correction

The failing-then-passing regression now covers an explicit pending assignment
and its exact input command id. The executor validates the full assignment
against the controller, current physical thread and stop control. It records a
terminal never-dispatched cancellation and cancelled-input marker together,
without changing the older run or issuing its stop. A delayed input is refused
before and after restore. Changed owner, executor, thread, attempt or lease
fields and invalid command ids are rejected without mutations.

The controller sends its exact pending assignment instead of substituting an
older accepted lease. It requires the matching input command id and terminal
non-delivery evidence before consuming the saved send/stop. Ordinary transport
acknowledgements still leave execution uncertain.

Cancellation records cannot hide earlier unsettled writers from placement
replacement checks or fence their controls. Stops for an already cancelled
future generation cannot stop the current provider turn. Assignments that
already reached the executor use the normal stop path; this correction does
not manufacture non-delivery evidence for them.

The formerly failing live restart test now passes. A new authenticated two-server
test crashes the controller while a send is held, restores the saved stop,
restarts both sides and replays both keyed calls. It verifies terminal
cancellation, the unchanged earlier executor binding and zero provider inputs
containing the cancelled prompt.

The all-target build and 74 remote-task, 11 lease-book and 32 runtime unit tests
pass at this checkpoint. The new cancelled-generation regression also checks
that its terminal record cannot hide an earlier writer from controller prepare
or executor assignment checks. Exact source still needs a full combined run.

The combined serial run at `6f8deb3` is now complete. All-target build passed,
2,092 unit tests passed with 13 ignored, and all 123 integration tests passed.
This closes the earlier 13 integration failures at that exact checkpoint.

## Accepted queued generations

The next correction separates a running provider turn from accepted inputs
waiting behind it. The oldest unsettled lease remains active. Preparing a
follow-up does not fence controls for that running turn.

Each peer command retains its own execution binding in the durable thread
record. This checkpoint held managed follow-ups in Shina's queue until the prior
turn settled. The provider-queue correction below replaces that restriction. Terminal
observations retain their exact binding even when a later input starts within
the same poll batch.

Cancellation names both the queued assignment and input command. The executor
can remove an accepted input only while its durable evidence proves no adapter
dispatch. It settles that input and lease without stopping an earlier writer.
Unknown or already-dispatched input remains subject to provider settlement.
Current-turn stops retain future managed inputs and carry their own durable
execution binding, so delayed stop replay cannot stop the next turn.

Peer waits may name an exact saved lease. The response does not attach an older
turn's question or completion to a queued generation. Scoped database reads
include every input-bound lease belonging to that thread, but no unrelated
thread leases. The new correction still requires its focused and combined
verification before publication.

The controller retains the last accepted send command after its transport
receipt. A later cancellation can therefore name an input still queued at the
executor. A live paired-server regression verifies this after transport
acceptance, checks that the older execution, run, stop serial and provider
terminal marker remain unchanged, then restarts both sides and replays the
keyed send and cancel without delivering the cancelled prompt.

This correction covers ordinary assigned follow-ups. A late steer still uses
the existing steer-to-follow-up path. Controller-issued lease attribution for
that fallback run needs a separate correction before issue 27 can close.
Cross-executor continuity and dependency composition also remain unproven.

Focused checks passed for this correction: 76 remote-task tests, 11 lease-book
tests, the distinct queued-input run-binding regression and the scoped database
lease regression. The accepted-queue cancellation/restart test and the existing
live late-steer attribution test both passed. A delayed exact writer stop also
has a regression proving that it preserves a later queued input and refuses to
act after the current execution changes. Full combined verification is pending.

Issue #27 remains open. Queued generations still need distinct current-run and
already-accepted queued-run control semantics. Pending-assignment cancellation
is now covered, but a full combined suite on this correction is still required.
Pending-input and owned-child quiescence now have regression coverage; restart
and cross-executor process proof still need review. Moving the same logical run between executors remains
unproven. Scheduler/catalog composition, exact-head CI, serialized publication
and downloaded release verification are separate gates. Issue #35 still owns
conversation and checkout content transfer.

## Delayed-stop replay and preserved input results

The full run at `187a69c` passed 2,095 unit tests with 13 ignored tests, but
failed one of 124 integration tests. The unchanged
`queued_target_response_is_not_provider_acceptance_until_flushed` requires
provider acceptance into its follow-up queue while the earlier turn remains
active. Holding all later managed input outside the adapter breaks that
contract. That failure remains a publication blocker.

The subsequent independent review also found delayed stop reconciliation
rewriting a completed provider result as cancelled. A preserved future input
could then inherit that cancellation result and never report its own completion.
Both regressions failed before this correction.

Stop reconciliation now preserves an existing terminal task outcome and result
id, even before lease refresh observes that outcome. When a preserved input
starts its own run, it clears prior result and acknowledgement state and records
its own reply boundary and attempt message. Replaying the older stop after that
transition fails the execution fence and cannot modify the new turn.

The regression also includes an undispatched input accepted before per-input
execution bindings existed. The exact-attempt stop removes that input, and the
ledger now marks it rejected instead of reconstructing it on the next pass.
The proof requires no provider dispatch and absence from the remaining queue.
An input bound to a later generation remains queued and is not rejected.

The all-target build, formatting and diff checks pass. All 77 remote-task tests,
11 lease-book tests and four injection tests pass. The existing live
accepted-queue cancellation/restart and late-steer attribution tests also pass.
These focused checks do not replace a new full aggregate run.

Task-wide cancellation still needs to include the observed attempt's running
writer rather than only cancelling its newest queued input. Provider-queued
acceptance and exact writer activation need separate attribution. Neither
problem is resolved by this result-preservation correction. No PR or publication
is authorized by these results, and issue #27 remains open.

## Attempt-wide cancellation of undispatched follow-ups

The controller previously treated cancellation of its newest queued lease as
completion of `task.cancel`. An older writer in the same observed attempt kept
running with no remaining controller wait or stop. The queued-input-only peer
control was correct, but composing it as a complete task cancellation was not.

The controller now retains exact assigned input commands by lease id. After
terminal proof for one cancelled input, it commits the next stop for an older
unsettled generation of that same attempt. Each stop has its own durable command
id. It never reuses the latest input command for another generation's control.
The chain does not report remote completion until its older writer also settles.
Transport acceptance of that writer's stop still does not prove settlement.

Selection checks the logical thread, ownership, controller, executor, Captain
and physical worker identities. It cannot expand to a newer generation or a
different attempt. Missing historical input commands or a different physical
worker require explicit reconciliation and leave occupancy unproven. No timeout
or restart substitutes for terminal proof.

One live rerun exposed an earlier queued assignment whose transport receipt had
not yet supplied its executor UID. An unlearned optional UID is not a conflicting
UID. Preparing that exact saved control leaves its lease phase unchanged; the
executor still validates the assignment, physical worker and input before any
mutation, and its matching reply supplies the missing metadata. Conflicting
known physical identities remain refused.

The single-lease peer control still supports cancelling an explicitly named
undispatched input without stopping the writer. Its existing unit test remains.
The task-level live regression now exercises the task-level contract instead:
two queued inputs are cancelled, the running writer settles, both sides restart,
all keyed controls replay, and one deliberate next attempt completes without
duplicate prompts or an orphaned lease.

Focused validation passes 80 remote-task tests, 32 runtime tests and the live
attempt-wide cancellation, offline-cancel fencing and late-steer attribution
tests. These results do not resolve provider-queued input acceptance or writer
activation attribution. Mid-chain process-crash recovery and historical writer
progress still need stronger evidence. The unchanged provider-acceptance
integration failure remains a blocker, so no publication or issue closure is
claimed.

The final focused rerun also passes the all-target build, all 80 remote-task
tests and the live two-input attempt cancellation test after the optional-UID
correction. A fresh run of the unchanged provider-acceptance regression still
fails. This is a confirmed remaining defect on this candidate, not an inferred
or ignored failure.

## Provider-queued acceptance and activation

Same-attempt follow-ups can now enter the provider queue with their own assigned
run id. The durable `peerProviderInputs` map retains that input's binding while
the older writer stays current. A successful prompt response advances only the
queued input's acceptance. Its matching user-message delivery marker activates
the new writer after the older turn settles. An input refusal settles only that
input, without failing the older writer.

Both focused acceptance/refusal unit tests pass. The previously failing
`queued_target_response_is_not_provider_acceptance_until_flushed` now passes
unchanged. The live task-wide cancellation/restart test also passes. That test
holds delivery explicitly so it continues to exercise cancellation before
provider dispatch rather than depending on timing.

Formatting and diff checks pass. These results resolve the known provider-queue
regression, not the remaining main composition, cross-executor continuity,
exact-head CI or publication gates.

## Published-main composition

The candidate now includes published main `25340bb`, including the admission
scheduler and effective-capability corrections. Peer wait snapshots retain both
the scheduler reservation and exact input-bound leases. Launch reserves target
admission while returning the accepted execution binding. Authenticated launch
rejection proof remains available to release provably unused scheduler capacity.

Protocol minor 47 retains both `execution-leases-v1` and
`admission-scheduler-v1`. The all-target compiler check passes after updating the
two scheduler test fixtures with the additive execution fields.

On the combined source, both queued-provider unit tests pass. The unchanged
provider-acceptance integration and task-wide cancellation/restart integration
also pass. No full local suite was repeated.

## Paired executor continuity

The renewed integration branch starts from the parked candidate and includes
main `a580381`, including the recovery-held refusal from PR #66. The parked
checkout and its protocol-number edits remain untouched.

The paired-server regression starts a controller-owned Captain turn, rejects
replacement and a fence while it has pending work, then stops the turn and
verifies quiescence through the real reconciliation handler. The test prepares
the successor using the controller's committed lease book and sends it to a
paired server. The executor accepts one physical worker, replays its receipt
without a second launch, and keeps the same logical Captain and home IDs at
the next generation. A local composer send cannot start work independently
on an executor alias. A fenced alias also refuses delayed inputs, and an
executor that is holding recovery rejects new input before admitting a lease.

The paired fixture tests the identity and writer-fencing contract; it does not
exercise a user-facing “Continue on” action. User-facing home placement and
full production continuity still need verification. Issue #35 owns transfer of
the conversation, checkout, and provider context. Non-authoritative provider
history still leaves interrupted input held or unknown.

Focused verification passed: the paired successor test, 91 remote-task
tests, the task-wide cancellation/restart contract, and the live distinction
between a target queue receipt and provider acceptance. Full exact-head CI
and publication are separate gates.
