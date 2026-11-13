# Approval and question port rules

Reference checkout: `/Volumes/DevDrive/AiStack/ksgcode`, commit `e5ad051f44`.
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
  At 1349 native approval option IDs trim surrounding whitespace, and empty
  trimmed IDs are unavailable. This does not normalize the literal IDs of
  Antigravity question choices.
- Generic ACP supplies no custom options. Antigravity supplies Allow once,
  Allow for this thread, Deny when corresponding kinds exist, then Cancel,
  per `apps/server/src/provider/acp/AntigravityProtocol.ts:74`. It copies
  `agy.security.warning` message, or title, with a 512-character limit and
  ellipsis. Interaction requests are questions, not approvals.
- `packages/provider-kiro/src/server/acpSupport.ts:95` prefers Allow once,
  offers Always allow only if no once option exists, offers Deny only for
  reject_once, and always offers Cancel. Its automatic approval cannot
  choose allow_always because that persists beyond the thread.
- `packages/provider-grok/src/server/acpSupport.ts:103` exposes session
  approval only for a trimmed `allow-edits-session` option ID of kind
  allow_always. At 106 the order and wording are Cancel, Decline when
  reject_once exists, Allow all edits this session when that session option
  exists, and Approve when allow_once exists. Other persistent grants do
  not appear as session approval.

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
- `provider/userInputAttachments.ts:17` resolves each attachment on the provider's
  server and refuses missing files. References use JSON-quoted names and paths:
  `Attached file "name": "path"` or `Attached image "name": "path"`.
  Multiple references join with a newline. They append to a nonempty string
  after two newlines, append as another array entry, or replace an empty answer.
  Extracted documents add ` (text extracted to: "path")`.
- `provider/DocumentText.ts:13` supports docx, xlsx, pptx, odt, ods, odp, rtf,
  epub and ipynb. An existing `.txt` sidecar wins. Extraction serializes with one
  permit, refuses inputs above 16 MiB, runs in an isolated child with a 15-second
  timeout and a 192 MiB heap, and caps stdout at 4 MiB plus 1,024 bytes.
  Sidecars use `.txt.part` then atomic rename. Failed paths have a one-second
  cooldown, permanent rejection on exit 2, and a 1,024-entry failure cache.
- `provider/documentTextWorker.ts:10` bounds expanded XML/archive text to
  32 MiB, output to 1,000,000 UTF-16 units, XML to 200,000 nodes and depth 128,
  archives to 2,048 entries and a 200:1 ratio above 1,024 bytes, and tables to
  10,000 rows and 512 columns. It rejects encrypted archives, reads UTF-8 and
  both UTF-16 byte orders, follows internal relationships only, preserves
  presentation/sheet/chapter order and notes, suppresses hidden and duplicate
  compatibility content, records skipped XML parts, and uses a fixed truncation
  notice. Notebook and RTF parsing preserve their source worker's bounded
  behavior. The worker source is the reference for all format-specific rules.
- Worker lines 19, 44, 98 and 111 suppress script/style/binary, phonetic,
  tracked deletion/move, field-code and paragraph-property content; preserve
  tabs and line breaks; retain one markup-compatibility choice/fallback;
  emit heading prefixes; and normalize table-cell whitespace.
  At 174, ODF repeat counts must be positive integers, trailing empty cells
  disappear, and typed ODF values supply missing displayed text.
  At 203, archives accept only XML, relationships, XHTML/HTML, OPF and SVG,
  reject encryption and unsafe names, validate entry sizes and enforce all
  expansion limits before decoding.
- At 281, notebooks accept string or string-array source, classify code versus
  Markdown, emit cell headings, preserve text/text-plain outputs, and format
  errors with ANSI-stripped tracebacks. There are at most 10,000 cells.
  At 330, RTF starts with the RTF signature, has group depth 128, default uc=1
  and Windows-1252, skips metadata/objects/pictures/field instructions, handles
  Unicode fallback and binary lengths, and accepts only Windows code pages 874
  and 1250 through 1258.
- At 427, DOCX reads word/document.xml; ODF reads content.xml; XLSX follows
  workbook relationships and ordered sheets, shared/inline strings and formulas,
  preserves gaps, and annotates numeric formats with the 1900/1904 date system.
  PPTX follows presentation order and internal speaker-note relationships.
  EPUB follows container, manifest and spine order, decodes chapter hrefs and
  drops fragments. Missing or malformed XML produces named skipped-part notes.
  At 631, output charges raw UTF-16 text against its budget before whitespace
  normalization, removes NULs, trims table cells, collapses excess blank lines,
  trims the result, rejects empty non-truncated extraction and appends the fixed
  truncation notice when a bound is reached.
