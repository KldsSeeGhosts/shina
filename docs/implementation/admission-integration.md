# Admission scheduler integration

The #23 integration branch starts from the #22 source/fence fix on published
main `37363de`. Its scheduler changes come from original Devin commits
`bc9c359` and `d3fe590`, retained as separate integration commits. The original
worker checkout remains unchanged.

Conflict resolution preserves the published identity and durable task-input
contracts. New reservations enter before delegate receipts are returned, but
the provider-start pump remains after receipt staging and checkpointing.
Peer launch and send keep their exact durable input markers and evidence
alongside the new admission fields. A ready placement uses the admission pump
instead of making an unconditional provider connection.

The current protocol uses tentative scheduler minor 46 and the exact
`admission-scheduler-v1` contract. Identity minor 42 and peer task evidence minor
45 remain distinct. Published migrations 008 and 009 are unchanged.

## Stop receipt integration regression

The original candidate treated a cleared `stop_owed` flag as proof that a
cancelled remote attempt no longer occupied capacity. Published #22 clears
that flag when the target accepts the stop command, then keeps the task's
remote state running until the provider actually settles.

The regression delivers that real stop receipt through the peer-reply reducer.
It failed before the correction because the reservation became released.
The correction keeps the slot held and uncertain through transport acceptance,
disconnection, revocation and incompatible replies. Only a terminal result
permits another queued task to start. Repeated reconciliation does not release
the slot twice.

## Placement and restart integration

The first full combined run passed 2,078 unit tests but failed the existing
Captain placement-retry regression. A ready checkout left its crew reservation
queued, so the normal provider connection stayed blocked. Placement completion
now pumps admission after clearing the placement fence. A separate capacity
regression proves that a ready checkout still waits when the account is full.

A live fake-provider rate-limit test also exposed queued delegated work becoming
interrupted on restart. Recovery now preserves only an exact queued attempt with
a durable reservation that was never granted and no recorded start. Previously
granted or uncertain attempts still use history recovery. The live test checks
provider refusal, one accepted input, a blocked second task, and preservation of
the same backoff deadline and queued task across restart.

The focused checks pass with 23 scheduler tests, seven authenticated live
admission contracts and the existing placement-retry regression. The earlier
58 remote-task tests also passed after integration. Full final-tree validation
still gates publication. #23 remains open.

The next full unit run passed 2,080 tests and found one older assertion that
expected every queued follow-up to become interrupted. That regression now
requires the saved never-granted reservation, a queued follow-up with no
provider input, and preservation and single delivery of the preceding result.
The separate previously-granted recovery regression still requires an
interrupted outcome. All 105 task and remote-task tests pass after this
test-contract update.

After integrating main `6eff805`, the final production tree builds with all
targets and all 129 server-runtime integration tests pass. Formatting and diff
checks pass. The exact-head aggregate unit run remains a required PR CI gate.

## Exact-head review corrections

The independent review at `00cea3a` found a capacity leak after a remote launch
was rejected before allocation, and a recovery interlock after a granted attempt
returned to the queue. Both new regressions failed before their corrections.

An authenticated rejected launch receipt now releases only its exact task and
attempt. It must match the saved command UUID and payload hash, name no target
thread, retain the saved origin task/attempt, and come from a peer with the
durable input-evidence contract. Transport errors, authorization failures,
interrupted or malformed replies, older attempts and legacy peers do not prove
absence. An unrelated older uncertain reservation remains held.

The task retains the exact rejected launch UUID independently of the scheduler's
bounded released-audit tail. A deliberate retry can allocate a fresh launch
command after this proof survives restart and audit pruning. Replaying that
retry keeps its one command. Launch transport acceptance remains distinct from
provider acceptance in the operation ledger.

Unknown remote outcomes refuse a newer attempt rather than accumulating more
reservations or replacing unverified work. No timeout or disconnected state
frees that occupancy. Peer evidence checks read the committed peer registry,
not a periodic in-memory copy. The paired-server regression reproduced that
cache window before the lookup correction.

A queued, previously granted attempt may connect solely to read its saved
session during recovery. Its admission delivery flag remains blocked, so that
connection cannot send the brief or native queued work. The regression restores
the saved scheduler and session, reads history, observes interruption, and
checks both reservation release and zero provider prompts.

A live two-server contract fills the target's account and pending queue, rejects
the remote launch without allocating a worker, verifies that the home slot
releases, restarts the home, and explicitly retries after target capacity is
available. The new command starts once, and a keyed replay does not start it
again. This test uses fixture-owned profiles and the fake provider only.

Direct input to a finished delegated child now reserves before `begin_prompt`
can write to its idle provider connection. Its new reservation inherits the
task's coordinator and Captain scopes. A delivered historical result does not
occupy a new slot. Task-owned children do not backfill from old residual input.

New direct sends, crew sends and peer follow-ups check the pending bound before
clearing drafts, staging files or recording target intent. Existing reservations
reuse their queue slot. Restored input accepted before a limit change remains
preserved and accounted for, even when that puts the queue above its new bound.
Task status queue positions now use the scheduler's global priority and fairness
order rather than the old per-home task-list order.

The first aggregate run after that change passed 2,092 unit tests and failed a
crew-tray assertion because demo tasks had no scheduler records. The queued
chip therefore had no position label. Demo tasks now record their active
reservations without pumping admission or starting providers. The existing
layout assertion and all 28 scheduler tests pass with that fixture correction.

These corrections do not complete #23. Exact-head aggregate CI, publication and
the final acceptance audit remain gates. Published migrations 008/009 and
production authentication and CPA configuration are unchanged.

At this checkpoint, the all-target build, formatting and diff checks pass.
All 24 scheduler unit tests, 63 remote-task tests and 47 task tests pass, as
does the new paired-server launch-refusal/restart/retry contract. This is
focused verification, not a replacement for the final full suite or CI.
