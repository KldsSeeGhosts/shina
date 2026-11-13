# Approval and question port rules

Reference checkout: `/Volumes/DevDrive/AiStack/ksgcode`.
This is the source inventory recorded before implementation. Completion and
verification are recorded separately, not implied by this inventory.

## 1. Approval decision contract

- `packages/contracts/src/providerPolicy.ts:58` defines `accept`,
  `acceptForSession`, `acceptAlways`, `decline`, and `cancel`. Options have
  a nonempty label and an optional nonempty warning.
- `apps/web/src/components/chat/ComposerPendingApprovalActions.tsx:26` defaults
  to Cancel, Decline, Always allow this session, Approve, in that order.
  An explicitly empty options list stays empty.
- The same component at lines 45 through 144 puts decline and accept in the
  primary row, with every other decision in More approval options. Composer
  labels truncate at 160 px; inline labels wrap. Warnings appear as an alert
  icon and tooltip. Disabled, responding, or unauthorized requests disable
  both rendering and handlers. Accept uses the primary tone.
- `packages/provider-pi/src/server/adapter.ts:1509` keys session grants by
  title length, title, and message. At 3103 only acceptForSession records a
  grant. At 3724 accept and session accept send confirmed true, decline sends
  confirmed false, and other decisions send cancelled true. Pi supplies no
  approval options, so the default list applies.
- `apps/server/src/orchestration-v2/Adapters/CodexAdapterV2.ts:5531` and 5602
  keep command and file-change session allowances separately. Session or
  always accept records an allowance for that request kind. Always maps to
  session on the native wire. These requests supply no options.
- The same adapter at 605 maps legacy approvals to approved,
  approved_for_session, a denied rejection with "User declined the request.",
  or abort. At 638 permissions accept returns requested permissions with
  turn scope, session accept uses session scope, and all other decisions
  return empty permissions with turn scope.
- `apps/server/src/provider/CodexMcpElicitation.ts:60` classifies persistence
  values case-insensitively, with session taking priority over always,
  permanent, forever, persistent. At 112 it reads metadata, form enums,
  oneOf entries, and persistence booleans for labels and availability.
  Only representable persistent acceptances appear. At 179 URL forms
  decline, required fields without answers decline, selected persistence
  and defaults populate content, and accepted persistence sets `_meta.persist`.
  Unsupported elicitations and elicitations without an active turn decline
  without a card, per `CodexAdapterV2.ts:5739`.
- `apps/server/src/orchestration-v2/Adapters/ClaudeAdapterV2.ts:3963` offers
  only Decline and Approve when suppressAlwaysAllowRule is true. Otherwise
  ordinary permissions use the default options. Reasons strip terminal
  controls. Full-access requests explain ask_rule versus provider_check.
- The same adapter at 4002 returns allow with updatedInput, toolUseID and
  user_temporary classification for accept. Session accept returns
  user_permanent and session-destination suggestions, falling back to an
  allow rule for the tool. Managed ceilings remove setMode suggestions.
  Denial uses trimmed feedback, else "User cancelled tool execution." or
  "User declined tool execution.", user_reject, and interrupt true only for
  cancel. MCP elicitations at 4083 and 11455 use the four default options,
  remember session grants by server name and message, and distinguish
  cancel from decline. MCP does not support feedback.
- `packages/provider-acp/src/server/adapter.ts:1344` maps accept to
  allow_once, session to allow_always with allow_once fallback, decline
  to reject_once, and cancel to cancelled. Automatic approval prefers
  allow_once. At 6690 an unavailable session decision downgrades to accept.
  Accept/session records turn/session policy grants.
- Generic ACP supplies no custom options. Antigravity supplies Allow once,
  Allow for this thread, Deny when corresponding kinds exist, then Cancel,
  per `apps/server/src/provider/acp/AntigravityProtocol.ts:74`. It copies
  `agy.security.warning` message, or title, with a 512-character limit and
  ellipsis. Interaction requests are questions, not approvals.
- `packages/provider-kiro/src/server/acpSupport.ts:95` prefers Allow once,
  offers Always allow only if no once option exists, offers Deny only for
  reject_once, and always offers Cancel. Its automatic approval cannot
  choose allow_always because that persists beyond the thread.