- `questionAttachments.ts:140` returns edited, unsent custom answers on Stop,
  provider cancellation or timeout to the ordinary thread draft, including
  only referenced thread contexts. It trims each answer, uses two newlines
  between entries and trims the existing prompt's end. At 177 it performs this
  before pruning resolved answer and attachment drafts. Suggested prefills
  remain unedited until changed, per `usePendingThreadRequests.ts:160`.
- `ChatComposer.tsx:3556` uses "Type your own answer, or leave this blank to use
  the selected option". `ComposerPrimaryActions.tsx:70` uses "Submitting...",
  compact "Next"/"Submit", wide "Next question", and "Submit answer"/"Submit
  answers". Previous is absent on the first question.

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
- `packages/client-runtime/src/state/threadRequests.ts:259` offers dismissal
  when the response capability or recorded response mode is message. A live
  question without that mode has no dismissal action.
- `packages/client-runtime/src/operations/commands.ts:968` dispatches the
  dedicated `thread.user-input.dismiss` command with thread and request IDs.
  `Orchestrator.ts:10476` rejects absent or resolved requests with
  "This question has already been answered." At 10500 it requires a user-input
  request with message response capability, otherwise "This question needs an
  answer. Answer it or stop the turn." It resolves the request as cancel.
  `Orchestrator.ts:10338` returns without dispatching any message or provider
  response for a cancelled message-mode request. Dismissal does not need a
  provider session and does not require restoring an archived thread.
- The cancelled resolution invokes the unsent-answer recovery in
  `questionAttachments.ts:140` through `threadRequests.ts:224`. Edited custom
  answers and their referenced thread records return to the ordinary draft,
  but no answer is sent. Suggested, unedited prefills do not return.

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

## Item 3 implementation map

`questionnaire::{Questionnaire,Question,Answer}` carries choice-only input,
literal initial answers, optional requirements and live/message/not-resumable
capabilities. `Questionnaire::{parse,answer,edit,toggle,response}` implements
literal option values, first-option and multi-select answers in selection order,
deduplication, offered-option validation, custom-answer precedence and shared
drafts for matching question IDs. Initial answers seed only new custom-answer
drafts. `Thread::{prepare_questionnaire,prepare_remote_questionnaire}` scopes the
form to the owning thread and request. Authoritative projections with the same
request identity do not replace local answer drafts.

`Questionnaire::pi` maps native select/input/editor requests.
`Questionnaire::rpiv` recognizes the native wire title and options and answers
only that native dialog. The previous collect-and-preanswer mechanism was removed.
`codex_questions` and `claude_questions` implement the provider normalizations
listed above. `codex::Client::item` projects async agent-message questions as
message-mode requests. `acp::{antigravity_question,Client::permission,Client::command}`
preserves exact native choice IDs, does not automatically approve interaction
requests, and sends the selected ID on the ACP permission response.

`Thread::{questionnaire_can_respond,questionnaire_toggle,questionnaire_step,
questionnaire_changed}` applies response locks, literal editing and the
200-millisecond single-select advance. Question and route changes cancel timers.
Rune consumes ordinary text-field input before the app shortcut handler,
per `runtime/input/keys.rs` lines 27 through 85. Digits cannot select an option
while another editable control owns the key.
`pi_ui::questionnaire_drawer` hides custom input for choice-only questions,
suppresses repeated descriptions, renders question Markdown through the existing
renderer, and disables controls for unauthorized, responding or stale requests.

`Thread::{expire_live_question,respond_message_question}` keeps message-mode
questions after turn completion or disconnect, marks lost live callbacks as
not resumable, and formats message answers with KSG's question/newline/answer
format. Answers steer supported active turns or queue behind unsupported ones.
Ordinary composer content remains separate. Persisted history preserves the
question, with explicit non-resumability after restart for lost live callbacks.

Protocol minor 14 adds `question-contract-v1`, the `questionContract` response
marker and `ThreadView.question_contract_supported`. The missing projection
field defaults false. New clients disable question responses against an old
server. The client transport and server connection reject the new marker if
the capability is absent. Minor-13 clients cannot silently submit new-contract
answers.

