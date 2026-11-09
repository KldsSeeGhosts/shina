# Shina

A native desktop chat workspace built with [Rune](https://github.com/anomalyco/rune) and the [Pi coding agent](https://github.com/earendil-works/pi). Shina is a working-name prototype, not a T3 Code or Capy client.

The interface is entirely Rune. Pi runs as a hidden background RPC process. There is no terminal emulator, webview, browser server, or Electron runtime in this app.

## Run on Windows

The development tools and Pi are already installed on this workstation.

```powershell
cargo run
```

Click the project name at the top of the thread list to open the project switcher. It lists your recent projects and has **Open folder...** and **Clone repository...**. Choose a project, then send a message. Your message appears immediately and the thread shows Working until Pi responds. Shina prepares Pi in the background when you start a draft, or on send if needed. There is no connection step. Sending a message starts agent work and can consume your CPA upstream quota.

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

Install Shina for your Windows account:

```powershell
cargo build --release
.\target\release\shina.exe --install
```

This copies the build to `%LOCALAPPDATA%\Programs\Shina\shina.exe`, adds a Start Menu shortcut, registers an Apps & Features entry, and starts the installed copy. The path never changes, so a Start pin or the Caelestia launcher keeps working across updates and development rebuilds. The installed app reopens the last workspace you used. Run `shina.exe --uninstall`, or remove Shina in Settings > Apps, to uninstall it. Your thread indexes in `%LOCALAPPDATA%\Shina` are kept.

The installed app checks GitHub Releases on startup through the GitHub CLI, using your existing `gh` login, because the repository is private. When a newer version exists, the sidebar shows **Update to vX.Y.Z**. After the download it offers **Restart to install**, which swaps the executable and reopens your workspace. Development builds never update themselves.

To publish a release, bump `version` in `Cargo.toml` in a PR, merge it, then run this from a clean `main`:

```powershell
.\tools\release.ps1 -Install
```

An optional development flag selects a project on launch:

```powershell
cargo run -- --workspace C:\path\to\project
```

If automatic Pi discovery fails, set `SHINA_PI_CLI` to the full path of Pi's CLI JavaScript file or a native executable. Earlier setups may still use `STILL_PI_CLI`; Shina reads it as a fallback. On Windows the adapter resolves the installed npm entry directly and launches it with Node. It does not use a command shell or copy credentials.

## What works

- Native chat with streamed text, selectable Markdown, syntax-highlighted code blocks, and a bottom composer. Sending immediately adds the submitted message without waiting for process startup.
- CPA catalog model selection and model-specific reasoning levels through Pi RPC. Pi's existing CPA default is used for new sessions.
- A native sidebar and chat reconstruction with variable Inter typography, light/dark palettes, compact tabs, search, and an optional overview pane.
- Four crew captains, each an original 40x40 pixel-art portrait that reacts to the crew's live state: Normal when ready, Determined while working, Surprised when a thread needs you, Dizzy after a failure, and Happy when settled. The set is Ember, Juniper, Sable, and Pip.
- Captain conversation groups with nested threads, a native Captain picker that shows each captain's live status, and thread pinning.
- Independent Pi processes for active threads. Switching threads preserves their drafts and leaves other work running. Shina releases processes after 30 idle minutes and resumes their saved session on the next send.
- Running, needs-input, and failed states. `agent_end` does not finish a run; the adapter waits for `agent_settled`. A finished thread goes quiet instead of showing a status, and a stopped run returns to idle. Background threads that finish get an unread badge.
- Canceling a run clears the queue before requesting an abort.
- Native confirmation, selection, and text replies for Pi RPC dialog records. Nothing auto-approves a confirmation.
- An atomic per-workspace index remembers thread titles, archive state, model/thinking choices, and Pi session paths. Opening a saved thread restores its transcript, tool summaries, and tool results automatically.
- Process crashes show an inline error with Retry. Shina never automatically replays a prompt that might already have run tools. Retrying an accepted run can repeat tool actions, so check any partial result first.
- Live progress distinguishes model thinking, text generation, tool execution, retries, and compaction. Rejected prompts return to the draft; cancellation releases a pending dialog before clearing the queue and aborting.
- A project switcher with recent projects, native folder selection, and cloning. Switching folders refuses to discard unsent drafts or interrupt busy threads, and names the thread that blocks it.
- Recent projects are saved in `%LOCALAPPDATA%\Shina\projects.json`, which replaces `last-workspace.txt` (read once to seed the list). Projects whose folder is missing are hidden from the menu but kept in the file.
- **Clone repository...** accepts `owner/repo`, a GitHub URL, or any git URL. Shorthand runs `gh repo clone`; URLs run `git clone`. Both run in the background without a console window, with prompts turned off, and use your existing git or `gh` login. Shina never asks for or stores credentials. The destination defaults to the current project's parent folder and remembers the last one used. Shina refuses a destination that exists and is not empty, then opens the new clone when it finishes.

Keyboard shortcuts are Ctrl+N for a new thread, Ctrl+O to open a folder, Ctrl+B for the sidebar, Ctrl+L for the composer, Ctrl+= and Ctrl+- to zoom the whole window (Ctrl+0 resets it), and Escape to stop a run. On macOS the primary shortcuts use Command. Shift+Enter inserts a newline.

Ctrl+K opens thread search. Captain entries currently organize separate Pi conversations. A captain's portrait reflects the most urgent live thread in its crew, so a failure outranks a request for help, which outranks working and idle states. Issues, pull requests, and automation navigation remain disabled until those integrations exist.

## Shina MCP

Agents running in Shina know they are in Shina and can use its tools, the way harnesses in T3 Code use T3's. Shina hosts a local [MCP](https://modelcontextprotocol.io) server at `http://127.0.0.1:<random port>/mcp`, using Streamable HTTP with JSON responses. Each connected thread gets its own random bearer token. The token tells Shina which thread is calling, and Shina keeps it only in memory.

| Tool | What it does |
|---|---|
| `list_threads` | Lists the workspace's threads with their ids, titles, parents, captains, and status. |
| `read_thread` | Returns a thread's most recent messages. |
| `list_captains` | Lists the crew with their roles and current status. |
| `delegate_task` | Starts a child under the caller's captain or a named one with a complete brief and no parent history. Children cannot delegate. Accepts `wait_seconds`. |
| `task_status` | Reports a thread's parent, status, completion flag, latest reply and error. Accepts `wait_seconds`. |
| `task_cancel` | Clears queued work and aborts a child. Only the parent that delegated it may cancel. |
| `task_send` | Sends a correction or follow-up brief to a child. Only its parent may send. Starts an idle child or queues after its current work. |

After delegating, a captain can end its turn. When a child settles, fails or is cancelled, Shina sends its parent one completion message with the child's id, title, final status, last reply and any error. Replies are capped at 6,000 characters and errors at 2,000. Results start a new parent turn if idle, queue after current work if busy, or reconnect a disconnected parent before delivery. Multiple results retain separate queue entries. Messages appear as Shina in the transcript, including after restoring Pi history, and never replace the user's composer draft.

`wait_seconds` is an integer from 0 to 600, default 0. A nonzero value holds the tool reply until completion or the deadline, then returns the current task status. Shina parks these replies without blocking its UI. Completion wake-ups still arrive when a tool waits, so captains should not poll in a loop.

Tool calls run on Shina's UI thread, so they see and change exactly what the window shows. Delegated threads retain their parent in the saved index and appear nested beneath it with an unread badge. Pending injections and completion tracking are in memory, not durable across app restarts. If reconnecting or sending fails, Shina retains the injection for a manual reconnect rather than retrying continuously.

Pi reaches the server through a small bridge extension, [`assets/pi/shina-mcp-extension.ts`](assets/pi/shina-mcp-extension.ts). Shina writes it to `%LOCALAPPDATA%\Shina\pi` and loads it with `--extension`, passing the endpoint and token in `SHINA_MCP_URL` and `SHINA_MCP_TOKEN`. The extension registers each tool as `mcp__shina__<tool>` and adds a "Shina" section to the system prompt. Other harnesses can connect to the same endpoint with a bearer header. Only Pi needs the bridge.

## Safety and storage

Pi tools can edit files and execute commands with your account's permissions. This prototype has no sandbox, worktree isolation, or built-in tool approval policy. Do not point it at an untrusted project.

Pi launches with automatic extension discovery, MCP, skills, and prompt templates disabled. Shina explicitly loads only your existing `extensions/cpa-provider.ts` from Pi's agent directory. That extension reads your existing CPA catalog enrollment and credentials and supplies the proxy routes, transport metadata, and reasoning capabilities. No model list or credential is copied into this repository. **Do not use this version if you depend on another Pi extension for safety checks or tool approvals.** The adapter does not rewrite provider configuration or authentication.

Pi must already be enrolled in CPA with `~/.pi/agent/cpa-catalog.json`, its catalog client, and `extensions/cpa-provider.ts`. `PI_CODING_AGENT_DIR` can select a different Pi agent directory. Shina reports a missing provider extension instead of silently using a direct provider. The workstation's existing `CPA_API_KEY` and local proxy remain outside Shina. Pi inherits Shina's environment, so the key must be present when Shina starts; a key set only in another shell will not reach an already-running app. Opening a picker refreshes catalog metadata after five minutes. Running processes read changed provider definitions when Shina next starts them, or after an app restart.

On Windows, Shina's small workspace index lives under `%LOCALAPPDATA%\Shina`. A Still-era index is copied over on first launch, so renaming the app does not lose saved threads. Pi keeps its own session files and credentials in its normal locations outside this repository. Drafts currently survive thread switches, but not app restarts. Run one Shina instance per workspace to avoid competing index writes.

Closing Shina closes the agent input pipes and waits for shutdown. After a short grace period, it terminates any remaining agent process. This is cleanup, not process-tree isolation.

## Validation

```powershell
cargo test
cargo clippy --all-targets -- -D warnings
cargo test installed_pi_answers_state_and_history_without_a_prompt -- --ignored
cargo test installed_pi_uses_captain_system_instructions -- --ignored
cargo test installed_pi_calls_shina_mcp_tools -- --ignored
cargo test installed_pi_edits_a_file_and_restores_the_session -- --ignored
cargo test installed_pi_parent_is_woken_by_child_completion -- --ignored --nocapture
cargo test real_clone_of_a_public_repository -- --ignored
```

The first opt-in integration test verifies CPA model discovery, reasoning levels, state, and history without a model prompt. The captain and MCP tests send real CPA prompts through automatic startup. The file-edit test edits a disposable fixture and resumes its session to verify saved messages and tool results. The crew test runs a real parent that delegates a PONG reply, ends its turn, and receives a completion wake-up. The prompt-based tests consume upstream quota and leave Pi's test sessions in its normal session directory.

The clone test clones two small public GitHub repositories, one with `gh` and one with `git`, into a temporary folder and deletes them afterwards.

Render with Rune's actual GPU renderer, without opening a window:

```powershell
cargo run -- --demo --snap artifacts\shina-light.png --size 1280x840 --scale 1
cargo run -- --demo --scene dark --snap artifacts\shina-dark.png --size 1280x840 --scale 1
cargo run -- --demo --snap artifacts\shina-narrow.png --size 620x720 --scale 1
cargo run -- --demo --scene captain --snap artifacts\shina-captain.png --size 1280x840 --scale 1
cargo run -- --demo --scene picker --snap artifacts\shina-picker.png --size 1280x840 --scale 1
cargo run -- --demo --scene projects --snap artifacts\shina-projects.png --size 1280x840 --scale 1
cargo run -- --demo --scene clone --snap artifacts\shina-clone.png --size 1280x840 --scale 1
```

## Implementation and limits

`src/ui.rs` draws the interface. `src/model.rs` reduces Pi events and owns thread state and automatic session lifetimes. `src/session.rs` caches provider metadata independently of thread runtimes. `src/backend.rs` handles subprocesses and JSONL framing on worker threads. `src/store.rs` saves the workspace index off the render thread. `src/projects.rs` keeps the recent projects list and runs clones.

Rune is pre-release and pinned to revision `a7f276c3630f85a06a0230295da4d2bd172d8d3b`. The app batches incoming records, bounds pipe queues, memoizes rendered Markdown, and uses Rune's event-driven redraw behavior. These are design choices, not proof of production performance. Each active thread has the memory cost of its Pi process until idle reclaim.

The earlier layout's release build reported forced at-rest CPU times below 0.15 ms at p95 for a small static example. That measurement predates the crew portrait pass and is not a benchmark of this revised layout, interactive frame rate, or live Pi work.

This is an initial desktop prototype. Login UI, attachments, git diffs, worktrees, multi-project concurrent work, and packaging/installers remain unfinished. Crash recovery resumes persisted Pi history, but cannot recover unsaved provider output or guarantee that retrying a tool action is safe. The transcript currently displays the latest 100 messages and the activity panel displays the latest 50 calls. Older transcript messages remain in Pi's session.

The supplied `assets/project-icon.png` is the T3 Code project favicon and the source for Shina's executable, taskbar, and window icons. The build generates Windows icon sizes without smoothing the pixel art. This user-supplied artwork is separate from the code's MIT license.

## Crew artwork

The four captains are original characters drawn as 40x40 pixel art by [`tools/crew_originals.py`](tools/crew_originals.py), framed like a Pokémon Mystery Dungeon bust and presented on the square crew tiles Capy uses. See [the artwork notes](assets/crew/README.md).

Each portrait is a static image selected by thread state. There is no frame timer and no extra redraw loop; the portrait changes when the crew's state changes.

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

A numeric folder (`assets/crew/0/Normal.png`) also works. `SHINA_SPRITES` wins over the workspace folder. All six faces must exist for a captain to use the override; otherwise that captain keeps the embedded artwork. The loader accepts any square PNG size and the app scales it into the 24 px and 40 px tiles.
