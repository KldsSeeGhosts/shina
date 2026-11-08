# Still

A native desktop chat workspace built with [Rune](https://github.com/anomalyco/rune) and the [Pi coding agent](https://github.com/earendil-works/pi). Still is a working-name prototype, not a T3 Code or Capy client.

The interface is entirely Rune. Pi runs as a hidden background RPC process. There is no terminal emulator, webview, browser server, or Electron runtime in this app.

## Run on Windows

The development tools and Pi are already installed on this workstation.

```powershell
cargo run
```

Click **Open folder** to choose a project, then **Connect Pi**. Connect only reads agent state and history. Sending a message starts agent work and can incur provider charges.

For an example conversation without an agent connection:

```powershell
cargo run -- --demo
```

For a native executable with no console window:

```powershell
cargo build --release
.\target\release\still.exe
```

An optional development flag selects a project on launch:

```powershell
cargo run -- --workspace C:\path\to\project
```

If automatic Pi discovery fails, set `STILL_PI_CLI` to the full path of Pi's CLI JavaScript file or a native executable. On Windows the adapter resolves the installed npm entry directly and launches it with Node. It does not use a command shell or copy credentials.

## What works

- Native chat with streamed text, selectable Markdown, syntax-highlighted code blocks, and a bottom composer.
- A native sidebar and chat reconstruction with variable Inter typography, light/dark palettes, compact tabs, search, and an optional overview pane.
- Three crew captains, each a Pokémon Mystery Dungeon portrait that reacts to the crew's live state: Normal when ready, Determined while working, Surprised when a thread needs you, Dizzy after a failure, Happy when settled, and Sad when offline.
- Captain conversation groups with nested threads, a native Captain picker that shows each captain's live status, and thread pinning.
- Independent Pi processes for connected threads. Switching threads preserves their drafts and leaves other work running.
- Running, needs-input, failed, and settled states. `agent_end` does not settle a thread. The adapter waits for `agent_settled`.
- Canceling a run clears the queue before requesting an abort.
- Native confirmation, selection, and text replies for Pi RPC dialog records. Nothing auto-approves a confirmation.
- An atomic per-workspace index remembers thread titles, archive state, and Pi session paths. Reconnect to load a saved transcript.
- Native folder selection. Switching folders refuses to discard unsent drafts or interrupt busy threads.

Keyboard shortcuts are Ctrl+N for a new thread, Ctrl+B for the sidebar, Ctrl+L for the composer, and Escape to stop a run. On macOS the primary shortcuts use Command. Shift+Enter inserts a newline.

Ctrl+K opens thread search. Captain entries currently organize separate Pi conversations. A captain's portrait reflects the most urgent live thread in its crew, so a failure outranks a request for help, which outranks working, settled, and offline states. Autonomous crew delegation is not connected. Issues, pull requests, and automation navigation remain disabled until those integrations exist.

## Safety and storage

Pi tools can edit files and execute commands with your account's permissions. This prototype has no sandbox, worktree isolation, or built-in tool approval policy. Do not point it at an untrusted project.

For this first adapter, Pi launches with extensions, MCP, skills, and prompt-template discovery disabled. This prevents startup integrations from running before the client supports their full behavior. **Do not use this version if you depend on a Pi extension for safety checks or tool approvals.** The adapter does not modify your Pi configuration or authentication. It uses Pi's configured default model.

On Windows, Still's small workspace index lives under `%LOCALAPPDATA%\Still`. Pi keeps its own session files and credentials in its normal locations outside this repository. Drafts currently survive thread switches, but not app restarts. Run one Still instance per workspace to avoid competing index writes.

Closing Still closes the agent input pipes and waits for shutdown. After a short grace period, it terminates any remaining agent process. This is cleanup, not process-tree isolation.

## Validation

```powershell
cargo test
cargo clippy --all-targets -- -D warnings
cargo test installed_pi_answers_state_and_history_without_a_prompt -- --ignored
```

The opt-in integration test starts the installed Pi and requests state and history. It sends no model prompt and performs no coding-agent tool calls.

Render with Rune's actual GPU renderer, without opening a window:

```powershell
cargo run -- --demo --snap artifacts\still-light.png --size 1280x840 --scale 1
cargo run -- --demo --scene dark --snap artifacts\still-dark.png --size 1280x840 --scale 1
cargo run -- --demo --snap artifacts\still-narrow.png --size 620x720 --scale 1
cargo run -- --demo --scene captain --snap artifacts\still-captain.png --size 1280x840 --scale 1
cargo run -- --demo --scene picker --snap artifacts\still-picker.png --size 1280x840 --scale 1
```

## Implementation and limits

`src/ui.rs` draws the interface. `src/model.rs` reduces Pi events and owns thread state. `src/backend.rs` handles subprocesses and JSONL framing on worker threads. `src/store.rs` saves the workspace index off the render thread.

Rune is pre-release and pinned to revision `a7f276c3630f85a06a0230295da4d2bd172d8d3b`. The app batches incoming records, bounds pipe queues, memoizes rendered Markdown, and uses Rune's event-driven redraw behavior. These are design choices, not proof of production performance. Each connected thread still has the memory cost of its Pi process.

The earlier layout's release build reported forced at-rest CPU times below 0.15 ms at p95 for a small static example. That measurement predates the crew portrait pass and is not a benchmark of this revised layout, interactive frame rate, or live Pi work.

This is an initial desktop prototype. Model selection and login UI, attachments, restored tool histories, git diffs, worktrees, multi-project concurrent work, crash recovery, and packaging/installers remain unfinished. The transcript currently displays the latest 100 messages and the activity panel displays the latest 50 calls. Older transcript messages remain in Pi's session.

## Crew artwork

The three captain portraits are Pokémon Mystery Dungeon artwork from [PMDCollab SpriteCollab](https://github.com/PMDCollab/SpriteCollab): Bulbasaur, Charmander, and Squirtle. They are licensed CC BY-NC 4.0, with the base portraits credited to CHUNSOFT. The files are not covered by this repository's [MIT license](LICENSE). They are included for a private, non-commercial experiment; see [the artwork provenance](assets/sprites/README.md) before redistributing or publishing a build. Pokémon and the related character names are trademarks of Nintendo, Creatures, and GAME FREAK. This project is not affiliated with or endorsed by them.

Each portrait is a static image selected by thread state. There is no frame timer and no extra redraw loop; the portrait changes when the crew's state changes.