Pi supplies literal native option values and editor prefill. Codex supplies its
normalized live questions and message-mode async questions, without unsupported
question extensions. Claude supplies trimmed AskUserQuestion data and
multi-select. Generic ACP supplies no questions. Antigravity supplies choice-only
questions through its interaction permission requests.

Tests include `questionnaire_contract_tests.rs`, `question_flow_tests.rs`,
per-provider fixture messages in each provider's tests, native RPIV transport
tests, four-width question layout tests and
`choice_only_questions_use_native_option_ids_and_refuse_an_older_peer`.

## Item 4 implementation map

`question_answers::Draft` retains an editor, files, context records and pending
imports under a runtime/request/question key on the owning thread and machine.
`State::with_question_draft` calls the existing intake, paste, removal and rich
reference functions against that scoped editor. It restores the ordinary editor
and all ordinary attachment/context state before checkpointing. The attachment
intake, staging and upload implementations themselves remain unchanged.
`Thread::{question_reserved_count,save_question_editor,load_question_editor}`
reserves the shared 100 slots across questions, including pending imports.
`Questionnaire::answer` blocks pending preparation and accepts attachment-only
answers only for custom-answer questions.

`State::{attach_paths_to,attach_image_with_options,paste_composer_content,
poll_context_imports}` captures question ownership, refuses choice-only or
responding intake, and prevents an async result from entering an ordinary draft
or a different request. Picker callbacks also check their original machine and
question target. Typing-guard-held questions retain ordinary draft intake.
Question attachment removal uses the existing referenced-image confirmation.
`pi_ui::questionnaire_drawer` calls the existing attachment control and chip
renderer for the question's files.

`Thread::request_question_submission` defers submission while work is pending.
`State::poll_question_answers` stages files with `stage_prompt_files`, uploads
through the existing `client::uploaded_command`, and preserves the request and
answer drafts on failure. It checks the owning runtime generation on completion.
`project_threads` converts only linked strings to KSG's `t3-thread://v2` Markdown
links; non-string selections remain literal. Deleted reference records retained
for undo are not transferred. Responses carry `attachmentsByQuestionId` and
`questionTextById`, never a separate provider turn.

`Thread::respond_question_with_files` resolves files only inside the server's
owned attachment directory. `append_references` applies KSG's string, array and
empty-answer suffix rules. Pi receives its direct string or comma-joined reply;
Codex receives its native answer arrays; Claude receives its updatedInput answers.
Antigravity's choice-only questions cannot take files. Generic ACP supplies no
answer-bearing questions.

`question_document::{ensure,ensure_named}` uses KSG's unchanged document parser,
mechanically bundled with its installed dependencies in
`assets/question_document_worker.cjs`. The Rust caller preserves KSG's process,
heap, timeout, stdout, input, sidecar, serialization and failure-cache limits.
It uses Node already used by Shina's provider workflow. Parser failure remains
optional, as in KSG, and does not reject an otherwise available attachment.
Shina's immutable ordinary-file blobs end in `.bin`; an owned, typed hard-link
view gives the parser the original extension without changing the blob or the
attachment's provider reference. Server extraction runs outside the reducer.
Concurrent retries share preparation by device, command UUID and argument hash.
The durable receipt still controls provider response deduplication.
The generated worker and its dependencies have MIT/ISC notices, also embedded
in the runtime copy.

Protocol minor 15 adds `question-attachments-v1` and the `questionAttachments`
marker. Missing support hides the question's attach action and refuses intake.
The transport rechecks capabilities after reconnect. Server negotiation below
15 explicitly refuses attachment-bearing answers. Ordinary text answers retain
the minor-14 contract.

Tests cover independent drafts, ordinary draft preservation, shared capacity,
pending imports, old peers, native file-path replies, string/array suffixes,
thread links, stale generations, failed preparation and document sidecars.
The native integration test
`question_files_use_server_paths_and_document_text_in_the_native_answer`
checks Claude's actual response, a notebook sidecar and minor-14 refusal.
`question_answer_files_fit_without_changing_the_ordinary_composer_at_all_widths`
checks all four required widths.

Cancelled/expired question resolution records travel in the minor-15 projection.
`Thread::{abandon_question,return_unsent_question_answers}` and `ThreadView::apply`
recover edited custom answers and referenced thread chips before pruning their
question drafts. Explicit answers do not recover submitted drafts.
Dismissals recover edited, unsent custom answers as described under item 5.
Older projections default these resolution records to an empty list, so this
new recovery behavior is unavailable against an older server. Basic text answers
still use the minor-14 contract.

