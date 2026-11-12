# Thread harnesses

A thread uses `pi` by default, `codex`, `claude`, or an ACP agent id. Choose Harness at the top of the model picker before the first send. Settings > Default harness cycles the default for new threads and captain homes, with Pi as the initial default. Workers inherit the parent's harness unless `delegate_task.target.harness` selects another. The choice stays fixed after sending. Older indexes without a `harness` field restore as Pi. Saved Codex, Claude and ACP sessions use opaque thread/session ids, not Pi session-file paths.

Pi remains CPA-only. Codex, Claude and ACP agents use their own configured providers and authentication. Shina does not install agents, sign in, configure providers, read or copy credentials, or edit their authentication files. Configure and authenticate these CLIs outside Shina before using them. An authentication denial stops startup without retries.

## Codex app-server

The built-in `codex` harness runs `codex app-server` over JSONL stdio. It is ported from KSG Code's MIT-licensed effect-codex-app-server client, replay protocol, CodexAdapterV2 and CodexDeveloperInstructions. It does not use codex-acp. Shina finds Codex on PATH or in its Windows per-user installation directory. `SHINA_CODEX_CLI` may name a native executable or a JavaScript entry point. Codex inherits its own provider environment and configuration directories.

Protocol verification used Codex CLI **0.167.2**, with `codex app-server generate-json-schema --experimental --out <temporary-directory>`, and KSG's generated `packages/effect-codex-app-server/src/_generated/schema.gen.ts` and `meta.gen.ts`. The checked contracts include initialization, model and skill listing, thread start/resume/fork/revert, full turn pagination, goal get/set/clear, feedback upload, turn service-tier overrides, account rate-limit updates and every `ThreadItem` variant. The installed schema additionally includes `thread/settings/update`, absent from KSG's generated request types, which Shina uses to apply the current policy and model before native goal activation. The official protocol guide is <https://developers.openai.com/codex/app-server/>.

At app startup, an off-UI-thread, short-lived app-server runs `initialize`, `initialized`, paginated `model/list` and project-scoped `skills/list`, then exits. It creates no Codex thread and sends no prompt. Models and exact reasoning efforts stay in memory for draft pickers, all Codex threads and `orchestrator_capabilities`. Server projections also carry this metadata to desktop drafts. Refresh models repeats discovery, and refreshes the active session's model and command lists. Changing the working directory refreshes the skill inventory without rereading the shared Codex model catalog. A live `thread/settings/updated` working-directory change also clears the old skills and forces a new directory-scoped `skills/list`. A missing CLI or failed probe leaves a visible picker explanation and retains session-time discovery as a fallback. Authentication denial stops automatic startup until the user takes an explicit recovery action.

Conversation connections initialize independently, discover their live metadata, then create, resume or fork the saved thread. Model selections are per-turn protocol overrides. No configuration file or login changes occur. Text and file/folder paths use Codex text input items. Images use typed image items with data URLs, as KSG does.

When a model advertises `serviceTiers`, or the deprecated `additionalSpeedTiers`, its reasoning picker has KSG's small service-tier segments. Standard uses the documented `default` id; other ids and labels come from the catalog. The choice belongs to the thread, survives app restart and goes in `turn/start.serviceTierForTurn`. It never changes the CLI configuration or the native thread's default service tier. A model change clears the explicit choice. Models without tier metadata have no control.

| Shina access mode | Codex approval policy | Codex sandbox |
| --- | --- | --- |
| Supervised | `untrusted` | `read-only` |
| Auto-accept edits | `on-request` | `workspace-write` |
| Full access | `never` | `danger-full-access` |
| Read only | `never` | `read-only` |

The first three map KSG's runtime modes. Read only additionally refuses all command, file-change and permission escalation requests without a dialog. Codex enforces its filesystem sandbox. Shina does not disable Windows security settings or set up a sandbox with elevation. If the installed Codex cannot enforce the requested sandbox, the operation fails instead of falling back to Full access.

Command and patch approvals use Shina's existing confirmation dialogs. Agent text, reasoning, plans, commands, patches and MCP/tool items enter Shina's event reducer. Commands retain output and exit codes. File changes retain each file's diff and move path. MCP and dynamic tools retain server or namespace, tool, arguments, typed content and structured results. Web results remain in `webResults`. Image views import typed previews when the native path is a readable image. Collaborative calls and subagent activity use subagent cards. Notifications for known child threads retain `parentToolCallId` on nested tool calls and never settle the root turn. Unknown thread ids still fail closed.

