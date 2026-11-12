# Composer submissions, queued edits and run control

Implementation report for parts A, B and C. KSG paths below are relative to
`/Volumes/DevDrive/AiStack/ksgcode`. Shina paths are relative to this repository.

## Part A

| KSG mechanism | KSG source | Shina implementation |
| --- | --- | --- |
| Snapshot draft content separately from expanded provider text | `apps/web/src/components/ChatView.logic.ts`, `snapshotComposerSend`; `chat/useComposerSend.ts` | `composer_rich::Snapshot::capture`, `composer_tokens::prompt_for`, `attachments::Prompt::draft` |
| Clear a dispatched draft, retain its snapshot until the outcome | `apps/web/src/components/chat/useComposerSend.ts` | `Thread::send`, `Thread::send_during_run`, `State::send`, `State::promote_client_draft`, `State::complete_client_creation` |
| Recover failed content without replacing later user work | `apps/web/src/components/chat/chatViewSend.ts`, `restoreComposerDraft`; `composerDraftStore.ts` | `Thread::restore_prompt`, `Thread::restore_editor`, `Thread::finish_remote_send`, `composer_rich::prepend` |
| Keep restoration attached to the owning thread after navigation | `useComposerSend.ts`, draft-target and route-generation checks | `State::poll_client`, `State::select_machine`, `State::apply_projection` |

The snapshot has the Rune editor's text, rich HTML, self-contained pill IDs and the
original attachment references. Derived file-pill attachments remain provider input,
not duplicate attachment chips. Restoring combines submitted attachments and later
attachments by ID. The combined draft can exceed a send limit so recovery never drops
content. The normal validation then asks the user to remove attachments before sending.

The remote path now clears at dispatch too. Thread creation, upload refusal and send
refusal restore the captured editor. An accepted receipt never clears subsequent edits.
Recovery into an ordinary draft does not replace an isolated queued-message edit.

Tests include `failed_send_restores_rich_document_pills_and_later_rich_edits`,
`failed_remote_send_restores_rich_snapshot_after_later_edits_and_navigation`,
the existing startup/refusal/image recovery tests, and
`failed_send_rich_draft_and_later_edits_fit_at_supported_widths`.

## Part B

| KSG mechanism | KSG source | Shina implementation |
| --- | --- | --- |
| Separate edit draft and ordinary thread draft | `apps/web/src/components/chat/useQueuedRunEdit.ts` | `queued_edit::QueuedEdit`, `Thread::edit_queued`, `Thread::cancel_queued_edit` |
| Save edits in place, including attachment changes | `useQueuedRunEdit.ts`, `prepareQueuedEditAttachments`; server `orchestration-v2/Orchestrator.ts`, `dispatchQueuedRunEdit` | `Thread::save_queued_edit`, `Thread::update_queued`, `State::execute_command` for `queue.update` |
| Pause the queue-exit watcher during Save; reject stale save completion | `packages/client-runtime/src/queuedRunEdit.ts`, `makeQueuedRunEditStore` | `Thread::reconcile_queued_edit`, `Thread::finish_queued_save`, edit UUID and separate outstanding-save ownership |
| Recover a changed edit only when the ordinary draft is empty | `queuedRunEdit.ts`, `resolveQueuedEditExit`; web `queuedMessageEdit.ts`, `recoverQueuedMessageEdit` | `Thread::reconcile_queued_edit` |
| Preserve edits across navigation and handle another client's removal or run start | `useQueuedRunEdit.ts`, queue watcher | `ThreadView::apply`, `State::apply_queue_receipt`, cached-editor preservation in `State::apply_projection` |
| Queue row wording, Cancel, Save and attachment controls | `QueuedRunsControl.tsx`, `ComposerPrimaryActions.tsx` | `pi_ui::queued_messages`, `ui::composer`, existing attachment controls |
| Edit latest at the start of the composer; promote next | `ChatView.tsx`, `thread.editQueuedMessage` and `thread.steerQueuedMessage` | `pi_ui::composer_queue_key`, Alt+Up and Mod+Shift+Enter |
| Return queued input to the ordinary draft | Existing Shina action, retained as requested | `Thread::clear_user_queue`, `queue.returnDraft`, `State::apply_queue_receipt` |

The server serializes Save with queue start and removal. It checks the runtime
generation, item ID and original content revision before replacing the prompt. The
revision also prevents one client's stale edit from overwriting another client's save.
The existing queue item stays visible and runnable until Save succeeds.

The queue row says "Editing in the composer" and "Cancel". The primary button says
"Update queued message" or "Updating queued message". Existing element keys remain.
The ordinary draft, including its attachments and formatting, returns after Save or
Cancel.