Item 4 validation passed `cargo fmt --check`, `cargo build` and the full
`cargo test` suite with temporary profiles and `RUST_TEST_THREADS=4`.
The main suite passed 1,084 tests with 11 opt-in tests ignored. The integration
target passed all 70 tests. Native desktop interaction and paid providers remain
unverified.

## Item 5 implementation map

`Thread::{questionnaire_collapsed,toggle_questionnaire}` stores the collapsed
question ID, not a request-wide boolean. A different active question opens.
Reopening restores the same editor and attachments. The collapse state remains
local across a matching server projection and resets on a new request or
unmount through thread or machine navigation.
`pi_ui::{questionnaire_drawer,questionnaire_key}` hides only the question and
options, not the answer editor or footer, and disables digit shortcuts while
collapsed. The header uses KSG's exact show/hide tooltip, a truncated question
summary when collapsed, and a position count only for multiple questions.

The header preserves the existing `dialog-cancel` key for the eligible dismissal
action and uses KSG's "Dismiss question without answering" wording. Live questions
without message response mode no longer offer the former generic Cancel button.
The dismiss control has its own Rune press target, so it does not toggle the
card. Rune's `runtime/input/pointer.rs:383` dispatches a press only to its held
target. Enter and Space use the existing focusable button behavior.

`State::dismiss_question` checks capability, permission, request ownership and
the responding lock before sending the dedicated command. Receipt handling in
`State::poll_client` releases only that command's owning thread, including after
navigation. `Thread::dismiss_message_question` validates the request ID,
runtime generation, question kind and message capability before recording its
cancelled resolution and clearing it. It calls the existing unsent-answer
recovery, never the provider response transport or prompt send functions.
Approvals, live callbacks and non-resumable questions cannot use this command.
An eligible message-mode question can be dismissed without a provider session
or restoring its archived thread.
Provider mappings do not change in item 5. Codex's asynchronous agent-message
questions supply message capability and are eligible. Pi, Claude and
Antigravity's live questions are not eligible.

Protocol minor 16 adds `question-dismiss-v1`. The new method requires the
existing `approve` grant. The UI disables dismissal when the capability is
missing. Both desktop transport and direct linked-server calls explicitly
refuse it against an older peer. Reconnect also checks pending dismissals before
replay. A server connection negotiated below minor 16 returns the explicit
update-required error before the reducer. Collapse itself is local and requires
no protocol change.

The final source review also corrected item 1's ACP approval-ID trimming and
Grok's trimmed session-option check in `acp::Client::permission` and
`acp::acp_approval_options`. The new whitespace fixture failed before the fix.
Antigravity question choices continue to preserve their exact native IDs.

New tests first failed for the missing collapse control and unsupported dismissal
command. Unit tests cover eligibility, stale IDs and generations, approval
rejection, no provider answer or new turn, unsent-answer recovery, matching
projections, new request identity and response receipts after navigation.
The headless tests cover collapse, reopen, hidden shortcuts, preserved drafts,
question advance, single versus multiple counts, independent dismissal,
responding, permission and older-peer guards at the required widths.
`question_dismissal_is_negotiated_idempotent_and_starts_no_provider_turn` checks
minor-15 refusal, current negotiation, durable receipt replay, repeated
dismissal rejection and no extra native answer or provider turn.

## Provider summary

| Provider | Approval options and warnings | Feedback | Questions and attachment answers | Dismissal |
| --- | --- | --- | --- | --- |
| Pi | No custom options. Uses the four defaults. Session grants last for the process-local session. | No | Native select, input and editor, plus exact RPIV recognition. Literal option values and editor prefill. Custom answers receive server-owned file references in the native string response. | No live-question dismissal |
| Codex | Defaults for command, file-change, legacy and permission approvals. MCP form schema supplies representable persistence options and labels. | No | Normalized live questions and asynchronous message-mode questions. Live file answers use native answer arrays. Message-mode answers use KSG's question/newline/answer message format. | Message-capability asynchronous questions only |
| Claude | Ordinary permissions use defaults or only Decline and Approve when standing grants are suppressed. MCP uses the four defaults. Reasons and safety explanations follow the adapter rules above. | Ordinary permissions only, not MCP | Trimmed AskUserQuestion data and native multi-select. File answers enter the same updatedInput answers, not another turn. | No live-question dismissal |
| Generic ACP | No custom list. Native permission kinds select allow_once, allow_always, reject_once or cancelled, with KSG's fallback and trimming rules. | No | No user-input questions supplied | No |
| Antigravity ACP | Native kinds supply Allow once, Allow for this thread, Deny, then Cancel. The thread option carries the bounded security warning. | No | interaction_ requests supply choice-only questions with exact native IDs. No attachments or custom answers. | No live-question dismissal |
| Kiro ACP | Prefer Allow once. Always allow appears only when no once option exists. Deny requires reject_once. Cancel always appears. | No | No additional question contract | No |
| Grok ACP | Cancel, optional Decline, optional Allow all edits this session for allow-edits-session only, optional Approve. | No | No additional question contract | No |