The 0.167.2 `ThreadItem` union has user messages, hook prompts, agent messages, function-call outputs, plans, reasoning, command executions, file changes, MCP calls, dynamic calls, collaborative calls, subagent activity, web searches, image views, sleep, image generation, entered/exited review mode and context compaction. Hook prompts, standalone function-call output, sleep, image generation and review markers keep the generic view and their full native payload. There is no standalone file-search, read-file, directory-list or todo-list item in this version's union. Compatibility handlers retain these typed extension items when reported, including supplied `fileSearchResults`; Shina never parses shell prose into search results. Plans and todo-list extensions use the plan card. Opaque web results and unrecognized items keep their native data.

`thread/tokenUsage/updated.last` supplies the context meter; cumulative totals are not context usage. Stop calls `turn/interrupt`. Steering uses `turn/steer` with an expected turn id, while follow-ups remain queued. Resume reconstructs the transcript from native history and reapplies session overrides.

Codex's non-secret user-input questions use Shina's existing questionnaire. Codex dynamic tool execution requests still receive method-not-found errors, not automatic approvals. Rendering a reported dynamic tool item does not authorize executing it.

Whole-conversation forks call `thread/fork` in a separate owned process and preserve the source. `deferGoalContinuation` prevents an inherited goal from starting inference just because the user forked. The installed schema has no `thread/rollback`; conversation rewind uses `thread/revert` with `beforeTurnId`, excluding the selected user turn and every later turn. New verified-version threads request `historyMode: "paginated"`. Resume, fork and rewind hydrate paginated history through `thread/turns/list` with `itemsView: "full"`, ascending pages, bounded history and repeated-cursor checks. The existing fork-message picker uses native turn ids rather than matching prompt text. Fork-from-message first forks, then reverts only the new thread. Rewind restores the selected prompt to the composer and can replace the transcript with an empty history. It does not restore or undo files on disk. Native legacy-history threads can fork in full, but Codex cannot revert them, so rewind and fork-from-message stay disabled.

The slash menu exposes KSG's `/compact`, `/goal` and `/feedback`. `/goal` calls `thread/goal/get`, `thread/goal/set` or `thread/goal/clear`; pause and resume set the native status, and mutations mark their origin as `user`. Before setting an objective or resuming, `thread/settings/update` applies the current working directory, approval policy, sandbox, model and effort without a prompt. An objective activates the native goal. Goal updates remain in `harness_state.goal`. `/feedback [reason]` explicitly uploads the thread and Codex logs to OpenAI with `feedback/upload`. Descriptions name that disclosure. These commands never become model prompts, including a command queued while the first session initializes. Native automatic goal turns still enter the ordinary reducer. Stop first pauses an active native goal, then interrupts its turn so automatic continuation cannot immediately restart it. Autonomous goal turns use the native service-tier default; Shina's explicit tier remains a per-user-turn override.

Goal inspection, pause and clear, and feedback upload remain available during a running turn. Setting or resuming a goal requires idle state. Stop, pause and clear cancel an activation whose settings update is still in flight.

Optional goals, feedback, fork/revert and the associated new history contract are enabled only when `initialize.userAgent` identifies Codex 0.167.2 or later, the minimum version checked here. Unknown and older versions keep the preexisting compact-only behavior. No generic slash-command enumeration method is assumed.

Authentication failures name the exact manual command, `codex login`, and the composer offers Copy command. It copies only that literal command. Shina never runs login, accesses credentials or retries a denied startup automatically.

`/compact` and the context popover's Compact context button call native `thread/compact/start` with the thread id, never a model prompt. Codex's protocol does not accept custom compaction instructions. Shina ignores `/compact` arguments with a visible notice. The request acknowledges startup only; Shina waits for the native turn to finish before releasing waiting prompts. Stop sends `turn/interrupt`, including when startup has not yet supplied the turn id.

Manual and automatic `contextCompaction` items use the same progress, completed, failed and stopped rows as Pi. A failed or interrupted turn closes a running compaction even if Codex omits `item/completed`. The deprecated `thread/compacted` notification also works without duplicating an item-backed row. Resume restores compaction rows from native thread history. Codex's current compaction item contains only an id and type, so Shina does not invent a summary or before/after token counts. Compaction clears current context usage to the dashed unknown state until a fresh `thread/tokenUsage/updated.last` report arrives.

