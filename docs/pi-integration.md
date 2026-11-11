# Pi integration

Shina controls one harness subprocess per active conversation. Pi is the default and owns model execution, compaction and its session files. Shina owns its threads, drafts, tabs, captain relationships and queued user turns. Pi inference remains CPA-only. Optional ACP harnesses use their own providers. See [thread harnesses](harnesses.md) for the registry, ACP protocol and limits.

## Implemented behavior

| Area | Behavior |
| --- | --- |
| Resources | Real sessions use Pi's normal extension, MCP, skill, template and project-trust discovery. Shina adds its CPA provider and thread-scoped bridge. Catalog probes remain isolated and offline. No provider configuration, credentials or trust records are edited by Shina. |
| Commands | Each process requests `get_commands` in its own workspace. The composer lists extension commands, templates and `/skill:name` commands with descriptions and source labels. Arrow keys select, Tab or Enter inserts, Escape dismisses. The thread menu can refresh discovery without discarding a previously usable inventory after an error. |
| Built-ins | `/model` opens Shina's model picker. `/compact [instructions]` maps to `compact`, not `prompt`. Unknown commands stay in the draft and produce an error rather than accidentally becoming a model request. Pi's terminal-only built-ins are not advertised as RPC commands. |
| Compaction | The context details expose Compact context. Pi receives optional `customInstructions`. Both manual and automatic compaction report progress, failure, cancellation, summary and token counts. Post-compaction token estimates are labelled approximate; the context ring stays unknown until Pi supplies fresh usage. Stop retires a process whose compaction cannot be interrupted by ordinary agent abort. |
| Steering | Enter or Send during a run submits `prompt` with `streamingBehavior: steer`, including typed image parts. A rejected steer returns to the draft without failing the original run. Prompts that race compaction wait for it to finish. |
| Queued turns | Ctrl/Command-click, Ctrl/Command+Enter, or the Queue dispatch mode stages a separate turn in Shina. Edit, Remove and Send now operate on stable local queue IDs. Only settlement dispatches the next queued turn. Stop returns queued input to the draft and clears Pi's native steering/follow-up queue before aborting. |
| Queue recovery | UI state saves queued text and image references. On reconnect, Shina compares pending inputs with authoritative history and recovers undelivered input into the draft. It does not replay potentially accepted work automatically. |
| Conversations | The thread menu can fork the current conversation, fork before a selected user message, or Edit from here. Whole forks use CLI `--fork` in a separate process. Selected-message forks and rewinds use native `fork` with a stable entry ID. Rewinding restores the selected prompt and images, changes no project files and keeps the original Pi session. |
| Session ownership | A fork never runs a second writer on the source session. Session changes wait for authoritative state and history before enabling sending. Late state responses from before the operation cannot rebind a thread to its old session. Lifecycle cancellation leaves the original conversation intact. |
| History | `get_entries` supplies the active branch by leaf ancestry, not append order. Shina retains pre-compaction transcript history while Pi independently builds compacted model context. Abandoned branches are not mixed into the visible conversation. Compaction summaries survive reconnect. |
| Names | Shina synchronizes thread titles through `set_session_name`. |
| Approvals | Supervised and Auto-accept edits use a blocking `tool_call` hook in Shina's bridge. The default remains Full access for existing profiles. Permission changes restart the idle runtime; forks and crew inherit the mode. A failed permission-bridge setup blocks supervised startup. |
| Extension UI | Confirm, select, input and editor requests support concurrent dialogs and expiration. Allow for session remembers an exact confirmation only for that process. Extension failures are visible in the conversation and diagnostics. Notifications and `set_editor_text` remain supported. |
| Results | Tool output retains up to 50,000 display characters, image references, parent call IDs, exit codes, and bounded structured result/details/nested-call metadata. Pi subagent extension results have individual status, model, task and output cards, with no invented Shina thread links. Pi retains the authoritative result. |
| Usage | The context meter accepts provider streaming usage and refreshes native session statistics after turns and compaction. Details show total processed tokens and Pi's reported session cost, separate from live context usage. |

UI sources include KSG Code's `ComposerCommandMenu`, `ComposerPrimaryActions`, `QueuedRunsControl`, `ContextWindowMeter`, `ComposerFooterModeControls`, and `ChatOverlayHosts`. Runtime behavior follows `packages/provider-pi/src/server/adapter.ts`, `commands.ts`, `status.ts`, and `mcpExtensionSource.ts`.

The selected-message picker is Pi-specific. KSG normally attaches its conversation actions to its own persisted run IDs; Shina uses Pi's native entry list and KSG's confirmation layout rather than guessing a boundary from repeated prompt text.

## Remaining differences from KSG Code

This closes the command, compaction, steering and native conversation-operation gaps. It is not complete application parity:

- Shina's attachment picker still accepts images only. KSG also resolves ordinary file attachments and extracted-text paths.
- RPIV's structured multi-select questionnaire presentation still uses Pi's native select/input fallback in Shina. Replies work, but there is no equivalent rich questionnaire form.
- KSG has a cross-session usage dashboard, provider maintenance/version checks, configurable launch arguments and multiple Pi provider instances. Shina has per-session statistics and its existing CLI path override, not those settings pages.
- KSG supports provider handoffs. Shina does not switch an existing conversation between harnesses. Pi remains CPA-only.
- Rollback here changes conversation history only. It does not implement KSG's filesystem checkpoint restoration.

Neither application can reproduce arbitrary terminal extension components through RPC. Pi's `custom()` terminal UI, terminal shortcuts, footer components and editor components do not become native controls automatically. Shina does not change Pi authentication or install packages on the user's behalf.

## Validation

Run the Rust checks:

```powershell
cargo fmt --check
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
```

Headless UI tests exercise the controls at 620, 960, 1280 and 1920 px. Reducer tests cover native compaction, stop, failures, steering rejection, compaction races, queue editing/promotion/recovery, session changes, stale responses, history ancestry, extension dialogs and structured results.

The installed-Pi smoke test uses a temporary profile and fixture provider:

```powershell
py tools/pi_native_smoke.py
```

An optional argument selects the installed `dist/cli.js`. It verifies real command discovery, handled extension commands, compaction, clone, RPC fork, CLI fork, steering, queue events, abort, and Shina's permission hook. A custom compaction hook supplies the fixture summary. A local fixture stream exercises the agent scheduler without external inference or credential refresh. The fixture also checks that forking leaves the original session unchanged. The temporary profile is removed afterward.