## 2. Feedback and redirect

- `packages/client-runtime/src/state/threadRequests.ts:29` sends trimmed,
  nonempty feedback only to a live request with supportsFeedback, as decline.
  Redirect declines live requests sequentially and stops on any failed receipt.
- `apps/web/src/components/chat/useComposerSend.ts:531` targets feedback
  only when the first displayed approval supports it.
- At 927 redirect waits for all attachment uploads before declining.
  Failed upload or permission validation leaves approvals untouched.
- At 955 only still-pending approvals shown at send time, on the same route,
  are eligible. At 960 pending attachment preparation, prompt changes,
  attachment ID/order changes, reply changes, draft identity changes,
  navigation, or an in-flight approval response prevent dispatch.
- At 999 feedback refuses attachments or added context with "Send approval
  feedback without attachments or added context." It clears the draft only
  on success and only if the draft and route remain unchanged.
- At 1035 redirect checks changes both before and after sequential decline.
  Newly arriving approvals prevent dispatch and preserve the draft.
  `chat/composerSend.logic.ts:63` defines the retry notices.
- Normal validation and local commands precede decline. Background submission
  goes through the same path.
- `ChatComposer.tsx:1154` forces redirects over displayed approvals to steer
  the waiting turn, regardless of the ordinary follow-up queue preference.
  Typing-guard-held approvals do not change that preference.

## 3. Question contract

- `packages/contracts/src/providerRuntime.ts:145` and
  `orchestrationV2.ts:1353` define option values, allowCustomAnswer,
  initialAnswer, multiSelect, and required. multiSelect defaults false.
  Runtime response capabilities at 1228 are live, message, or not_resumable.
- `packages/client-runtime/src/state/pendingUserInput.ts:19` preserves
  explicit option values literally, trims fallback labels, deduplicates
  selections, and keeps selection order.
- At 60 attachmentsBlocked prevents answering. Choice-only questions ignore
  custom text. Literal answers preserve whitespace and empty strings.
  Ordinary custom text trims. Custom text wins over selections. Multi-select
  returns all normalized selections; single-select returns the first.
  Attachments alone can provide an empty answer only when custom input is
  allowed.
- At 93 typing custom text clears selection when literal or nonempty.
  At 134 toggling an offered option clears custom text; unknown options do
  nothing. At 161 submission requires an answer to every question.
- `state/threadRequests.ts:108` seeds initial answers once per question,
  never for choice-only input and never over an existing draft. Draft keys
  include environment, thread and request. Resolved request drafts prune.
- `ComposerPendingUserInputPanel.tsx:53` shows the first pending request,
  keyed by request ID. At 94 message-mode remains answerable after the
  provider turn ends, not_resumable does not. Responding disables actions.
  At 151 single-select advances after 200 ms, multi-select does not.
  At 173 digits 1 through 9 select offered options except while an editable
  control, another pane, background thread or modifier owns the keystroke.
  Changing questions, disabling, or unmounting clears auto-advance.
- At 253 question prose uses ChatMarkdown. Descriptions equal to labels do
  not repeat. Option rows use 10 px gaps, 6 px vertical padding, 14 px labels,
  and a first-line shortcut column. `ChatComposer.tsx:616` uses initialAnswer
  to select literal answer editing and hides custom input for choice-only.
- Pi `adapter.ts:3671` maps a native select's string options to literal values,
  labels empty values "Empty value", and maps input to a "Submit empty value"
  option with description "Send an empty string to the extension." Editors have
  no options and initialAnswer is prefill or the empty string. The first nonblank
  message or placeholder is question text and makes title the header. Otherwise
  title is the question and header is "Question". `userInput.ts:14` recognizes
  RPIV only by exact title and option matching, one native dialog at a time.
  Header defaults are "Select options" and "Select an option". Preview text uses
  the first 600 characters. Multi-select wire answers join with a comma.
