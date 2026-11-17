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
