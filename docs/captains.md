# Captains, scope and memory

A Captain is the assistant, with a profile, one primary home conversation, an ordered workspace scope, explicit memory, crew membership, and planning documents. A Workspace is a registered project folder. `Project.captain` still selects that workspace's default captain. It does not restrict that captain to one workspace.

A Thread is a conversation backed by Pi or an ACP session. A captain's home is a thread, not a separate assistant for each project. A Task is a durable delegated brief, state, result and delivery record. Its worker has a separate thread and cannot delegate further. Resetting the captain's conversation does not erase its tasks.

When machines are linked, the new captain editor's "Lives on" row chooses its home server. The choice is read-only after creation. Folder browsing stays on that server. The Brief scope picker also lists projects on linked peers, grouped by machine. Switching the home server while creating a captain retains the profile fields and clears the previous server's workspace choice. It does not copy folders.

## Continuous homes and migration

Opening a captain opens its primary home regardless of the sidebar's workspace filter. The index stores `captainHomes` and `captainScopes` separately from the profile. Captain ids stay stable so `memory.json` can refer to them. Migration retains unused preset profiles too, rather than deleting profiles and renumbering the roster.

Older indexes can contain several homes for a captain. Migration retains every home, session reference, transcript source and crew. It chooses the home with the latest recorded activity as primary, using the thread id to break ties. All other homes appear through `earlier_conversations` as "Earlier conversations". No session file is deleted. Existing home projects contribute to scope, with the primary home's project first. Crew keeps its historical parent links, but task lists, capacity limits, control, questions and completions route through the primary home. Earlier homes cannot delegate new work.

Legacy project-less Pi homes used thread scratch folders. Migration resolves their relative session paths against those old folders before switching to captain-owned scratch folders. Absolute Pi session paths and opaque ACP session ids remain unchanged.

## Scope

A captain may have zero or more workspaces. An empty scope means a general assistant. Its home runs in `captains/<captain-id>/scratch` under Shina's data directory. One workspace uses that project's folder. Several workspaces use the first project's folder, and the prompt lists every scoped project id and path.

`delegate_task` accepts optional `workspace`, either a project id or a path in scope. It is required with several scoped workspaces. A missing value uses the only workspace or the general assistant's project-less worker scratch folder. An explicit workspace outside scope is rejected before a child or task is created. Lone threads may delegate only into their own workspace.

Remote scope entries contain an owning server id and project id in the additive `captainRemoteScopes` index field. They do not change the home conversation's working directory. A home with only remote scope entries keeps its own scratch directory. `captain.scope` accepts optional `remoteProjects`, an array of `{"server":"<server id>","projectId":9}`. Omitting it preserves remote scope.

For remote delegation, discover projects through `orchestrator_capabilities.peers[].capabilities.projects`, then call `delegate_task` with `{"prompt":"Inspect the project","target":{"environmentId":"<server id>","projectId":9,"model":"<advertised model>"}}`. `target.machine` remains an alias for `environmentId`; `workspace` remains an alias for a target-owned project id or exact opaque path. An explicit project must be in the captain's remote scope. It needs no local counterpart or Git origin. When no explicit project is given, Shina matches the caller's primary local project's Git origin and requires exactly one matching target project. Ambiguous matches return the candidates, never the first checkout. Paths stay on their owning machine.

Remote scope editing requires protocol minor 8 and advertises `remote-project-scope-v1`. Older owners produce an update-required error. The peer launch contract remains unchanged, so targets supporting `peer-tasks-v2` and `thread-read-v2` still accept these tasks. Repository identities use a one-minute cache and a two-second Git subprocess timeout. Startup, project addition and peer linking warm the cache. A cold automatic delegation or peer discovery call waits on background workers and resumes the original request, without blocking the serialized reducer or asking the caller to retry. Lookup failures name the project and owning machine. Discovery includes per-project `repositoryError` data so explicit scoped projects remain usable even when Git fails.

Scope is an application routing limit, not an OS sandbox. The agent still has the user's filesystem permissions. Captain homes start in read-only mode and receive instructions to delegate code changes. Existing saved permission modes remain user-controlled. Delegated workers use the selected workspace even when it differs from the home.