Tests in `src/queued_edit_tests.rs` cover isolation, attachment edits, Cancel, Save,
concurrent saves, generation changes, run start, removal, a save in flight during
removal and an older server. Client-state tests cover machine switches and recovery
on return. The server integration test
`queued_edit_commits_in_place_and_rejects_stale_removed_and_started_items` uses the
fake provider and checks receipt replay. The headless UI test
`queued_edit_controls_save_cancel_and_shortcuts_fit_at_supported_widths` checks the
controls and shortcuts.

## Part C

| KSG mechanism | KSG source | Shina implementation |
| --- | --- | --- |
| Resolve queue versus steer from preference and actual turn capability | `packages/client-runtime/src/state/composerDispatch.ts`; server `CommandPolicy.ts`; web `useComposerSend.ts`, `ComposerPrimaryActions.tsx` | `TurnCapabilities`, `Thread::turn_capabilities`, `Thread::can_steer`, `Thread::follow_up_queues`, server `thread.send` dispatch |
| Pi and Codex steering contracts, including Codex automatic compaction | `packages/provider-pi/src/server/adapter.ts`; server `Adapters/CodexAdapterV2.ts` | `pi_session::TURN_CAPABILITIES`, `codex::TURN_CAPABILITIES`, `codex::Client::command`, `Thread::drive_native` |
| Claude steering only when the installed protocol can interrupt queued input | server `Adapters/ClaudeAdapterV2.ts`, native capability handling | `claude::Client::state`, capability updates, `Thread::turn_capabilities` |
| ACP native steering negotiation and identity-based admission | `packages/provider-acp/src/server/activeSteering.ts`, `adapter.ts` | `acp::supports_active_steering`, `Client::steer`, prompt `_meta`, `_t3/steer` response handling |
| ACP interrupt-and-restart steering, except Copilot | server `CommandPolicy.ts`, `decideSteeringExecution`; `provider-acp-registry/src/server/adapter.ts` | ACP `restart_prompt`, `Client::command`, `Client::settle`; Copilot ID and executable checks |
| Stable Stop target; pending, timed-out and landed states; ten-second retry | web `ComposerPrimaryActions.tsx` | `StopControl`, `Thread::stop`, `Thread::tick_stop`, `ui::composer` |
| Hold queued turns on Stop and after a failed run | server `orchestration-v2/Orchestrator.ts`, Stop and queue-hold updates | `Thread::stop_task`, `Thread::mark_stopped`, `PiSession::queue_held`, `recover_queued_prompt`, `Thread::drive_native` |
| Continue the selected interrupted run before releasing its held queue | web `useComposerResume.ts`; server manual-continuation validation and queue resume | `Thread::resume_run`, `State::resume_thread`, server `thread.resume` |
| Share the send lock and retain ownership after navigation | web `draftSendState` and `useComposerResume.ts` | `resume_command`, `resume_pending`, receipt handling across machines |
| Resume control and persistent right-edge Stop button | web `ComposerPrimaryActions.tsx` | `ui::composer`, questionnaire Stop control, Pixelarticons play glyph |

Native ACP steering requires advertisement version 1, `idempotent: true` and
`holdsPrompt: true`. An explicit `turn_settled` refusal falls back to an app-owned
queued turn. An explicit rejection restores the draft. An unknown or malformed
response produces a delivery warning and does not resend potentially admitted input.
Standard ACP steering instead cancels the current provider attempt, waits for its
settlement and starts the steering prompt in the same logical run. Stop clears a
pending restart before cancelling, so it cannot start afterward. Copilot cannot use
that fallback because cancellation can terminate its background agents, as in KSG.

Stop retains its run ID while the user types. A ten-second timeout enables another
Stop attempt without killing the provider process. A new run resets the old Stop
state. The server rejects a stale run target before stopping anything. Held queued
turns remain editable and survive a server restart without automatic dispatch.

Resume uses the exact wording "Continue where you left off." The server validates the
source run and runtime generation. It starts the continuation before releasing the
queue. A queue-only resume adds no continuation message. A later provider refusal
holds the remaining queue again.

Tests in `src/run_control_tests.rs` cover capability resolution, Stop timeout and
landing, a new run, old-peer refusal, unknown capability fields, held-queue recovery,
exact-source continuation and queue-only resume. ACP tests cover advertisement,
turn IDs, uncertain admission and Stop racing an interrupt-restart. Existing fake
Codex and Claude tests exercise steering, compaction and interruption.