`account/rateLimits/updated` populates `harness_state.usageLimits` through the shared usage-window normalizer below. Sparse updates retain other reported windows. As in KSG, model-specific pools do not replace the main Codex allowance. A reported individual spend limit becomes a Monthly credit limit window.

## Claude

The built-in `claude` harness runs the user's Claude Code CLI over stream-json stdio directly from Rust. There is no Agent SDK runtime or Node sidecar. The protocol port uses installed `@anthropic-ai/claude-agent-sdk` **0.3.321**, its `ProcessTransport`, `Query` and type definitions, and Claude Code **2.1.322**. Behavior follows KSG Code's `ClaudeAdapterV2.ts`, `ClaudeProvider.ts` and `provider/Drivers/ClaudeSkills.ts`. Shina checks PATH, `~/.local/bin`, the legacy `~/.claude/local` installation, and per-user npm locations on Windows and Unix. `SHINA_CLAUDE_CLI` overrides discovery. Missing installations have an unavailable notice in the harness picker.

Launch uses `--print --input-format stream-json --output-format stream-json --verbose --include-partial-messages --replay-user-messages --permission-prompt-tool=stdio`. A `control_request` with subtype `initialize` registers a blocking `PreToolUse` callback and requests nested subagent events. Its `control_response` supplies models, supported effort levels and slash commands before any prompt or inference request. Shina also lists Claude's documented model aliases. Aliases without advertised effort metadata do not claim supported effort levels.

A short-lived initialization-only probe now warms that model catalog at app startup, independently of a conversation. It uses `--no-session-persistence`, no Shina MCP endpoint and no prompt. Drafts and `orchestrator_capabilities` share the in-memory catalog, including through server projections. Refresh models repeats the probe. Discovery failure keeps the same visible session-time fallback as Codex.

The installed CLI's help calls supervised mode `manual`; SDK 0.3.321 calls it `default`. Shina uses `manual` at launch and `default` in `set_permission_mode`. Both control spellings were accepted by CLI 2.1.322 in an initialization-only probe. The SDK's `--permission-prompt-tool=stdio` flag is hidden from this CLI's help but works. `apply_flag_settings` with `effortLevel` changes effort in memory, never through `update_settings` or a configuration file. `set_model` changes the live session model.

| Shina access mode | Claude permission mode | Shina callback policy |
| --- | --- | --- |
| Supervised | `manual` at launch, `default` on the wire | Allow reads, ask for other permission requests |
| Auto-accept edits | `acceptEdits` | Allow reads and edits, ask for other requests |
| Full access | `bypassPermissions` | Approve requests |
| Read only | `dontAsk` | Block every non-read tool in `PreToolUse`, refuse mutation and escalation callbacks without a dialog |

These are tool policies, not an OS sandbox. Claude can satisfy ordinary permissions from its own allow rules before asking the host. Shina's blocking pre-tool hook therefore enforces Read only even for tools covered by those rules, including shell commands, subagents, skills and unknown MCP tools. Startup fails if the CLI cannot enforce a blocking hook. Access changes use the live `set_permission_mode` control. Tightening to Read only takes effect in Shina's hook before the CLI acknowledges the change. A hook or callback cannot undo a tool already running.

The CLI owns persisted sessions. Shina stores only the session UUID, starts in the project or scratch directory, and passes that same UUID to `--resume`. `--replay-user-messages` acknowledges new inputs; it does not replay old turns. Like the SDK's `getSessionMessages`, Shina reads only transcript JSONL under `CLAUDE_CONFIG_DIR/projects`, or `~/.claude/projects`, and follows the saved parent chain to reconstruct visible history. Saved subagent transcripts rebuild nested calls. Missing, malformed, cyclic or oversized history fails resume instead of creating a replacement. The history limit is 64 MiB per transcript. Fork and rewind remain Pi-only even though Claude has session-fork and file-rewind operations. File rewind alone is not conversation rewind.

Assistant text, thinking, tool inputs and results use the shared reducer. `Task` and `Agent` tools become `subagent` calls; nested events carry `parentToolCallId`, and completed tasks populate the existing subagent results. `TodoWrite` updates `harness_state.plan.plan` with `step` and `status` entries, matching Codex's plan key. An `ExitPlanMode` proposal stores markdown in `harness_state.plan.text` and `.markdown`, with status `proposed`. The confirmation dialog approves implementation or keeps planning. Like KSG, Shina denies the native exit-plan operation after capturing it. The chosen follow-up starts after the planning turn settles. `harness_state.planDecision` records `approved` or `keep-planning`. `AskUserQuestion` uses the existing questionnaire and returns answers keyed by the CLI's question text.