Changing scope requires an idle home and disconnects its process. Relative Pi session paths first become absolute, so a new working folder does not lose the old session. Running workers keep their original project. The next captain prompt also reports the scope change for ACP sessions whose initial context was already delivered.

`list_threads`, `list_captains`, and `orchestrator_capabilities` report scope as project records, including ids and paths.

## Crew and planning documents

Crew membership is separate from task execution. The Brief can recruit an eligible idle root thread, release a member, or move it to an in-scope Captain. Recruitment preserves the thread's history, model and workspace. Running work and unacknowledged delegated results block membership changes. Delegated subagents cannot be recruited as independent roots. Existing workers retain their task records when moved or released.

Membership and saved documents survive a Captain conversation reset. Earlier conversations can read the documents but cannot edit them. Crew questions and approvals appear at their current Captain, not a former Captain inferred from an old parent link.

The Brief opens these documents in right-panel tabs:

- `NOTES.md` is the working plan, also readable by crew.
- `AGENTS.md` contains private Captain rules. Crew agents cannot read it through Captain tools.
- `CREW.md` contains shared crew rules.
- `DIGEST.md` is the saved user-facing digest, opened through the Digest section.

Each document accepts at most 64 KiB of UTF-8 without null characters. Writes check the revision read by the editor. Conflicts keep the draft and require an explicit choice. A linked-machine save stays pending until its exact server and command receipt arrives. A matching document projection alone does not confirm a save.

Drafts belong to a machine, Captain and document. They survive tab, thread and machine navigation in memory, but do not survive quitting Shina. Closing a saving tab retains its pending draft. Offline or unsupported hosts cannot accept new document or crew changes; last-known documents remain readable.

Agent tools are `captain_read`, `captain_document_read`, `captain_document_write`, `captain_crew_list`, `captain_crew_join`, `captain_crew_release`, `captain_reports_list`, `captain_crew_report` and `captain_question_answer`. Only the primary Captain home can mutate Captain planning state through these tools. The Captain receives bounded prefixes of its private rules and plan each turn. Local crew receives a bounded prefix of shared crew rules; full documents are available through authorized reads on the owning server. Pi's read-only policy permits its authenticated Shina tools. Native Claude's read-only policy deliberately denies MCP mutations, including delegation; this port does not loosen that provider policy.

Peer-launched workers receive the shared crew context in each new launch or work follow-up command. The command captures its context once; retries and restored pending commands keep the original payload and UUID even if the rules change. The snapshot never includes private Captain profile instructions or AGENTS.md/NOTES.md contents. Input, editor and select answers retain their exact value without appended context. A brief or follow-up that exceeds the peer's 64 KiB limit after adding context rejects before allocating or requeueing work. Full originating documents remain on the Captain's server.

These operations require `captain-state-v1`, introduced at protocol minor 29. Structured report submission, question answers and the `captain.reports.list`/`captain.report.answer` user RPCs require `captain-reports-v1`, introduced at protocol minor 30. Report wake policies require `captain-events-v1`, introduced at protocol minor 35.

## Independent crew commands

Protocol minor 34 adds `captain-crew-control-v1` and user RPCs `captain.crew.create`, `captain.crew.send` and `captain.crew.stop`. Agent tools are `captain_crew_create`, `captain_crew_send` and `captain_crew_stop`. They control independent roots directly, without fake Task records or delegated flags.

Creation validates the full first input, scoped project, model, harness, access policy and attachment/reference compatibility before allocating a fresh thread ID. Membership and the first input commit together. Invalid input leaves no blank root. A startup failure keeps the accepted root and saves its input for recovery, including a synchronous refusal before the next poll. Creation does not change coordinator focus or tabs, and ordinary draft reuse never takes a blank crew member.

The current create command starts in a local scoped workspace or a project-less scratch folder. It does not initialize a new Git worktree. A multi-workspace Captain requires an explicit project. Unsupported environment fields reject rather than silently changing where work runs. Existing worktree threads can still be recruited without changing their placement.

