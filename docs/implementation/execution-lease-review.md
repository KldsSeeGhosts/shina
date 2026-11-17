# Execution lease review and bounded corrections

The review uses original #27 candidate `be0e3f2` in a separate root-owned checkout.
The original worker checkout remains unchanged. This is not yet a current-main
integration or a publication candidate.

Three new regressions failed against the original candidate:

- A queued follow-up could prepare a newer generation while an earlier lease
  was uncertain. Checking only the latest lease also missed uncertainty in an
  older running generation behind a prepared follow-up.
- Keyed prepare replay accepted a different controller, workspace, executor or
  Captain if the logical thread/run and operation fields matched.
- Executor assignment replay checked only part of the immutable binding and
  accepted a different executor, owner, attempt or delivery.

The correction checks the complete saved ownership and replay binding before
returning a lease. New generations retain workspace/owner/controller authority.
Queueing cannot bypass uncertainty in any previous writer or move an unsettled
writer to another executor. A settled run may still deliberately change its
executor. Assignment replay checks the saved immutable fields and executor-local
alias/UID; it does not treat a matching lease UUID alone as authority. The
existing steer path reuses the saved queue flag instead of changing it on replay.

## Verification

The corrected tree passes ten execution-book tests, four execution-focused
runtime/model tests, 49 remote-task tests and 30 runtime tests. Two authenticated
paired-server tests also pass: lost launch-response replay with identity grants,
and distinct executor aliases under one controller Captain. All-target checking,
formatting and diff checks pass. A full combined-main suite has not run.

## Remaining review and integration

The paired-executor test manually assigns a different logical worker thread to
each executor. It proves the shared Captain id and one unchanged home, but is
not evidence of moving the same active logical run between executors.

The queued-generation control contract still needs review. The original test
explicitly makes a prepared follow-up the active generation and refuses controls
for the older provider turn before that turn settles. Current-run cancellation
and queued-run cancellation must remain distinct without stopping newer work.
The executor's hard disconnect/quiescence path also needs coverage with real
child-process ownership, pending inputs and restart uncertainty.

`apply_execution_reply` still needs a source/immutable-binding audit. It checks
lease identity but uses an allow-rebase path for later replies. Compose the
published #22 trusted source/request-fence work rather than restoring the old
unscoped input-evidence path.

Before publication, integrate current identity, task evidence, strict projection
and scheduler code. The original minor 44 and documentation reserving migration
9 are obsolete. Published migrations 008/009 must not change; scheduler 46 and
catalog 47 are pending, so leases need a separately serialized number. #35 still
owns content/context transfer. #27 remains open.