Keep planning uses native `plan` permission mode and refuses non-read permission callbacks, even under Full access. Approval restores the chosen access mode before the implementation follow-up.

Stop sends `interrupt` with `cancel_queued: true`; Shina retains the existing ten-second disconnect deadline. The interrupt receipt identifies cancelled input UUIDs for recovery, including their images and file references. Accepted input is never retried after an error. Steering sends another typed `user` message while stdin remains open. Before the CLI advertises cancellable input, steering stays queued so Stop cannot leave an unrecallable follow-up running. Follow-ups stay in Shina's bounded queue until settlement. Images use base64 image content blocks. File and folder attachments use absolute-path text.

Slash commands come from initialization and `commands_changed`. Filesystem discovery adds `skill:` entries for user and project skills and enabled user, project, local and managed plugin installations, including manifest-declared skill directories. Managed settings participate in plugin enablement and skill overrides. The CLI's non-builtin command list supplies account-scoped synced plugin skills without Shina reading account or authentication files. `/skill:name` becomes the CLI's native `/name` invocation.

The context meter uses the last top-level assistant request's `input_tokens`, cache-read tokens, cache-creation tokens and output tokens. It never uses cumulative `result.modelUsage` token totals. Only that metadata's model context capacity is used. Native `/compact`, including optional instructions, drives the Compact context button. Status and compact-boundary messages open and close existing compaction rows. Failure, Stop and disconnect also close a running row. Compaction invalidates usage until another assistant request reports it. Rate-limit events now use the shared normalized `harness_state.usageLimits` contract below instead of retaining an incompatible raw CLI event. Distinct `rateLimitType` windows merge rather than replacing one another.

Authentication failures name `claude auth login` and offer the same Copy command control as Codex. Shina copies only the literal command, never credentials, and never runs it.

The process receives a thread-scoped HTTP MCP server through an inline `--mcp-config` JSON argument, including the loopback URL and bearer header. `--append-system-prompt` adds `mcp::INSTRUCTIONS` and captain instructions. `--system-prompt-snapshot off` keeps those instructions current on resume. No token enters the saved index or diagnostics. User-curated memory changes use the common `<shina_memory_diff>` prompt path.

## Registry

Shina reads `harnesses.json` in its data directory. On Windows this is `%LOCALAPPDATA%\Shina\harnesses.json`. On Unix it is `$XDG_DATA_HOME/Shina/harnesses.json`, or `~/.local/share/Shina/harnesses.json` when that variable is unset. The file is optional and Shina never rewrites it.

The file is an array. For example:

```json
[
  {
    "id": "my-acp",
    "name": "My ACP agent",
    "command": "C:\\Tools\\my-acp.exe",
    "args": ["--acp"],
    "env": ["MY_AGENT_API_KEY"]
  }
]
```

`id` must be unique, lowercase, and contain only letters, numbers, dots, underscores or hyphens. Its first character must be a letter or number. `pi`, `codex` and `claude` are reserved. `name` and `command` are required. `args` and `env` default to empty arrays. `env` contains names to inherit at launch, never secret values. The ordinary OS environment needed for process launch and the agent's own configuration directories is also inherited. Other environment variables, including provider tokens, require an explicit name in `env`. Shina's MCP variables cannot be overridden this way.

Commands can be executable paths or names on PATH. JavaScript entry points run with Node. On Windows, recognizable npm `.cmd` shims resolve to their JavaScript entry rather than running a shell. Other shell wrappers and compound shell commands are unsupported. A command configured in this file runs with your user permissions.

Shina also detects `claude-code-acp`, `codex-acp`, `gemini --experimental-acp`, and `opencode acp` on PATH without running them. A configured entry overrides detection with the same id. Refresh models or restart Shina to reread the registry. A malformed registry shows an error and blocks ACP startup rather than silently substituting a harness.

## ACP protocol and limits

The client implements ACP protocol version 1 as JSON-RPC 2.0 JSONL over stdio. It negotiates capabilities, starts sessions in the thread's project or scratch directory, and supports text and negotiated image prompts. Agent text, thoughts, tool calls and tool results use Shina's existing transcript reducer. Plans also enter the transcript as typed cards, with later updates replacing the first card. Mode updates remain in per-thread runtime metadata. Advertised commands enter the composer command menu.