User creation accepts `{"captain":0,"project":1,"input":{"text":"Work","attachments":[]},"target":{"harness":"pi","model":"exact-id"}}`, with optional title and user access selection. Input also accepts the ordinary rich draft and context envelopes. Image input needs declared model support. Agent creation accepts a plain prompt and uses the existing worker access policy, not a new agent-selected permission mode.

Sends use the native input queue and wait behind the current turn. They never answer a provider approval or a structured human question. Recovered or stopped input requires user review before more work. The queue survives restart held, without automatic replay. Stops require the exact observed `runId` and string `runtimeGeneration` from `captain_crew_list`; a stale stop rejects.

Each agent operation requires a stable UUID `requestId`. Its receipt shares the existing SQLite transaction and receipt table under an internal Captain audit owner. That owner is not registered as a device and has no credentials or grants. Identical same-batch or restart retries return the saved identity without creating, sending or stopping again. A different method/body conflicts. A retry after release, move or deletion reports the original operation plus current existence/membership; it never reattaches or recreates work. Earlier homes may read their old receipts but cannot start new operations. Workers and foreign Captains cannot claim authority through parameters.

`saved:true` means the input or control intent committed, not worker acceptance or completion. MCP replies and provider prompts use the existing commit-ordered outbox. Captain status and work counts follow canonical crew and current Task routes, including after a home reset, rather than historical parent links.

The Brief's "New crew thread" action opens the ordinary composer with a removable crew chip. Its first send uses one atomic `captain.crew.create` command. Rejections restore the submitted text and attachments and retain the chip. A machine change removes the crew binding. A workspace outside the Captain's local scope, a new worktree or a multiple-model selection blocks the send without discarding the draft. Linked creation uses the same upload and staged-attachment claim handling as an ordinary send.

The client saves unsent crew bindings with its draft records. If Shina closes while creation is pending, the restored draft keeps the original command identity and submitted input held. It never turns that uncertain creation into an ordinary send or a second create. Check the Captain's crew on its owning machine before starting more work.

## Lifecycle controls

The durable server supports `captain.lifecycle.preview`, `captain.archive` and `captain.unarchive` through `captain-archive-v1` at protocol minor 31. A preview returns an opaque `captainVersion`, the exact `crewSeen` IDs, affected thread IDs and blockers. Mutations require that version and crew list. A stale confirmation must be reviewed again with a new command ID. An uncertain command retains its original ID and body for receipt lookup or retry.

Archive requires idle current and earlier homes, idle canonical crew, no queued input or pending provider requests, and no active routine or Task. Exact saved report, answer and Task-result notifications may be held instead of delivered. The action records each affected thread's previous archive marker and the IDs of enabled routines it paused. It does not infer membership from historical parents of moved or released crew.

Unarchive restores only those recorded thread markers. Independently archived members stay archived. User-paused routines stay paused; action-paused routines get a new future schedule without replaying missed or already reserved slots. Unarchive does not release held notifications or start providers. An explicit user turn or Deliver now releases notifications through the existing acceptance checks. Opening an archived home does not restore it. Inactive owners cannot write documents, change scope or crew, run routines, send work or bypass the group action with individual thread restore, delete or undo.

Protocol minor 32 adds `captain-promote-v1` for `captain.promote.preview` and `captain.promote`, and `captain-retire-v1` for `captain.demote`. The ordinary thread menu opens the Captain editor in promotion mode. Machine and workspace stay fixed. A surviving retired home can restore its original profile, plan, rules and memory instead of creating a fresh Captain.

Promotion appends a stable Captain identity and uses the existing ordinary thread as its home. It preserves the thread ID, history, native access mode, model, session, machine, project and Git worktree. A separate `scratchDirectory` record preserves project-less working directories across role changes. Pi session paths become absolute before a role change; ACP session IDs remain opaque. No role command grants permissions or starts a turn.

Demotion requires every owned Task result, crew report, question and answer to be settled. It keeps the current home as an ordinary thread, archives earlier homes, releases canonical crew, clears project default-Captain links and pauses routines. Task and report identities do not change. The profile, documents, curated memory and scope remain under the retired Captain's stable index. Restoring that profile requires its surviving ordinary former home and original workspace; it does not resume paused routines. A newly promoted Captain starts with fresh memory rather than copying another profile.