## Compatibility summary

Protocol major remains 1. Missing capability entries mean unsupported, not an
optimistic fallback. Older peers retain ordinary messaging and the older
confirmation contract. New operations do not silently become older decisions.

| Minor | Capability | Behavior without it |
| --- | --- | --- |
| 12 | `composer-requests-v1` | New request controls are disabled. Client transport and server negotiation refuse `requestContract` decisions with the explicit update-required error. Missing projected request support is false. |
| 13 | `approval-feedback-v1` | Feedback and approval redirect are refused before declining anything. The older projection cannot safely identify all pending approvals. |
| 14 | `question-contract-v1` | New question responses are disabled or explicitly refused. Missing projected question support is false. |
| 15 | `question-attachments-v1` | Question attachment intake and the attach action are unavailable. Attachment-bearing answers are explicitly refused. Minor-14 text answers remain available. Older projections do not provide the new cancelled/expired recovery records. |
| 16 | `question-dismiss-v1` | Dismissal is disabled or explicitly refused, including after reconnect and in direct linked-server calls. Local collapse remains available. |

No Rune limitation blocked a rule in this port. The frontend-design skill
kept the new controls within Shina's existing pixel palette, fonts and helpers.
No new visual style or provider authentication setup was introduced.

## Final verification and limits

The final code passed `cargo fmt --check`, `cargo build` and `cargo test`,
with temporary `XDG_DATA_HOME` and `LOCALAPPDATA` profiles and
`RUST_TEST_THREADS=4`. The main suite passed 1,092 tests with 11 opt-in tests
ignored. All 71 tests in `tests/server_runtime.rs` passed.
Three additional final runs of `cargo test --test server_runtime` each passed
all 71 tests, in 64.35, 64.31 and 65.58 seconds.

| Item | Regression tests |
| --- | --- |
| 1 | `approval.rs` unit tests, Pi approval/session tests, Codex and Claude native request fixtures, ACP permission fixtures, older-connection negotiation and the four-width approval-options/warnings layout test |
| 2 | `approval_send_tests.rs`, native Claude feedback/redirect integration, old-peer refusal, background sends, upload/draft/navigation/arrival races and the four-width feedback/redirect layout test |
| 3 | `questionnaire_contract_tests.rs`, `question_flow_tests.rs`, Pi/RPIV, Codex, Claude and Antigravity question fixtures, native choice-only integration with an older peer and the four-width choice-only/Markdown layout test |
| 4 | `question_attachment_contract_tests.rs`, `question_attachment_tests.rs`, `question_answers.rs` and `question_document.rs` unit tests, cancelled-projection recovery tests, native file-answer/notebook-sidecar integration with an older peer and the four-width scoped-file layout test |
| 5 | Collapse/projection/navigation tests, request/generation/capability/permission/response-lock guards, archived/sessionless dismissal, native no-answer/no-turn integration with receipt replay and old-peer refusal, and four-width collapse/reopen layout tests |

No identified KSG rule remains blocked by a Shina or Rune limitation.
Unverified work is manual native macOS and Windows interaction, installed or
paid-provider end-to-end calls, and two actual installed mixed-build machines.
Version-skew checks negotiate older capabilities against the current test server.
The added document tests exercise notebook extraction and failure/cache behavior.
Other format-specific paths use the unchanged bundled KSG worker but do not have
separate added Rust fixtures for every parser branch and extraction bound.

No subagents, push or PR were used. The protected application, live user-data
directory, running application processes and CPA services were not modified.
Literal pastes remain literal and do not fold into files. Copy, cut and undo use
the existing rich-context functions on the scoped answer editor. Question images
use file-path references, not native prompt image parts, so preparation validates
and stages each file separately without changing ordinary prompt image limits.
