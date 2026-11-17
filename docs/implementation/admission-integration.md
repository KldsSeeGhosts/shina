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

The focused integration checks pass with 21 scheduler tests and 58 remote-task
tests. Full combined-tree validation and the live admission contracts still
gate publication. #23 remains open.