Protocol minor 33 adds `captain-delete-v1`. Use `captain.lifecycle.preview` with `action:"delete"`, then `captain.delete` with the preview's `expectedVersion`, exact `crewSeen`, and `confirmName` equal to the current name. `keepConversations` defaults to true. Deletion requires the same settled report, question, answer and Task-result checks as demotion. It removes the profile's private instructions, books, curated memory, scope and routines in one transaction, leaving a minimal stable-index tombstone. Other Captains keep their indices and records.

Kept homes become archived ordinary threads. Crew always survives. Deleting an archived Captain restores the crew's pre-group archive markers. `keepConversations:false` removes only explicit home IDs, never recursive descendants. The preview refuses that option when Tasks, parent links or historical report/answer records still reference a home. A retired profile no longer owns its former ordinary conversations; deleting those threads is a separate action. Old thread undo cannot restore a deleted Captain's roles or crew.

Archive, unarchive, promotion, demotion and deletion require the existing runtime database transaction. Memory, routine and index changes commit together. File-only legacy mode refuses these multi-record transitions. A persistence failure quarantines the runtime rather than falling back to JSON. These commands do not delete files, worktrees, provider history, external learned memory or backups. The deletion preview reports whether an external memory backend is configured so the client can explain that separate retention.

The Brief and primary-home thread menu offer Captain archive, demotion and deletion instead of individual home archive/delete actions. Archived homes replace the composer with an Unarchive action. The Archive lists retired profiles under "Former captains" and opens their records read-only. Every dialog uses the server's preview and exact crew IDs. A stale confirmation refreshes the preview but requires another explicit press. Linked actions remain pending until their exact machine, method and command receipt arrives. A matching projection alone never confirms them.

## Crew turn reports

A recruited root thread records each observed terminal turn outcome (completed, failed or cancelled) as a durable crew report. The report captures the run id, thread title, the final reply of that turn and the error text; it never infers work completion, questions, artifacts or pull requests from message text. Delegated subagents, peer-launched workers and Captain homes do not produce crew reports.

Reports persist in `captainReports` and use the same durable injection and acceptance machinery as delegated results, subject to each report's saved wake policy. A report keeps one globally unique `crew-` delivery id through capture, restart, home reset and retry. The Captain's home acknowledges a report only when its exact id appears in a Captain home's accepted history; evidence on a different Captain's home never acknowledges it. A reset preserves pending reports and strips their queued copies from superseded homes. After a restart, undelivered React and Context reports hold until the Captain's current home sees an explicit user turn or a resume action; a reset alone does not release them. Ignore reports never deliver. An undelivered React or Context report blocks crew release and move. Deleting the source thread removes its reports unless the deletion is undone.

Captain homes, including earlier conversations, can read their own Captain's reports through `captain_reports_list`. Crew threads can read only reports they authored. Each entry exposes a derived `source` (`turn` for observed outcomes, `agent` for explicit claims), `questionRevision` and the answer or null, and paged reads bound by the protocol page budget return a cursor rather than truncating evidence. Reading never acknowledges a report or wakes a process.

A recruited root can also submit explicit progress, blocked, completed or failed claims through `captain_crew_report`, with an optional question and evidence references. Artifact references are claims only: Shina never opens, executes, downloads or verifies them. A stable `requestId` makes retries return the original report id, and a changed body under the same id is a conflict. A question names its decision maker: only the Captain's current primary home can answer `captain` questions through `captain_question_answer`. The person can answer either recipient through the approved `captain.report.answer` RPC; agents cannot answer `user` questions. Saving an answer, delivering it to the question's source thread and the worker accepting it are three separate facts; only the source thread's exact accepted delivery id settles delivery. Undelivered answers hold across restart like reports, release on an explicit turn in the source thread or the owning Captain's current home, and never replay once accepted. An unanswered question or a pending answer blocks crew membership changes even when its report uses Ignore.

