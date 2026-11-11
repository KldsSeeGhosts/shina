# Captains, scope and memory

A Captain is the assistant, with a profile, one primary home conversation, an ordered workspace scope, and explicit memory. A Workspace is a registered project folder. `Project.captain` still selects that workspace's default captain. It does not restrict that captain to one workspace.

A Thread is a conversation backed by Pi or an ACP session. A captain's home is a thread, not a separate assistant for each project. A Task is a durable delegated brief, state, result and delivery record. Its worker has a separate thread and cannot delegate further. Resetting the captain's conversation does not erase its tasks.

## Continuous homes and migration

Opening a captain opens its primary home regardless of the sidebar's workspace filter. The index stores `captainHomes` and `captainScopes` separately from the profile. Captain ids stay stable so `memory.json` can refer to them. Migration retains unused preset profiles too, rather than deleting profiles and renumbering the roster.

Older indexes can contain several homes for a captain. Migration retains every home, session reference, transcript source and crew. It chooses the home with the latest recorded activity as primary, using the thread id to break ties. All other homes appear through `earlier_conversations` as "Earlier conversations". No session file is deleted. Existing home projects contribute to scope, with the primary home's project first. Crew keeps its historical parent links, but task lists, capacity limits, control, questions and completions route through the primary home. Earlier homes cannot delegate new work.

Legacy project-less Pi homes used thread scratch folders. Migration resolves their relative session paths against those old folders before switching to captain-owned scratch folders. Absolute Pi session paths and opaque ACP session ids remain unchanged.

## Scope

A captain may have zero or more workspaces. An empty scope means a general assistant. Its home runs in `captains/<captain-id>/scratch` under Shina's data directory. One workspace uses that project's folder. Several workspaces use the first project's folder, and the prompt lists every scoped project id and path.

`delegate_task` accepts optional `workspace`, either a project id or a path in scope. It is required with several scoped workspaces. A missing value uses the only workspace or the general assistant's project-less worker scratch folder. An explicit workspace outside scope is rejected before a child or task is created. Lone threads may delegate only into their own workspace.

Scope is an application routing limit, not an OS sandbox. The agent still has the user's filesystem permissions. Captain homes start in read-only mode and receive instructions to delegate code changes. Existing saved permission modes remain user-controlled. Delegated workers use the selected workspace even when it differs from the home.

Changing scope requires an idle home and disconnects its process. Relative Pi session paths first become absolute, so a new working folder does not lose the old session. Running workers keep their original project. The next captain prompt also reports the scope change for ACP sessions whose initial context was already delivered.

`list_threads`, `list_captains`, and `orchestrator_capabilities` report scope as project records, including ids and paths.

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
