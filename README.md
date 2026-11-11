# Shina

A native desktop chat workspace built with [Rune](https://github.com/anomalyco/rune) and the [Pi coding agent](https://github.com/earendil-works/pi). Shina is a working-name prototype, not a T3 Code or Capy client.

The standalone app's interface is entirely Rune. Pi runs as a hidden background RPC process. There is no terminal emulator, webview, browser server, or Electron runtime in this app.

## Run on Windows

The development tools and Pi are already installed on this workstation.

```powershell
cargo run
```

Shina 0.9 keeps all your projects and chats in one window. Use Add project for **Open folder...** or **Clone repository...**. New thread opens a project picker, including **No project**. Threads from different projects run concurrently and can share the tab strip.

Lone threads use the [T3 Code lifecycle](docs/t3-thread-management.md). Pinned and Active read as one list, followed by collapsed Snoozed and Settled shelves. A running thread stays in its place and shows Working. A collapsed shelf keeps the open thread visible. Settled starts with 10 rows and adds 25 per page. Hovering a row shows Snooze and Settle; right-click opens the rest of its actions at the pointer. Shift-click projects in the scope picker to select several.

Settle, snooze, unpin, archive and draft discard offer a grouped five-second Undo in the sidebar footer. Snooze offers one hour, three hours, this evening, tomorrow morning, next Monday and a custom local time or duration. A completion stays snoozed. An approval, question or new failure wakes it early. Unvisited wakes show "Woke". Completed runs show "Done" only after the last visit.

Inactivity settling defaults to three days. The thread menu can disable it per thread and cycle the global interval through off, 1, 3, 7, 14 and 30 days. `autoSettleAfterDays` in the index also accepts other day counts. Manually un-settling opts the thread out until its next user message. Settling releases its Pi process. Captains never auto-settle.

Drag lone-thread rows into Pinned, Active or Settled. Pinned rows reorder with fractional keys. Snoozed rejects drops. Ctrl-click selects several rows, Shift-click selects a range, and right-click opens bulk actions. Drag the sidebar separator to resize it, or double-click to reset its width.

Send a message to start work. Shina prepares Pi in the background when you start a draft, or on send if needed. There is no connection step. Sending a message can consume your CPA upstream quota.

The model and reasoning controls work before a thread has a Pi process. A separate, short-lived Pi discovery process reads your shared CPA catalog and model-specific thinking levels without sending a prompt or saving a session. Choices stay on the thread and apply before its next prompt. The model picker supports filtering by model name, ID, or provider. Shina refuses to send prompts through a direct provider, including when an older session restores one.

For an example conversation without an agent connection:

```powershell
cargo run -- --demo
```

For a native executable with no console window:

```powershell
cargo build --release
.\target\release\shina.exe
```

## Install and update

On macOS, install a native app bundle with:

```sh
sh tools/install-macos.sh
```

This builds a release executable, installs `/Applications/Shina.app` and registers it with Launch Services. Set `SHINA_MACOS_APP_DIR` to another writable applications folder if needed. The launcher uses the existing Pi/CPA environment without embedding credentials. An update keeps the replaced app beside it as `Shina-app.previous`, and a reinstall retains the previous app in `~/Library/Application Support/Shina/App Backups` when no update copy exists. Conversation data is kept either way.

Install Shina for your Windows account:

```powershell
cargo build --release
.\target\release\shina.exe --install
```

This copies the build to `%LOCALAPPDATA%\Programs\Shina\shina.exe`, adds a Start Menu shortcut, registers an Apps & Features entry, and starts the installed copy. The path never changes, so a Start pin or the Caelestia launcher keeps working across updates and development rebuilds. The installed app restores the global project list, threads, tabs and selected thread. Run `shina.exe --uninstall`, or remove Shina in Settings > Apps, to uninstall it. Your data in `%LOCALAPPDATA%\Shina` is kept.

### Automatic updates

During personal development, every push to `main` runs the [fast update workflow](.github/workflows/updates.yml). No PR build, test, lint, or review gate delays that push. Windows x86_64, Apple-silicon Mac, and Linux x86_64 build in parallel. Intel Mac is not built. Each completed platform publishes immediately without waiting for the others; Linux failures cannot delay Windows or Mac updates.

Cargo.toml holds the development baseline, currently `0.11.0`. CI replaces its zero patch with the workflow run number in the build checkout only, for example `0.11.42`. The executable and Mac bundle embed that exact version. Source commits and credentials do not change. Keep the workflow identity and its run counter; when starting a new major/minor line, keep the baseline patch at zero. Platform assets arrive independently, while `SHA256SUMS` grows to cover uploaded assets. Reruns preserve assets already published for the same commit.

The `dev-release` profile disables LTO, enables incremental compilation, and caches dependencies between main pushes. Tests, formatting and Clippy run only when the manual workflow's `checks` input is enabled. The normal push builds once and publishes. Use `cargo run` for local iteration without waiting for CI. Full optimization remains available manually through the existing release profile.

The workflow uses hosted GitHub runners and the job-scoped `GITHUB_TOKEN`; no CPA enrollment, signing secrets, or workstation access is needed. Mac bundles retain the existing ad-hoc signing rather than Developer ID notarization. Hosted runner usage is billed against the private repository's Actions allowance. New main pushes cancel superseded builds. Checksums, saved drafts, explicit restart and rollback remain in the app; they do not delay building or publication.

Linux initially ships as `Shina-linux-x86_64.tar.gz`, built on Ubuntu 24.04. Extract and run `./Shina/shina` with a Vulkan-capable desktop and existing Pi/CPA enrollment. It requires glibc 2.39 or newer. Linux installation and in-app updates are not implemented yet; replace the portable download manually. Windows and Mac retain their existing in-app updater.

An installed copy of Shina updates itself without a download step. It checks GitHub Releases 2 seconds after start and every 30 seconds after each check, through the GitHub CLI and your existing `gh` login, because the repository is private. When a newer version exists, Shina downloads it in the background, checks its size and SHA-256 against the release (the asset digest, or the release's `SHA256SUMS` file) and prepares the replacement next to the installed app. The sidebar shows a passive **Downloading update vX.Y.Z** row meanwhile, then one **Restart to update to vX.Y.Z** button and nothing else.

The button is disabled while any thread is running, waiting for you, configuring Pi, or has queued messages or an open question, while a repository is cloning, and while a rename or captain editor is open. Composer drafts never block, because they are saved. A restart saves and flushes everything first, swaps the executable or app bundle, and reopens Shina on your last workspace.

A normal quit always saves and flushes, but does not install an update. The download stays ready for the next launch. Only the restart button approves installation. This avoids stale install approvals when Windows cancels a session end. macOS uses an atomic bundle exchange. Windows starts a copy of the previous executable as a helper, waits for the UI process to exit, then replaces the executable with `ReplaceFileW`. A failed launch restores and reopens the previous build when the platform reports the failure.

Failed checks and downloads retry at the next interval. The sidebar shows the last failure and a retry button, and session details include bounded updater diagnostics. A real authentication denial suspends automatic attempts and shows the manual remedy, `gh auth login`. After repairing the login yourself, use Retry update check or restart Shina. HTTP 403 alone does not count as an authentication denial. Shina never runs a login or stores a token. Development builds, `--demo` runs and snapshot runs never update themselves.

macOS updates need release assets that `release.ps1` did not publish before: a complete `Shina.app` zipped per architecture, as `Shina-macos-aarch64.zip` (Apple silicon) and `Shina-macos-x86_64.zip` (Intel), plus a `SHA256SUMS` file covering every asset. Build a Mac zip on the matching Mac:

```sh
sh tools/package-macos.sh
```

It reuses the installer's bundle construction without installing anything and writes `target/release-package/Shina-macos-<arch>.zip`. An unwritable applications folder or a filesystem without atomic exchange defers installation without asking for elevation. Failed preparation removes that attempt and retries at the next interval.

For a deliberate manual release instead of the push workflow, choose a version higher than the latest automatic build, update `Cargo.toml` and `Cargo.lock`, and merge it. Build each Mac zip on the matching Mac, then run this from a clean `main`. Restore a zero-patch baseline on a higher major/minor line before resuming automatic builds:

```powershell
.\tools\release.ps1 -MacZips <path to>\Shina-macos-aarch64.zip -Install
```

The release gets `shina.exe`, the Mac zips and a `SHA256SUMS` file. Without the Mac zips the release still publishes, but installed Macs have no asset to update from.

An optional flag adds or focuses a project on launch without replacing the other projects:

```powershell
cargo run -- --workspace C:\path\to\project
```

If automatic Pi discovery fails, set `SHINA_PI_CLI` to the full path of Pi's CLI JavaScript file or a native executable. Earlier setups may still use `STILL_PI_CLI`; Shina reads it as a fallback. On Windows the adapter resolves the installed npm entry directly and launches it with Node. It does not use a command shell or copy credentials.

## What works

- Native chat with streamed text, selectable Markdown, syntax-highlighted code blocks, and a bottom composer. Sending immediately adds the submitted message without waiting for process startup.
- CPA catalog model selection and model-specific reasoning levels through Pi RPC. Pi's existing CPA default is used for new sessions.
- A native sidebar with warm light/dark palettes, compact tabs, search, and an optional overview pane. Installed Anthropic Sans and Serif are preferred for controls and replies. Bundled Inter and Source Serif 4 cover machines without those fonts. Code keeps the platform's monospace font.
- Four crew captains, each an original 40x40 pixel-art portrait that reacts to the crew's live state: Normal when ready, Determined while working, Surprised when a thread needs you, Dizzy after a failure, and Happy when settled. The set is Ember, Juniper, Sable, and Pip.
- Captains are persistent coordinators with their own home conversations and crew, separate from lone threads. Several captains can share a project, and a captain can have no project. New-captain creation suggests a project without a captain. Captain homes never enter thread shelves or thread search. Opening a captain shows its crew beneath it. Opening crew highlights the captain. Crew approvals surface on the parent row; questions stay with the coordinator.
- Independent Pi processes for active threads. Switching threads preserves their drafts and leaves other work running. Shina releases processes after 30 idle minutes and resumes their saved session on the next send.
- Running, needs-input, and failed states. `agent_end` does not finish a run; the adapter waits for `agent_settled`. A finished thread goes quiet instead of showing a status, and a stopped run returns to idle. Background threads that finish get an unread badge.
- Steer an active run with Enter or the send button. Ctrl/Command-click or Ctrl/Command+Enter queues a separate turn instead. Queued messages support edit, remove and Send now actions. Stop returns unsent queued messages to the draft and clears Pi's native queue before aborting.
- Attach PNG, JPEG, GIF and WebP images with the composer button, clipboard paste or file drop. Remove images from their preview chips. Image-only messages work with vision models. Each message accepts up to eight images, 10 MiB per image and 20 MiB combined. Shina validates image contents and dimensions before sending Pi's typed image parts. Text-only models refuse image prompts without clearing the draft.
- The composer context ring shows Pi's current context usage, not total tokens across the session. Hover for usage and capacity, or focus and activate the ring to open its details. It refreshes after startup, completed turns, model changes and compaction. Unknown post-compaction usage has a dashed ring until Pi reports a new measurement.
- Workspace-local `/` completion for Pi extension commands, prompt templates and skills. `/model` opens the picker. `/compact [instructions]` and the context details' Compact context button call Pi's native compaction RPC, without asking the chat model to summarize. Compaction status, summaries and token figures remain visible after resuming the session.
- Fork a conversation or fork before a selected user message from the thread menu. Edit from here rewinds the conversation through Pi's native fork operation and restores the selected prompt and its images. Files are not reverted, and the original Pi session stays on disk. Source threads keep their own processes and sessions.
- Native confirmation, selection, editor and text replies for Pi RPC dialog records, including concurrent requests and expiration. Confirmations require the user, unless they previously chose Allow for session for that exact request. The permission choice lasts only for that Pi process.
- One atomic global index remembers projects, thread ownership, titles, pins, archive state, model/thinking choices, tabs and Pi session paths. Opening a saved thread restores its transcript, tool summaries and tool results automatically.
- Process crashes show an inline error with Retry. Shina never automatically replays a prompt that might already have run tools. Retrying an accepted run can repeat tool actions, so check any partial result first.
- Live progress distinguishes model thinking, text generation, tool execution, retries, and compaction. Rejected prompts return to the draft; cancellation releases a pending dialog before clearing the queue and aborting.
- Projects are thread attributes, not folder groups. A project scope filter narrows the sidebar. Adding or focusing a project keeps all running threads and unsent drafts.
- Right-click a sidebar row or use its hover actions for rename, pin/unpin, archive/restore and confirmed deletion. The chat header offers the same actions. Deletion stops that thread and all its delegated descendants, then removes them from the index and tabs. The confirmation names this policy. Project files and Pi session files stay on disk.
- Project actions in the project picker remove a project from the list without deleting its folder or threads. Add project lists removed projects for restoration. Existing tabs and agents keep working.
- **Clone repository...** accepts `owner/repo`, a GitHub URL, or any git URL. Shorthand runs `gh repo clone`; URLs run `git clone`. Both run in the background without a console window, with prompts turned off, and use your existing git or `gh` login. Shina never asks for or stores credentials. The destination defaults to the current project's parent folder and remembers the last one used. Shina refuses a destination that exists and is not empty, then opens the new clone when it finishes.

Keyboard shortcuts are Ctrl+N or Ctrl+Shift+O for a new thread, Ctrl+Alt+N for a project-less thread, Ctrl+O to open a folder, Ctrl+B for the sidebar, Ctrl+L for the composer, Ctrl+= and Ctrl+- to zoom the whole window (Ctrl+0 resets it), and Escape to stop a run. Ctrl+Shift+S settles or un-settles, Ctrl+Shift+P pins or unpins, Ctrl+Shift+[ and Ctrl+Shift+] navigate visible rows, and Ctrl+1 through Ctrl+9 jump to a row. F2 or double-click renames. Ctrl+Z undoes a fresh thread action even in a text field. After typing another key, text undo takes precedence. On macOS the primary shortcuts use Command. Shift+Enter inserts a newline.

Ctrl+K opens thread search. Ctrl+N starts a thread in the current project, while the New thread button offers a project picker. The logo, search and sidebar toggle share the title strip with the tabs. Collapsing the sidebar keeps its toggle at the top-left of that strip. A captain's portrait reflects the most urgent live thread in its project's crew.

## Shina MCP

Agents running in Shina know they are in Shina and can use its tools, the way harnesses in T3 Code use T3's. Shina hosts a local [MCP](https://modelcontextprotocol.io) server at `http://127.0.0.1:<random port>/mcp`, using Streamable HTTP with JSON responses. Each connected thread gets its own random bearer token. The token tells Shina which thread is calling, and Shina keeps it only in memory.

| Tool | What it does |
|---|---|
| `list_threads` | Lists the calling captain's home and crew, or lone threads in the caller's project. `all_projects: true` includes every conversation. |
| `read_thread` | Returns a thread's most recent messages. |
| `list_captains` | Lists the crew with their roles and current status. |
| `orchestrator_capabilities` | Reports exact CPA model IDs, supported reasoning levels, inherited and default selections, discovery status, and Pi/local-machine limits. |
| `delegate_task` | Starts a child in the caller's project and machine, under the calling captain if there is one, with a complete brief and no parent history. Accepts a model and reasoning `target`, or inherits the caller's selection when omitted. Children cannot delegate. A named captain must match the calling coordinator, not a project-wide assignment. Accepts `wait_seconds`. |
| `task_status` | Reports a thread's project, machine, parent, status, completion flag, latest reply and error. Accepts `wait_seconds`. |
| `task_cancel` | Clears queued work and aborts a child. Only the parent that delegated it may cancel. |
| `task_send` | Sends a correction or follow-up brief to a child. Only its parent may send. Answers a pending input/editor/select question, starts an idle child or queues after its current work. It never approves a confirmation. |

After delegating, a captain can end its turn. When a child settles, fails or is cancelled, Shina sends its parent one completion message with the child's id, title, final status, last reply and any error. Replies are capped at 6,000 characters and errors at 2,000. Results start a new parent turn if idle, queue after current work if busy, or reconnect a disconnected parent before delivery. Multiple results retain separate queue entries. Messages appear as Shina in the transcript, including after restoring Pi history, and never replace the user's composer draft.

Call `orchestrator_capabilities` before selecting a delegation model. Pass an exact advertised ID and reasoning level, for example `target: {"provider":"cpa","model":"<advertised ID>","options":{"reasoning":"max"}}` when that model supports `max`. `thinking` is an alias for `reasoning`. Shina rejects unknown models, unsupported levels and unknown target settings before creating a child. It applies the selection and checks Pi's actual state before sending the brief. If Pi substitutes a model or clamps the level, the child fails without receiving the task. Omitting `target` inherits the caller's pinned or current selection, or the catalog default if the caller has no selection. Thread reads and task status include `selection`.

`wait_seconds` is an integer from 0 to 600, default 0. A nonzero value holds the tool reply until completion or the deadline, then returns the current task status. Shina parks these replies without blocking its UI. Completion wake-ups still arrive when a tool waits, so captains should not poll in a loop.

Tool calls run on Shina's UI thread, so they see and change exactly what the window shows. Delegated threads retain their parent in the saved index. Captain crew appears in its captain's view, never in the lone-thread list. Pending injections and completion tracking are in memory, not durable across app restarts. If reconnecting or sending fails, Shina retains the injection for a manual reconnect rather than retrying continuously.

Pi reaches the server through a small bridge extension, [`assets/pi/shina-mcp-extension.ts`](assets/pi/shina-mcp-extension.ts). Shina writes it to `%LOCALAPPDATA%\Shina\pi` and loads it with `--extension`, passing the endpoint and token in `SHINA_MCP_URL` and `SHINA_MCP_TOKEN`. The extension registers each tool as `mcp__shina__<tool>` and adds a "Shina" section to the system prompt. Other harnesses can connect to the same endpoint with a bearer header. Only Pi needs the bridge.

## Safety and storage

Pi tools can edit files and execute commands with your account's permissions. The composer permissions menu offers Supervised, Auto-accept edits and Full access. Existing threads retain Full access until you change it. Supervised asks before commands and file changes; Auto-accept edits permits built-in edits and asks before other actions. Mode changes restart an idle thread's process, and forks and delegated children inherit the mode. These are tool approvals, not an OS sandbox or worktree isolation. Trusted extensions run with your account's permissions and are not confined by the tool hook.

Real conversations now load Pi's configured extensions, MCP servers, skills and prompt templates, subject to Pi's normal project trust flow. Shina also explicitly loads its bridge and the existing CPA provider extension when present. Catalog probes remain offline, ephemeral and resource-restricted. Hosts without a CPA extension can use their existing catalog-generated CPA provider in Pi's `models.json`. Pi's enrollment supplies proxy routes, transport metadata, reasoning capabilities and credentials. No model list or credential is copied into this repository. The adapter does not rewrite provider configuration, project trust or authentication. See [Pi integration](docs/pi-integration.md) for behavior, validation and remaining differences from KSG Code.

Pi must already be enrolled in CPA through its provider extension or a configured CPA provider with models in `~/.pi/agent/models.json`. `PI_CODING_AGENT_DIR` can select a different Pi agent directory. Shina reports missing CPA enrollment instead of silently using a direct provider. The workstation's existing `CPA_API_KEY` and local proxy remain outside Shina. Pi inherits Shina's environment, so the key must be present when Shina starts; a key set only in another shell will not reach an already-running app. Opening a picker refreshes catalog metadata after five minutes. Running processes read changed provider definitions when Shina next starts them, or after an app restart.

### Connection errors after moving CPA

Shina's localhost MCP endpoint controls threads. It is separate from Pi's CPA inference connection. An assistant message such as "Connection error." comes from Pi's provider request, not proof that the Shina MCP failed.

Check the CPA provider extension and the local proxy or tunnel on the machine running Shina. A provider URL such as `http://127.0.0.1:8317` refers to that machine. After moving CPA from `serverseesghosts` to the Mac Studio, a client that uses an SSH tunnel must forward to `kidsseemac` instead of the old server. The Mac Studio can use its local CPA listener directly. Catalog discovery URLs and inference URLs are separate; a working model picker does not prove inference works.

Keep endpoint and tunnel configuration in the existing CPA/Pi configuration, outside this repository. Shina does not rewrite authentication or replace the provider extension. After updating the host-owned configuration, restart Shina so new Pi processes load it. A missing Pi extension, a Pi process exit, and a provider connection error have different causes; preserve the full error when reporting a failure.

On Windows, Shina stores its version 4 global index in `%LOCALAPPDATA%\Shina\index.json`. First launch checks the per-workspace indexes for every path in legacy `projects.json` and `last-workspace.txt`, plus Still-era indexes. Missing folders migrate only when their index contains threads. Missing folders under the system temp directory are ignored. Verbatim Windows path prefixes are removed from project paths, and equivalent paths share one project. Colliding thread ids and parent links are remapped together. Sessions, pins, archived state, delegation and captain profiles survive. Every preserved captain-home flag restores an independent coordinator, including the Pip, Sable and Ember homes v0.8 hid. Project captain fields now name a default, not the only allowed captain. Shina never deletes or rewrites legacy indexes. A damaged index or lifecycle record blocks saving instead of overwriting it with a partial import.

Version 3 pin, archive and unread booleans migrate to timestamps. A synthetic completion at the saved activity time preserves a legacy unread flag. New unread state derives from `lastCompletedAt > lastVisitedAt`. Lifecycle timestamps use Unix milliseconds inside each record's `lifecycle` object. Runtime `Completed` means a Pi run finished, not that the thread is settled.

Shelf expansion, multi-project scope, sidebar visibility and width, and drafts live in a separate `ui-state.json`. Draft images use small references to deduplicated files in Shina's local `attachments` folder, not inline base64. Display thumbnails are at most 512 pixels per side; Pi receives the original image bytes. Retry, startup cancellation and discard undo preserve the images. Shina reports missing saved draft images instead of silently treating the draft as complete. Image blobs and thumbnails currently remain on disk after removal. Context usage is not persisted and refreshes from Pi on reconnect and provider streaming usage. The details include Pi's session token total and reported cost. Queued user messages and image references survive in UI state. After a restart, Shina checks authoritative history and recovers undelivered messages into the draft rather than automatically replaying them. Unsent thread metadata still uses the global index so existing thread IDs and tabs remain stable. A malformed UI-state file also blocks saving.

Migrated threads use their saved activity time, their Pi session file's modification time, or the legacy index's modification time, in that order. The sidebar hides unknown times. The Captains section shows assigned coordinators only. Its single assignment control lets you choose an unassigned project.

Each thread starts Pi in its own project's folder. A project-less thread uses `%LOCALAPPDATA%\Shina\chats\<thread-id>`, isolated from other chats and reused on resume. Projects and threads record a machine id. The backend dispatch currently supports only `local`; unknown machines fail instead of running locally. Project notes/context and a remote `shina serve` host are follow-up work.

Pi keeps its session files and credentials in its normal locations outside this repository. Drafts survive thread and project switches and app restarts. Run only one Shina instance at a time to avoid competing global-index writes.

Closing Shina terminates its owned agent and clone process trees. Windows assigns each subprocess to a kill-on-close Job Object before allowing it to run; Unix uses process groups. Pipe writes run separately from process supervision, and shutdown waits have deadlines. Reconnecting to a saved session waits for its previous Pi owner to exit. A stop that gets no settlement forces a disconnect after ten seconds; model and thinking commands time out after thirty seconds.

Failed index writes retry while idle and on flush, even if the state has not changed. Flush waits at most five seconds per store. A persistent disk failure can still prevent saving. File replacement and directory flushing are best effort against power loss, especially on Windows filesystems that do not support directory flushes.

History images import on the RPC reader rather than the UI thread. Validated blobs and previews are reused. The transcript draws at most the latest 32 image previews and labels older omitted previews. Large project icons use cached 64-pixel thumbnails; oversized or invalid icon sources use the project initial. Thread diagnostics retain at most 128 entries, each limited to about 2 KiB.

The loopback MCP transport accepts fixed-length and chunked POST bodies and `Expect: 100-continue`. It limits bodies to 4 MiB, headers to 32 KiB, individual header lines to 8 KiB, concurrent connections and queued calls to 32, and UI request processing to 16 calls per frame. Header reads have a five-second absolute deadline.

## Validation

```powershell
cargo test
cargo clippy --all-targets -- -D warnings
cargo test installed_pi_answers_state_and_history_without_a_prompt -- --ignored
cargo test installed_pi_uses_captain_system_instructions -- --ignored
cargo test installed_pi_calls_shina_mcp_tools -- --ignored
cargo test installed_pi_edits_a_file_and_restores_the_session -- --ignored
cargo test installed_pi_parent_is_woken_by_child_completion -- --ignored --nocapture
cargo test installed_pi_accepts_images_and_restores_them_from_history -- --ignored --nocapture
cargo test installed_pi_runs_projects_and_scratch_concurrently -- --ignored --nocapture
cargo test real_clone_of_a_public_repository -- --ignored
```

The first opt-in integration test verifies CPA model discovery, reasoning levels, state, and history without a model prompt. The captain and MCP tests send real CPA prompts through automatic startup. The file-edit test edits a disposable fixture and resumes its session to verify saved messages and tool results. The crew test uses GPT 6.1 Sol to discover capabilities, delegate a PONG reply to DeepSeek V4.1 Flash at max reasoning, end its turn and receive a completion wake-up. The vision test sends a color fixture to GPT 6.1 Sol, resumes its saved image history and checks current context usage before and after reconnect. These two tests select their models in memory, without changing Pi's defaults. The prompt-based tests consume upstream quota. Test Pi processes receive `--session-dir` pointing inside the owned temporary test profile, not Pi's normal session directory.

The multi-project live test runs two projects and a project-less chat concurrently. Each reads a different relative-path fixture to verify its working directory. The clone test clones two small public GitHub repositories, one with `gh` and one with `git`, into a temporary folder and deletes them afterwards.

Test builds inject an owned temporary Shina data directory per test thread. Default state storage, scratch chats and MCP bridge files cannot resolve to the user's Shina profile. Legacy recents/index tests also use explicit temporary base directories. Tests do not mutate process-wide environment variables to redirect parallel tests.

Render with Rune's actual GPU renderer, without opening a window:

```powershell
cargo run -- --demo --snap artifacts\shina-light.png --size 1280x840 --scale 1
cargo run -- --demo --scene dark --snap artifacts\shina-dark.png --size 1280x840 --scale 1
cargo run -- --demo --snap artifacts\shina-narrow.png --size 620x720 --scale 1
cargo run -- --demo --scene captain --snap artifacts\shina-captain.png --size 1280x840 --scale 1
cargo run -- --demo --scene captain-workspace --snap artifacts\shina-captain-workspace.png --size 1280x840 --scale 1
cargo run -- --demo --scene captain-workspace-dark --snap artifacts\shina-captain-workspace-dark.png --size 1280x840 --scale 1
cargo run -- --demo --scene captain-search --snap artifacts\shina-captain-search.png --size 1280x840 --scale 1
cargo run -- --demo --scene captain-crews-dark --snap artifacts\shina-captain-crews.png --size 1280x840 --scale 1
cargo run -- --demo --scene picker --snap artifacts\shina-picker.png --size 1280x840 --scale 1
cargo run -- --demo --scene projects --snap artifacts\shina-projects.png --size 1280x840 --scale 1
cargo run -- --demo --scene clone --snap artifacts\shina-clone.png --size 1280x840 --scale 1
cargo run -- --demo --scene sidebar-narrow --snap artifacts\shina-sidebar-narrow.png --size 620x720 --scale 1
cargo run -- --demo --scene collapsed --snap artifacts\shina-collapsed.png --size 1280x840 --scale 1
cargo run -- --demo --scene empty --snap artifacts\shina-empty.png --size 1280x840 --scale 1
cargo run -- --demo --scene empty-dark --snap artifacts\shina-empty-dark.png --size 1280x840 --scale 1
```

The [workspace design notes](docs/workspace-design.md) record the Claude Desktop reference, font sources, and this pass's design choices.

## Implementation and limits

`src/ui.rs` draws the interface. `src/model.rs` reduces Pi events and owns global project/thread state and automatic session lifetimes. `src/session.rs` caches provider metadata independently of thread runtimes. `src/backend.rs` dispatches machine hosts and handles local subprocesses and JSONL framing on worker threads. `src/store.rs` migrates legacy data and saves the global index off the render thread. `src/projects.rs` holds project records, project actions and background clones.

Rune is pre-release and pinned to revision `a7f276c3630f85a06a0230295da4d2bd172d8d3b`. The app batches incoming records, bounds pipe queues, memoizes rendered Markdown, and uses Rune's event-driven redraw behavior. These are design choices, not proof of production performance. Each active thread has the memory cost of its Pi process until idle reclaim.

The earlier layout's release build reported forced at-rest CPU times below 0.15 ms at p95 for a small static example. That measurement predates the crew portrait pass and is not a benchmark of this revised layout, interactive frame rate, or live Pi work.

The full T3 thread-management port is on hold for a separate spec. Snooze and auto-wake, settle/unsettle rules, collapsible and paged settled shelves, drag between sections and undo notices are not implemented in this release. `Status::Settled` still means the Pi run finished, not that the user settled the thread. Future thread-list lifecycle fields must remain separate from process status.

This is an initial desktop prototype. Login UI, git diffs, worktrees and remote hosts remain unfinished. Crash recovery resumes persisted Pi history, but cannot recover unsaved provider output or guarantee that retrying a tool action is safe. The transcript currently displays the latest 100 messages and the activity panel displays the latest 50 calls. Older transcript messages remain in Pi's session. Pi RPC records have a 64 MiB limit, so exceptionally image-heavy saved history can require inspection in Pi or a new Shina thread.

The pixel S and three colored dots use the same drawing code in [`src/brand.rs`](src/brand.rs) for the in-app mark and native window/Windows executable icons. Checked-in PNG, ICO favicon, and Mac iconset exports retain sharp pixels at every size. The old Pikachu project icon is no longer used. Regenerate exports after changing the mark:

```sh
cargo run --example export-icons -- assets
```

`assets/project-icon.png` is the high-resolution app mark. `assets/favicon.png` and `assets/favicon.ico` are the project/browser favicons. The macOS installer packages `assets/macos/Shina.iconset` into the app bundle.

## Crew artwork

The four captains are original characters drawn as 40x40 pixel art by [`tools/crew_originals.py`](tools/crew_originals.py), framed like a Pokémon Mystery Dungeon bust and presented on the square crew tiles Capy uses. See [the artwork notes](assets/crew/README.md).

Captains are the sidebar's first section. Each captain row shows a 36 px portrait, or 44 px for the open captain (otherwise the most urgent one), its name, and one status line such as "Working · 2 threads" or "Needs you · approval". The portrait slot, badge slot and timer slot have fixed sizes, so a status change never moves the row or its neighbors.

Sidebar portraits are animated from short sprite strips: Ready breathes and blinks, Working types and nods, Needs you hops once every six seconds, and Done hops once. Failed and every portrait outside the sidebar stay static. The animation does not re-run the view or lay anything out. [`src/portraits.rs`](src/portraits.rs) swaps frames in a shared `rune::Pixels` per portrait and clip, and the app's poll asks Rune to wake it only when the next frame is due. All rows that share a portrait and clip use one swap. One idle captain costs about one repaint a second, and a working captain about four. Frames play only for rows on screen while the window has focus, and not under the system's reduced-motion setting. A minimized or background window, a collapsed sidebar or a captain-free sidebar schedules no frames at all.

### Drop-in portrait sets

Shina can load a different set without a rebuild. Point `SHINA_SPRITES` at a folder containing six PNGs per captain, or put the folder at `<workspace>/assets/crew`:

```text
assets/crew/ember/Normal.png
assets/crew/ember/Happy.png
assets/crew/ember/Determined.png
assets/crew/ember/Surprised.png
assets/crew/ember/Dizzy.png
assets/crew/ember/Sad.png
...same six for juniper, sable, and pip
```

A numeric folder (`assets/crew/0/Normal.png`) also works. `SHINA_SPRITES` wins over the workspace folder. All six faces must exist for a captain to use the override; otherwise that captain keeps the embedded artwork. The loader accepts any square PNG size and the app scales it into tiles from 20 px to 48 px.

A drop-in set with only the six faces keeps static portraits. To animate it, add all four strips beside the faces: `Idle.strip.png` (4 frames), `Working.strip.png` (4), `Attention.strip.png` (4) and `Happy.strip.png` (3). A strip is square frames side by side, and frame 0 should match the static face. The frame order is described in [the artwork notes](assets/crew/README.md). A missing strip or a strip with the wrong frame count leaves that captain static.