Open user-addressed report questions appear below live provider requests in Needs you. Answers remain pending until the exact command receipt arrives. Report delivery does not answer a question, and saving an answer does not prove that the source thread accepted it. Captain-addressed questions have a separate collapsed list. Artifact references remain unverified claims and do not open paths or run commands. The current question contract supports text answers, not option pickers, reassignment or resolution without an answer.

## Report wake policies and activity

Each Captain has a report policy for new automatic crew turn reports and explicit `captain_crew_report` claims. Legacy Captains and reports default to React.

- React queues the report for the current Captain home and wakes it through the existing delivery path.
- Context holds the report until an explicit user turn in the current Captain home or `task.resumeResults` releases it.
- Ignore retains the report without an injection or wake. It remains undelivered and does not itself block crew release, move, demotion or deletion.

Every report saves its policy at capture. Changing the Captain's policy never rewrites outstanding reports. Context reports still block membership and settlement until accepted. Open questions and pending answers remain blockers under all policies. Only exact provider acceptance evidence marks a report delivered. Reading a report or rendering activity does not.

The user command `captain.reportPolicy` accepts `captain`, `policy` and `expectedPolicy`, with policy values `react`, `context` or `ignore`. The server rejects a stale expected value without changing anything. The policy and command receipt commit in the runtime database transaction. Identical command retries return the saved receipt. Archived and retired Captains keep their policy, but only active Captains can change it. Deletion removes the policy.

The Brief's Crew section has the Reports control. A change stays disabled and shows Updating until its exact receipt arrives. Arrow keys move focus without sending a change; Space or Return selects. Older servers show an update note instead of unsupported choices.

`captain.read` includes `reportPolicy` and a source-backed `activityDigest`. The digest derives Needs you, What's new and In progress from durable crew reports, saved answers, actionable provider requests and canonical crew and Task routes. It does not run a model, acknowledge deliveries or modify `DIGEST.md`. That document remains the separate saved summary.

Digest rows preserve source IDs and distinguish report delivery, answer saving and source acceptance. A saved answer can remain `answer_pending` until the source accepts it. Ignore reports remain visible, and their human questions remain actionable. A report claiming completion does not prove that work shipped. Non-resumable provider requests do not appear as actionable.

The Brief initially shows up to three Needs you rows, two What's new rows and three In progress rows. Show more expands each group within the digest's bounds. A crew-question row opens its answer card on the Captain's current home, not an ordinary message to the worker. Other rows open their source thread when available. Context and Ignore tags describe each report's saved policy, not the Captain's current setting. Delivery text never infers that a Context report is still held.

The digest bounds its sections to 16, 32 and 16 rows respectively, with overflow counts. Titles and summaries have character limits. Its version fingerprints the derived rows and counts without a clock, so unchanged source records produce the same version after restart.

## Explicit memory

Each captain has at most 256 memories. Each memory has an id, text of at most 500 characters, an essential flag, and a user-controlled flag. Shina deduplicates text after collapsing whitespace and lowercasing it.

The primary captain home can call:

- `remember` with `text` and optional `essential`, default false.
- `forget_memory` with `id`.
- `list_memories`.
- `search_memories` with `query`, using case-insensitive substring or all-term matches.

Workers can read their captain's memory and call `propose_memory` with text. A successful worker turn attaches suggestions to its durable task completion message. Suggestions are deduplicated and limited to 8 KiB per task result so retained completion deliveries remain bounded. Suggestions never become memories without captain review and an explicit `remember` call.

Model writes stage in the calling thread during its active turn. Reads in that thread include its staged changes. `agent_end` is not settlement. Shina commits only on successful `agent_settled`; failure, abort, stop and disconnect discard staged changes. A restart never replays an incomplete memory transaction. Commit revalidates staged changes against user edits made during the turn.

User-created memories and any memory the user edits or marks essential are user-controlled. The model cannot change or forget them. The user can edit or delete them through the State API. Remembering duplicate text is idempotent and does not change its essential flag or ownership.

Essential memories enter the captain context under an 8 KiB UTF-8 byte budget, including the label and serialized records. Complete records enter in stored order; records that do not fit are omitted, not truncated. Other memories remain available through the tools. A global user profile holds name, time zone and preferences in at most 1 KiB of serialized JSON. Every captain context includes that profile.

