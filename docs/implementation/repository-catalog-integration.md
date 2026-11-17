# Repository catalog integration

This branch preserves original #28 commit `9424b27` and integrates published main
`6eff805`. The original Devin checkout remains unchanged. Schema 10 follows the
already published schema 9 task-operation and input-intent tables. The tentative
catalog contract is `logical-repositories-v1` at minor 48, after scheduler 46
and execution leases 47. Execution ownership remains a separate integration gate.

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

Final exact-head CI, schema/protocol publication and #27 execution ownership
integration remain required. No universal exactly-once remote-provider,
cross-runtime speed or complete issue-28 claim is made.

No production CPA, provider login, authentication configuration, real profile or
paid-provider request changed. No worker was replaced.

## Review follow-up

Devin reviewed clean candidate `94c5fd5`. A same-named tag could win over the
selected local or origin branch because the resolver used a bare ref name.
The expanded branch regression failed with the tag's older commit before the
fix. Resolution now probes only `refs/heads/<branch>` and
`refs/remotes/origin/<branch>`. It never interprets a tag as the selected branch.
All 15 catalog units and the paired two-executor fresh-clone/restart test pass.

Further root regressions reproduced the cold-identity and rejected-launch
pollution findings. An explicit project/repository pair now waits for its
selected checkout identity and fails closed on missing, failed or mismatched
identity. Placement validation runs before learning the repository or recording
a provisioning binding. Neither rejection allocates a worker or input intent.

Unresolved identity still produces a terminal rejected receipt. Its message now
states that no worker was created and directs a fresh delegation after executor
discovery, rather than promising that retrying the rejected command can work.

Two more regressions reproduced idle discovery without a full commit and clone
completion after cancellation with a permanent provisioning binding. Binding
discovery now reports a changed batch and requests a full catalog projection.
A clone that finishes after cancellation becomes an available bound checkout.
The cancelled task's placement fails without creating its worktree or starting
a provider. Its clone remains on disk for later use.

The all-target build, formatting and diff checks pass after these corrections.
All 19 catalog units, 58 remote-task units and the live paired two-executor
clone/restart contract pass. The four new catalog regressions failed against
the prior candidate before the corresponding corrections.

The review also identified these limits:

- Repository updates cannot clear fields or replace the clone URL. Remote
  checkout bindings are visible in task placement, not the controller catalog.
- Catalog setup metadata is not carried in the launch payload. Do not claim
  that catalog setup configuration drives executor setup.
- A pre-existing executor catalog id may differ from the controller's id, and
  restore lacks an explicit duplicate binding-key validation.

The issue's acceptance criteria govern closure. Clearing nullable catalog fields,
replacing a clone URL in place and extra malformed-index validation are not
acceptance requirements and will not delay this issue. Controller catalog
binding visibility is also separate from the required per-operation record,
which already includes repository, base commit, checkout and executor.

The required remaining work is narrower: compose the declared #27 execution
ownership dependency, confirm distinct provisioning failure states, and resolve
the setup-configuration contract without moving platform-specific paths or
credentials into shared state. Then run the normal exact-head CI and release
gates. No additional speculative cases are required for closure.

The setup execution contract reuses repository-owned `t3.json` and the existing
worktree-create runner. A fresh repository clone carries that configuration to
the selected executor; the selected branch's setup runs before its provider.
Platform-specific commands, paths and credentials are resolved on that executor.
The catalog's optional `setup` string is descriptive metadata, not a shared shell
command. It is not executed or copied as provider instructions.

The existing two-executor clone/restart test now includes a repository setup
script and checks one setup run per worktree before and after keyed replay.

## Provisioning states

| Condition | Observable state |
| --- | --- |
| No clone on the selected executor | Placement binding `pending`, catalog binding `provisioning` |
| Git reports unavailable HTTP/SSH credentials | Binding `failed`, placement `errorCode` and catalog `reasonCode` are `credentials_unavailable` |
| Other checkout failure | Binding `failed`, code `provisioning_failed`, with the bounded Git reason |
| Missing harness or negotiated repository contract | Launch preflight refuses before allocating the executor thread, with the missing requirement |
| Offline executor | Remote task reports `target_offline`, not a checkout or authentication failure |
| Multiple eligible checkouts | Selection refuses as ambiguous and requires an explicit executor-local project |

Authentication classification uses Git's explicit authentication, disabled
prompt and HTTP authorization errors. An unclassified error remains
`provisioning_failed`; it does not pretend to prove why a private repository
is inaccessible. No credential helper, login or provider setting is changed.

Failure codes persist in the authoritative index and placement record.
Retry clears the failure code and reuses the recorded checkout target.
The catalog now leaves `provisioning` when cloning fails; a successfully bound
clone remains bound if only later placement planning fails.
`credential_failures_are_distinct_and_retry_the_recorded_checkout` checks these
ordinary authentication failures against a transport failure, snapshot
round-trip and retry without starting a provider.

## Published-main composition

This candidate now includes published main `25340bb`, including scheduler and
effective-capability corrections. Repository launch still reserves target
admission before its provider can start. The two scheduler placement fixtures
and two remote-task fixtures include the additive catalog fields.

The all-target compiler check and existing two-executor clone/setup/restart
contract pass on the combined code. No full local suite was repeated. The
subsequent protocol-number update reserves 48 for this dependent catalog
publication, after execution ownership at 47. That dependency still needs to
land before this candidate can publish.
