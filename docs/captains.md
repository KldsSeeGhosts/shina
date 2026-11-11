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
```

`model::MemoryView` has public `id: String`, `text: String`, `essential: bool`, and `user_controlled: bool` fields. `model::UserProfile` has public `name: String`, `time_zone: String`, and `preferences: String` fields and implements `Default`. Memory user actions save through the separate memory store; the existing application checkpoint-and-flush waits for it along with the index and draft stores.