The prompt labels memory and profile as user-curated data, not instructions. Because a process's appended system prompt is fixed, later changes appear as a short `<shina_memory_diff>` data block before the next prompt. The block lists removed ids and new or changed records, including essential-flag changes and profile edits. It does not start a turn by itself. The index stores only context fingerprints, not memory text or profile values. Transcript recovery removes the wrapper from visible prompts and preserves exact task delivery markers.

Shina saves committed memory and the profile in a separate `memory.json`, using the same atomic temporary-file, sync and rename writer as the index. A malformed, duplicate, over-limit or unreadable file disables saving and reports an error. Shina does not overwrite the damaged file. Staged writes never enter the file.

## Reset conversation, keep memory

`reset_captain_conversation` requires an idle home. It retains the old thread and session file, creates a fresh primary thread with no session or transcript, and keeps the captain profile, scope, memory and user profile. It preserves the harness and pinned model choices. Task control and completion route to the new home without changing task ids, delivery ids, attempts, or historical parent links. The next send starts a fresh Pi or ACP session. The old thread remains in "Earlier conversations".

Routines follow the current primary home. Reset rebinds idle routines without changing their IDs, schedules, paused state or reserved slots; an active delivery blocks reset. Startup repairs legacy routing left on earlier homes. Earlier conversations may list routines but cannot create, pause or delete them. Dispatch checks both Captain and home identity, and unanswered user-recipient crew questions hold scheduled work. In the durable server, routine routing and the new home commit in one database transaction. File-only legacy storage does not provide that multi-record guarantee.

## Optional Honcho learned memory

Honcho is a second, optional memory backend. Without configuration, Shina keeps its existing local-only behavior. `memory.json` still owns explicit memories, essential flags, user edits and locks, and the global user profile. Honcho cannot edit them. Enabling Honcho does not upload that file or import earlier conversation history.

Create `honcho.json` in Shina's data directory, outside the repository. Windows uses `%LOCALAPPDATA%\Shina`; other platforms use the same data directory as the index. Restart Shina after changing settings.

```json
{
  "enabled": true,
  "base_url": "http://127.0.0.1:18766",
  "workspace": "shina",
  "token_file": "C:/private/shina-honcho.jwt",
  "recall_tokens": 1500,
  "recall_timeout_ms": 1500
}
```

The base URL is an HTTP or HTTPS origin, without `/v3`, credentials, query parameters or a fragment. Workspace defaults to `shina`. Use a separate workspace from other applications. Set `SHINA_HONCHO_TOKEN` in Shina's launch environment instead of an absolute `token_file` path if preferred. The environment variable takes precedence. Shina reads the token at startup, never logs it and never writes it. It does not create the token file or change authentication. An authentication denial suspends requests until the user repairs the token and restarts Shina.

For a loopback-only server, the user can establish a tunnel before launching Shina:

```sh
ssh -N -L 127.0.0.1:18766:127.0.0.1:18765 user@example-server
```

This is an example, not a tunnel Shina starts. Point `base_url` at the local forwarded port. Use HTTPS for a non-loopback network connection. Redirects and automatic HTTP proxy discovery are disabled so credentials cannot follow a redirect or an inherited proxy.

### What Shina sends

One workspace contains a `user` peer, one peer per captain, and one generic `worker` peer. A captain's initial peer id combines its stable roster index and a hash of its name. Shina persists that mapping in the outbox ledger so renaming the captain does not change its identity. Each home and worker thread has a separate session. Session ids include an installation id and the local thread id, so resets and independent profiles do not collide.

After a successful settled turn, Shina queues the submitted user text and final assistant reply, each capped at 4,000 Unicode characters. Steering text joins the submitted text under the same cap. Internal task-result wakeups do not become human messages. For a delegated worker, Shina sends only its brief and final report. The captain authors the brief and the generic worker authors the report. A generic worker avoids creating a permanent peer for every task; sessions already distinguish the workers.

Shina does not send tool calls, tool output, reasoning traces, attachments or file contents separately, full worker transcripts, failed or aborted turns, user-curated memory records, or the global user profile. A user message or final report can itself contain sensitive information, including quoted file content. Those texts go to the configured server when enabled.

