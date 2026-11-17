# Repository catalog integration

This branch preserves original #28 commit `9424b27` and integrates published main
`6eff805`. The original Devin checkout remains unchanged. Schema 10 follows the
already published schema 9 task-operation and input-intent tables. The tentative
catalog contract remains `logical-repositories-v1` at minor 47. Scheduler minor
46 and execution leases remain separate publication gates.

## Reproduced findings and corrections

Two regressions failed before the correction. Repository registration returned
an embedded synthetic HTTP token in `cloneUrl`. Checkout provisioning deleted
an unowned deterministic staging folder before refusing an invalid clone
source. Shared clone URLs now reject HTTP userinfo and transport passwords at
registration, peer launch and restore. SSH transport usernames remain allowed.
Errors do not echo rejected URLs.

Each clone now reserves an exclusive random staging folder. Cleanup applies only
to that newly created folder. A later attempt leaves existing staging folders
untouched. Interrupted folders remain available for manual recovery. Completed
target clones are adopted through the existing identity check. The target path
uses the full immutable repository id, so changing a display name or sharing an
id prefix cannot select a different or colliding checkout.

Peer catalog learning refuses conflicting existing ids and remote claims across
multiple records before changing the catalog. Restore validates that the clone
source belongs to the saved remote set. Alias additions cannot exceed the
record's validation limit.

Current-main integration also exposed a missing-checkout early return before
durable peer input staging. Both missing and existing checkout launches now use
the same durable input path. The adoption regression checks the saved brief
while provisioning is pending and exactly one marked input after it is ready.
Catalog default branches now enter the placement plan, while explicit bases,
upstream branches and shared placements retain their own semantics.

## Verification and remaining acceptance

Focused checks pass with 14 catalog tests, 21 identity tests, 58 remote-task
tests and 26 placement tests. The schema 9 to 10 test compares identity and
task-operation/input-intent payloads after reopening. The failed-clone test uses
only a local refusing Git socket and checks that unowned work remains intact.

The original checkout adoption fixture is not proof of a successful fresh clone
over two live paired executors. That end-to-end repository-only launch, restart
and same-key replay still need coverage before #28 can close. Credential
failures still have a bounded Git error string rather than a typed failure
category. Schema/protocol publication and #27 execution ownership integration
also remain required. No exactly-once remote-provider, cross-runtime speed or
complete issue-28 claim is made.

No production CPA, provider login, authentication configuration, real profile or
paid-provider request changed. No worker was replaced.