When the agent advertises `promptCapabilities.embeddedContext`, file attachments use `resource` blocks for valid UTF-8 text without NULs up to 256 KiB, and `resource_link` blocks for binary files, larger files and folders. Without embedded context, Shina sends absolute path text. Images keep their negotiated typed image blocks. Files remain at their original paths; missing files block sending without clearing the draft.

`session/request_permission` uses Shina's existing confirmation queue. Supervised mode asks before mutations and execution. Read and search requests can proceed. Auto-accept edits also approves tools classified as edits. Full access approves permission requests. Declines and cancellation use the agent's advertised option ids. These are approval policies, not an OS sandbox. The ACP agent still controls whether it requests permission.

Stop sends `session/cancel`. An unresponsive stop triggers Shina's existing ten-second disconnect deadline. ACP v1 has no standard steering API, so messages sent during a run become follow-ups and wait for settlement. Stop recovers queued user input. Shina never retries an accepted prompt after an agent error because tools may already have run.

Shina loads saved sessions with `session/load`, or `session/resume` when advertised instead. Load replay rebuilds the transcript. A resume implementation that does not replay history can continue the agent's context but cannot restore Shina's visible transcript. Agents without either capability cannot reconnect a saved thread, including after idle process reclamation. They show an error instead of starting a replacement session.

The composer's `$` menu uses Pi's skill commands, Codex `skills/list`, or ACP's advertised `$` commands. Claude ACP agents also discover user, project, additional-directory and enabled plugin skills from their filesystem metadata. Claude skill dispatch follows KSG Code's native slash-command text blocks. Pi's bridge expands selected skills with Pi's frontmatter loader. Discovery does not read credential or account files. Shina reads synced Claude plugin skills only when one marked account bucket is unambiguous.

Model selection supports advertised legacy `models` and model-category select config options. Native compaction, reasoning selection, conversation forks and rewinds are unavailable for ACP. Filesystem and terminal client capabilities are explicitly false. Requests for them receive method-not-found responses and a visible thread error. ACP v2, native ACP MCP transport and SSE-only MCP agents are unsupported.

## Shina awareness and process ownership

Every real Pi startup passes through the same harness boundary, including lone threads, captain homes, delegated threads, forks and resumes. That boundary appends `mcp::INSTRUCTIONS` and any captain instructions to Pi's system prompt. The Pi bridge also reads the MCP server's `instructions` and adds them before agent execution if they are absent.

ACP sessions register the thread-scoped Shina MCP URL and bearer header when the agent advertises HTTP MCP. Otherwise Shina supplies a stdio MCP server using its own executable's `--mcp-bridge` mode. An invalid-parameters rejection of HTTP setup also permits one stdio attempt, if supported. Authentication failures never trigger that fallback. The relay forwards JSON-RPC to the authenticated loopback HTTP server without saving its token. An agent that explicitly rejects both transports fails startup.

The MCP initialize response contains the same harness-neutral Shina instructions. ACP also receives a separate `<shina_context>` text block on the first prompt, including any captain instructions. Shina persists delivery per session and recognizes the block in replay, so later prompts and resumes do not repeat it. The block stays out of the visible user transcript.

Codex receives `mcp::INSTRUCTIONS` and the same captain profile, scope and essential-memory prompt as Pi in `developerInstructions` on both `thread/start` and `thread/resume`. This leaves Codex's own base instructions intact. `config["mcp_servers.shina"]` registers the thread's loopback URL and bearer header as a required server. Shina does not save these tokens in its index or diagnostics. Later user-curated memory changes use the existing `<shina_memory_diff>` prompt path, identical to Pi.

`delegate_task.target.harness` accepts `pi`, `codex`, `claude`, or an installed ACP id. With no target, children inherit their parent's harness and pinned or effective selection. A harness-only target uses that harness's configured default. Explicit models and reasoning levels require discovered metadata. `orchestrator_capabilities` reports the shared prompt-free native catalogs before any session, then supplements them with live session metadata. Codex models also report `service_tiers`. Pi still rejects direct providers.

ACP and Codex use the same bounded connection queues, shutdown registry and per-thread process owner as Pi. Windows assigns the agent and its descendants to a kill-on-close Job Object before allowing execution. Unix uses a process group. Reads and writes run off the UI thread, writes and startup have deadlines, and a failed reap quarantines the owner until restart.

Claude uses that same process owner, bounded queues, shutdown registry and kill-on-close implementation.

## Validation

### Typed transcript tools