`honcho-outbox.json` atomically stores pending pairs, installation/message counters and captain mappings. It contains conversation text, so protect it like the local transcript. A single uploader retries pairs in order. It checks `metadata.shina_message_id` through Honcho's message-list endpoint before writing each message. This handles restarts, partial batches and a lost acknowledgement after a committed write without resending messages already visible on the server. Honcho v3.5.0 has no server-enforced client idempotency key, so this requires one Shina writer per data directory and read-your-writes visibility from the configured server. It is not a distributed exactly-once guarantee. Delivered pairs leave the outbox; an offline backlog grows until connectivity returns. A write failure reports a passive error and preserves the existing queued pairs. Shina refuses to reuse an outbox against a different server or workspace.

### Recall, tools and health

Before each primary captain prompt, Shina retrieves the captain's context for `target=user`, including its representation and peer card. Network work runs off the UI thread. The total recall deadline, including get-or-create calls, is at most 1.5 seconds. A timeout skips remote recall and sends the prompt with local memory unchanged. `recall_tokens` bounds the serialized learned-data block using a conservative one-UTF-8-byte-per-token upper bound. It includes the label and JSON overhead. Very small budgets omit recall. The block lives inside `<shina_memory_diff>` and labels the content as untrusted data, not instructions.

Primary captain homes merge local `search_memories` results with Honcho user-message search. Remote search failures retain local results and add `honcho_error`. `ask_memory(query)` asks the captain peer's dialectic chat endpoint about the user and returns a labelled answer. It has a 15-second deadline. Workers keep local memory search and proposals but get neither automatic Honcho recall nor `ask_memory`.

`State::memory_backend_status()` returns `MemoryBackendStatus::Disabled`, `Connected { queued_count, deriver_pending }`, or `Offline { last_error, queued_count }`. Status errors do not fail a conversation turn. The uploader checks `/queue/status` after draining and every ten seconds while enabled. Pending deriver work is separate from Shina's queued messages.

Honcho's deriver runs its own LLM and embedding calls on the server. Those calls use that server's provider configuration and can incur costs independently of Shina's selected chat model.

The integration targets Honcho v3.5.0. Routes and payloads were checked against its tagged `docs/v3/openapi.json`, `src/routers`, message-filter implementation and TypeScript SDK, plus the public Honcho v3 documentation. Fake HTTP server tests require no live server, token or SSH tunnel.

## Backend UI API

These methods belong to `model::State`. Captain and thread ids are `usize`; `ProjectId` is a `usize` alias. All mutations report validation errors without changing persisted data.

```rust
pub fn captain_home(&self, captain: usize) -> Option<usize>;
pub fn earlier_conversations(&self, captain: usize) -> Vec<usize>;
pub fn captain_scope(&self, captain: usize) -> Vec<ProjectId>;
pub fn set_captain_scope(&mut self, captain: usize, projects: Vec<ProjectId>)
    -> Result<(), String>;
pub fn reset_captain_conversation(&mut self, captain: usize) -> Result<(), String>;

pub fn memories(&self, captain: usize) -> Vec<MemoryView>;
pub fn add_memory(&mut self, captain: usize, text: String, essential: bool)
    -> Result<String, String>;
pub fn edit_memory(&mut self, captain: usize, id: &str, text: String)
    -> Result<(), String>;
pub fn delete_memory(&mut self, captain: usize, id: &str) -> Result<(), String>;
pub fn set_memory_essential(&mut self, captain: usize, id: &str, essential: bool)
    -> Result<(), String>;
pub fn user_profile(&self) -> UserProfile;
pub fn set_user_profile(&mut self, profile: UserProfile) -> Result<(), String>;
pub fn memory_backend_status(&self) -> MemoryBackendStatus;
```

`model::MemoryView` has public `id: String`, `text: String`, `essential: bool`, and `user_controlled: bool` fields. `model::UserProfile` has public `name: String`, `time_zone: String`, and `preferences: String` fields and implements `Default`. Memory user actions save through the separate memory store; the existing application checkpoint-and-flush waits for it along with the index and draft stores.