- Codex `CodexAdapterV2.ts:4454` normalizes missing or blank question ID to
  question-1 with a one-based index, header to "Question", question to
  "Choose an answer.", option label to Option 1 with a one-based index, and
  description to its label. These native questions supply no multi-select,
  literal prefill, custom-answer restriction or explicit option value.
  At 5432 completed async agent messages with questions become message-mode
  requests with zero-based string IDs, "Question" headers, title as question
  and native string options as labels.
- Claude `ClaudeAdapterV2.ts:4907` unwraps a type-record input, discards blank
  questions and options, trims question text and labels, keys each question by
  its question text, defaults the header to Question 1 with a one-based index
  and blank descriptions to the option label. It preserves previews and maps
  multiSelect only when true. No other question fields are supplied.
- Antigravity `AntigravityProtocol.ts:111` recognizes interaction_ tool IDs,
  rejects empty or duplicate option IDs, defaults blank titles to
  "Choose an option.", bounds question text to 8,000 characters and labels to
  512 characters with "...", and supplies choice-only single-select questions
  with exact option IDs as values. Labels use trimmed name or option ID.
  Replies prefer exact IDs, with a uniquely matching label as fallback.
- Message-mode answers use question text, a newline, and the trimmed answer,
  joining array answers with ", " and question replies with two newlines,
  per `Orchestrator.ts:10170`. Optional unanswered questions are skipped.
  A response steers the active run when supported, otherwise queues after it.
  No replies produces "Enter an answer before sending." A dismissed
  message-mode request dispatches no message.

## 4. Attachments and references

- `chat/useComposerAttachmentIntake.ts:125` refuses question attachments
  without server support, for choice-only questions, or while responding,
  with "This question cannot accept attachments."
- At 142 async imports capture their owning draft before awaiting. At 151
  capacity reservations happen synchronously across all question drafts,
  including preparing attachments. Existing intake validation still applies.
- `ChatComposer.tsx:629` keys attachments by environment, thread, request and
  question. At 637 open attachment targets require custom-answer eligibility
  and no response in flight. Target changes invalidate async imports.
- `apps/web/src/pendingUserInput.ts:16` projects referenced thread records
  into answer links, preserves non-string answers, and transfers only records
  referenced by the answer, not records retained for undo.
- Question answers carry attachmentsByQuestionId and questionTextById under
  `packages/contracts/src/providerPolicy.ts:90`. The provider adapters must
  receive the same attachment-answer mechanism, not an unrelated new turn.

## 5. Collapse and dismissal

- `ComposerPendingUserInputPanel.tsx:107` stores a collapsed question ID,
  so advancing to a different question reopens it. Request identity remounts
  the card. Collapse never clears the draft.
- At 224 the toggle uses "Show the question and its options" or "Hide the
  question and its options". A collapsed header shows question text and,
  only for multiple questions, its position and total.
- At 253 dismissal says "Dismiss question without answering", does not toggle
  the card, and disables while responding. The pending request projection
  determines eligibility from the response capability and response mode.

## Item 1 implementation map

| Rule | Shina function |
| --- | --- |
| Decisions, defaults, explicit empty lists, warning text | `approval::{decision,response,defaults,options,validate}` |
| Primary versus More options, warning tooltips, disabled handlers | `ui::approval_actions`, used by `ui::dialog` and `crew_ui::needs_you` |
| Permission grant, responding lock, request identity | `State::respond_to_request`, `Thread::respond`, `State::poll_client`, `server::connection`, `Client::can_approve` |
| Pi wire replies and process-local grant identity | `Thread::respond`, `Thread::remember_session_approval`, `pi_session::approval_key` |
| Codex modern and legacy decisions, kind-scoped allowances, permission scopes | `codex::Client::{server_request,answer_decision,approval_kind}` |
| Codex MCP schema, persistence, defaults, required fields and app label | `approval::{mcp_response,mcp_options,mcp_app_name,mcp_form,mcp_metadata}`, `codex::Client::server_request` |
| Claude permission updates, cancellation interrupt, restricted options and native safety asks | `claude::Client::{server_request,answer_decision}` |
| Claude MCP consent without unseen defaults, session grants, cancellation | `approval::claude_mcp_acceptance`, `claude::Client::{server_request,answer_decision}` |
| Generic ACP decision-to-option IDs and provider-specific lists and warnings | `acp::Client::{permission,command}`, `acp::acp_approval_options` |

