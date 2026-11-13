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
question drafts. Explicit answers and dismissals do not recover discarded drafts.
Older projections default these resolution records to an empty list, so this
new recovery behavior is unavailable against an older server. Basic text answers
still use the minor-14 contract.

Item 4 validation passed `cargo fmt --check`, `cargo build` and the full
`cargo test` suite with temporary profiles and `RUST_TEST_THREADS=4`.
The main suite passed 1,084 tests with 11 opt-in tests ignored. The integration
target passed all 70 tests. Native desktop interaction and paid providers remain
unverified.
Literal pastes remain literal and do not fold into files. Copy, cut and undo use
the existing rich-context functions on the scoped answer editor. Question images
use file-path references, not native prompt image parts, so preparation validates
and stages each file separately without changing ordinary prompt image limits.
