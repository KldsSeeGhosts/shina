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

Focused checks pass with 15 catalog tests, 21 identity tests, 58 remote-task
tests and 26 placement tests. The schema 9 to 10 test compares identity and
task-operation/input-intent payloads after reopening. The failed-clone test uses
only a local refusing Git socket and checks that unowned work remains intact.

The full serial run at `7595f1c` passed 2,073 unit tests and 122 integration
tests. The new repository-only live test then exposed a missing local default
branch after fresh clone. The executor now resolves a local base or its origin
tracking ref to a commit without switching its current branch. The placement
still records the selected branch name beside the resolved commit.

The live test passes through two authenticated linked executors and a loopback
Git daemon. Each executor starts with no checkout, clones the same catalog
repository, creates its own isolated worktree at the configured release branch,
and receives one fake-provider input. It then restarts both the home and target
and retries the keyed delegation. The task, clone and worktree remain the same
and the provider input count stays one. The controller has no registered
checkout or dummy clone. The Git source fixture is outside its catalog.

After the branch-resolution fix, the all-target build, formatting and diff
checks passed. The 15 catalog, 58 remote-task, 26 placement and one live
repository-contract tests passed again. The new branch-resolution unit test
also checks that the executor's current branch does not change.

Credential failures still have a bounded Git error string rather than a typed
failure category. Final exact-head CI, schema/protocol publication and #27
execution ownership integration also remain required. No universal exactly-once
remote-provider, cross-runtime speed or complete issue-28 claim is made.

No production CPA, provider login, authentication configuration, real profile or
paid-provider request changed. No worker was replaced.