Pi supplies no custom approval options. Codex supplies defaults for ordinary
approvals and schema-derived persistence options for MCP. Claude supplies defaults
for ordinary approvals, only Decline and Approve when a standing grant is
suppressed, and the four default choices for MCP. Generic ACP supplies defaults.
Antigravity, Kiro and Grok supply their KSG-specific lists. Antigravity supplies
the bounded native security warning.

Protocol minor 12 adds `composer-requests-v1`. New responses carry
`requestContract: true` and the decision-preserving response envelope.
The server refuses that marker on connections negotiated below minor 12.
The client refuses it when the server lacks the capability. Missing
`ThreadView.requests_supported` defaults to false and disables the new controls.
The hello reply also reports `canApprove` from the existing `approve` grant.
Older clients can still send their existing confirmation envelope. Native
adapters translate it to accept, session accept, decline or cancel.

Tests cover default and empty lists, warning text, permissions, modern and legacy
Codex decisions, Codex MCP persistence, Claude session updates and cancellation,
Claude suppressed options and MCP grants, generic and specialized ACP options,
and Pi's grant lifetime and failed-send guard. The integration test
`composer_request_capability_refuses_new_decisions_on_an_older_connection`
checks negotiation and explicit refusal. The headless test
`approval_options_warnings_and_responding_guards_fit_at_supported_widths`
checks 620, 960, 1280 and 1920 px.

One default-concurrency suite run failed in the existing
`client_draft_archive_discard_and_delete_are_local_and_restore_the_tab_on_undo`
test because its expected undo entry was gone. The isolated test passed.
No undo code changed. Subsequent full checks use `RUST_TEST_THREADS=4` and
temporary `XDG_DATA_HOME` and `LOCALAPPDATA` profiles.

## Item 2 implementation map

`approval_send::State::{begin_approval_send,poll_approval_sends}` ports
`useComposerSend.ts:927–1070`. It captures displayed request IDs, prepares all
attachments before a decision, filters resolved requests, declines remaining live
requests sequentially, and rechecks content, ordered attachment IDs, editor identity,
runtime generation, navigation, pending imports and newly arriving approvals.
`Thread::{pending_approvals,shown_approvals}` includes queued requests while excluding
typing-guard-held requests. `State::respond_to_approval_id` targets queued approvals
without changing the displayed dialog. Failed responses stop the transaction.

Feedback targets the first displayed request only. It trims text, rejects
attachments and context, sends decline with native feedback rather than a new
prompt, and clears only the unchanged owning draft after success. Native feedback
is retained independently of provider history by `history_with_approval_feedback`.
`State::dispatch_send` and `send_background` share this transaction. Local commands
and normal validation run first. Backend or typing-guard-hidden sends do not
implicitly decline approvals. Dispatch releases the typing guard after capturing
the displayed approval IDs, as in `ChatComposer.tsx:2169`.

Protocol minor 13 adds `approval-feedback-v1`, required by the desktop for both
feedback and redirect because old servers cannot project all queued approvals.
Feedback commands are rejected explicitly on connections negotiated below 13.
The underlying decision capability remains `composer-requests-v1`.
The hello record reports `canOperate` separately from `canApprove`; redirect
checks both before declining. A missing operate grant preserves the approval
and the draft.
All providers use the item 1 decision mappings. Claude ordinary permission requests
support feedback. Pi, Codex, generic ACP and Claude MCP do not.

Tests in `approval_send_tests.rs` cover plural sequential declines, arrivals during
decline, changed drafts, later edits during remote response, background send,
feedback history, preparation refusal, local commands, old peers and non-resumable
requests. `approval_feedback_and_decline_redirect_use_native_decisions_and_refuse_an_older_peer`
checks native Claude denial feedback, decline-before-redirect, transcript persistence,
no extra feedback prompt and minor-12 refusal.
`approval_feedback_and_redirect_composer_fit_at_supported_widths` checks all four
requested widths.