`Tool.kind` and `Tool.changes` default to absent and empty for saved transcripts and
older server projections. Only an absent kind uses the legacy Pi name classifier.
Each change keeps `path`, optional `old_path`, `operation`, unified `diff`, `additions`
and `deletions`. ACP old/new text becomes a unified diff before flattening.

`Tool.input` retains named input fields. `Tool.structured` retains the native
`structuredContent`, `details` and `nestedCalls`, plus `input`. Normalized search
results live at `fileSearchResults` as `{fileName, line?, column?, preview?}` and
`webResults` as `{url, title?, snippet?}`. Providers that supply no structured
results keep their text output. No result entries are inferred from prose.

Plans use typed tools so the existing transcript projection carries each card.
Adapters replace the same plan ID's payload in place, including in replay history.
Tool and thought `order` values preserve events at the same text offset and default
to the old content-block order for legacy records.

### Usage limits

Codex and Claude use one `harness_state.usageLimits` shape:

```json
{
  "windows": [
    {"id": "codex:primary", "name": "5 hour", "usedPercent": 92.0, "resetsAt": 2100000000}
  ]
}
```

`resetsAt` is Unix seconds, or null when unreported. Percentages are finite and
clamped to 0 through 100. Codex supplies `usedPercent` directly. Claude's
`utilization` fraction becomes percent, and `rateLimitType` identifies its window.
Missing windows preserve earlier reports. These are account usage allowances,
not the thread's context-window capacity.

The context popover shows a Usage block only when windows exist. Each row has its
window name, used percent, a 6 px bar and the local reset date and time, or
"Reset not reported". Counts and times use 11 px Departure Mono. Bars use the
existing faint ink and coral at 90 percent or more. The card keeps its existing
palette, frame, spacing and Compact context action. Service-tier segments follow
KSG's `ComposerModelEffort.SpeedControl` footer style.

Compaction records retain a transcript `anchor` and their original `historyIndex`
in provider history. Completion replaces the running record without moving it.
Legacy compactions without position metadata remain at the transcript's end.
`AssistantWork.stopped` is a durable turn marker and defaults to false.
`harness_state.workRetries` retains one anchored retry event per turn, including
its latest attempt, limit when reported, recovery or final error. Native Codex
retry errors do not always include an attempt limit; Shina does not invent one.

`cargo test acp::tests` runs a Python ACP fixture from an owned temporary Shina profile. Python must be installed as `python` on Windows or `python3` on Unix. No third-party agents or external inference run in automated tests. Tests cover initialization, authenticated MCP registration and relay, stream reduction, permission responses and policy, cancellation, load/resume, images, queues, model switching, authentication denial and unsupported capabilities. Layout tests cover the harness picker at 620, 960, 1280 and 1920 pixels.

`cargo test codex` runs a temporary Python app-server fixture and harness integration tests. They cover initialization, prompt-free paginated discovery and failure, pre-send model capabilities, project skills, streaming text and reasoning, command output, every schema item and generic fallbacks, live/history metadata parity, nested calls, plans, approval responses, read-only write refusal, interrupt, fork/revert and full turn history, normalized usage windows, native goal/feedback dispatch, turn-only service tiers, attachment input, model efforts, awareness and harness delegation. Compaction tests cover manual success, request rejection, failed turns, automatic and deprecated notifications, prompt races, cancellation, usage invalidation and refresh, and resume history. The ignored `real_codex_pong_smoke` test sends one paid prompt followed by native compaction in an isolated temporary workspace and passes a temporary `LOCALAPPDATA` to Codex. It never uses the real Shina profile.

`cargo test claude` runs a Python stream-json fixture under an owned temporary Shina profile and Claude transcript directory. Tests cover the handshake, MCP registration, streamed text and thinking, tool results, nested calls, all permission policies, plan and questionnaire responses, steering, interruption and recovery, saved-session history, model and effort controls, image input, native compaction, authentication denial, missing executables, skill discovery and delegation capabilities. The ignored `real_claude_pong_smoke` test sends one real prompt in an isolated temporary workspace and Shina profile. It uses the CLI's existing authentication without reading or changing it. Run it only when the CLI is already signed in.

Claude tests also cover the prompt-free non-persistent catalog probe, normalized
multi-window limits and the exact manual login command. Headless UI tests cover
usage rows and service-tier controls at 620, 960, 1280 and 1920 px, plus both
authentication copy controls. No real provider prompt was run for this change;
both paid smoke tests stayed ignored.