The server integration test
`stop_holds_the_server_queue_and_resume_continues_only_the_selected_interrupted_run`
checks Stop, a restart with held input, stale generation refusal, stale continuation
refusal, continuation-before-queue ordering, stale Stop refusal and receipt replay.
`capability_labels_stop_target_and_resume_fit_at_supported_widths` checks visible
labels, the stable Stop position, timeout and Resume.

## Commands, fields and compatibility

| Addition | Contract |
| --- | --- |
| `thread.send.draft` | Optional rich snapshot. Missing snapshots remain valid for older clients. The server validates it against admitted text and attachments. |
| `queue.update` | `threadId`, `inputId`, `generation`, `expectedRevision`, `text`, `attachments`, optional `draft`. `existingAttachmentIds` lets the remote client retain server-owned attachments and upload only new images. |
| `queue.returnDraft` | `threadId` and `generation`. Removes app-owned queue entries atomically and returns their prompt snapshots. Native queue recovery uses the existing claimed-input path. |
| `queue.edit` | The obsolete destructive operation returns an explicit update-required error. |
| Queue projection | Existing `queue` tuples remain unchanged. `queue_drafts` is an additive scoped-history field. `queue_edit_supported` gates editing. |
| `thread.stop.runId` | Optional for old clients. When present, it must match the current logical run. |
| `thread.resume` | `threadId`, `generation`, optional `continuationOfRunId`, `releaseQueue`. Validates before starting or releasing input. |
| Run projection and storage | `run_id`, `resumable_run_id`, `continuation_of_run_id`, `queue_held`, `run_control_supported`, `turn_capabilities`. Index records use the corresponding camel-case run ID fields. |
| Pending queue storage | `kind: "queued"`, `inputId`, `held`, text, attachments and rich snapshot. Recovery holds records and deduplicates item IDs. Unknown input kinds are ignored. |
| Capability record | Interrupt, active steering, interrupt-restart steering, queued messages and steering during automatic compaction. Missing fields default to false and unknown fields are tolerated. |
| Local-only state | Isolated queued edit, ordinary draft, edit UUID, save command owner, Stop target/phase/deadline and resume command owner. |
| Negotiation | Protocol major remains 1. Minor 8 adds `composer-transactions`; minor 9 adds `run-control`. New commands retain explicit operation grants. |

Older peers can still send ordinary messages. Features they cannot safely perform
produce update-required errors rather than using the removed destructive edit or
an untargeted new-client Stop. Unknown fields remain additive. Unknown send modes
and unknown rich-pill kinds produce an update-required error without discarding
the stored document.

## Adaptations to Shina

Rune uses a real `TextEdit` document rather than KSG's browser draft representation.
Snapshots therefore retain rich HTML and self-contained pill IDs. KSG guards failure
restoration when a newer draft has content. Shina retains its existing prepend-and-merge
recovery policy, now for both rich documents, so both the submission and later edits
remain editable rather than leaving the failed submission in an optimistic timeline row.

KSG identifies queued work by run and message IDs. Shina's single ordered input queue
uses its existing item ID plus runtime generation and a content revision. KSG stores
holds per queued run. Shina holds the thread's ordered queue as a unit, which matches
Stop's scope here. Resume combines KSG's continuation and queue-release operations in
one serialized server command.

The frontend-design skill guided preservation of the existing design system. KSG's
structure and wording use Shina's existing frames, labels, controls and pixel palette.
The Resume icon is the MIT Pixelarticons play glyph, under the existing icon licence.
No new visual style was introduced.

## Validation and limits

Each part has regression tests and passed formatting, build and full tests before its
commit. Visible changes have headless coverage at 620, 960, 1280 and 1920 pixels.
The final checks passed `cargo fmt --check`, `cargo build` and `cargo test`.
The main suite passed 857 tests with 11 opt-in tests ignored. The integration suite
passed all 39 tests.
All checks use temporary `XDG_DATA_HOME` profiles. No production app, user data,
running Shina process or CPA service was modified. `runtime.rs`, `database.rs`,
`composer.rs` and `composer_menu.rs` were not changed.

Part A's initial commit is `2703021`; part B's is `eb5d173`. The final part C commit
also includes the shared remote-receipt and cached-editor integration fixes found
during the combined review. Its hash and final test counts are in the handoff.

Unverified work is native Windows rendering and interaction, manual GUI interaction
on macOS, paid-provider end-to-end calls and real installed mixed-version peers.
Provider checks use local fake providers. Version skew uses protocol and projection
tests. The repository's opt-in installed-provider and paid-provider tests remain
ignored. The Stop timeout uses an injected clock rather than waiting on a real
unresponsive provider.
