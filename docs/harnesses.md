# Thread harnesses

A thread uses `pi` by default, `codex`, or an ACP agent id. Choose Harness at the top of the model picker before the first send. Settings > Default harness cycles the default for new threads and captain homes, with Pi as the initial default. Workers inherit the parent's harness unless `delegate_task.target.harness` selects another. The choice stays fixed after sending. Older indexes without a `harness` field restore as Pi. Saved Codex and ACP sessions use opaque thread/session ids, not Pi session-file paths.

Pi remains CPA-only. Codex and ACP agents use their own configured providers and authentication. Shina does not install agents, sign in, configure providers, read or copy credentials, or edit their authentication files. Configure and authenticate Codex or an ACP agent outside Shina before using it. An authentication denial stops startup without retries.

## Codex app-server

The built-in `codex` harness runs `codex app-server` over JSONL stdio. It is ported from KSG Code's MIT-licensed effect-codex-app-server client, replay protocol, CodexAdapterV2 and CodexDeveloperInstructions. It does not use codex-acp. Shina finds Codex on PATH or in its Windows per-user installation directory. `SHINA_CODEX_CLI` may name a native executable or a JavaScript entry point. Codex inherits its own provider environment and configuration directories.

Shina initializes the connection once, sends `initialized`, discovers models and exact reasoning efforts with paginated `model/list`, then creates or resumes the saved Codex thread. Model selections are per-turn protocol overrides. No configuration file or login changes occur. Text and file/folder paths use Codex text input items. Images use typed image items with data URLs, as KSG does.

| Shina access mode | Codex approval policy | Codex sandbox |
| --- | --- | --- |
| Supervised | `untrusted` | `read-only` |
| Auto-accept edits | `on-request` | `workspace-write` |
| Full access | `never` | `danger-full-access` |
| Read only | `never` | `read-only` |

The first three map KSG's runtime modes. Read only additionally refuses all command, file-change and permission escalation requests without a dialog. Codex enforces its filesystem sandbox. Shina does not disable Windows security settings or set up a sandbox with elevation. If the installed Codex cannot enforce the requested sandbox, the operation fails instead of falling back to Full access.

Command and patch approvals use Shina's existing confirmation dialogs. Agent text, reasoning, plans, commands, patches and MCP/tool items enter Shina's event reducer. `thread/tokenUsage/updated.last` supplies the context meter; cumulative totals are not context usage. Stop calls `turn/interrupt`. Steering uses `turn/steer` with an expected turn id, while follow-ups remain queued. Resume reconstructs the transcript from returned turns and reapplies session overrides.

Codex's non-secret user-input questions use Shina's existing questionnaire. Native compaction, conversation forks and rewinds remain Pi-only. Codex dynamic tool requests receive method-not-found errors, not automatic approvals.

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

`id` must be unique, lowercase, and contain only letters, numbers, dots, underscores or hyphens. Its first character must be a letter or number. `pi` and `codex` are reserved. `name` and `command` are required. `args` and `env` default to empty arrays. `env` contains names to inherit at launch, never secret values. The ordinary OS environment needed for process launch and the agent's own configuration directories is also inherited. Other environment variables, including provider tokens, require an explicit name in `env`. Shina's MCP variables cannot be overridden this way.

Commands can be executable paths or names on PATH. JavaScript entry points run with Node. On Windows, recognizable npm `.cmd` shims resolve to their JavaScript entry rather than running a shell. Other shell wrappers and compound shell commands are unsupported. A command configured in this file runs with your user permissions.

Shina also detects `claude-code-acp`, `codex-acp`, `gemini --experimental-acp`, and `opencode acp` on PATH without running them. A configured entry overrides detection with the same id. Refresh models or restart Shina to reread the registry. A malformed registry shows an error and blocks ACP startup rather than silently substituting a harness.

## ACP protocol and limits

The client implements ACP protocol version 1 as JSON-RPC 2.0 JSONL over stdio. It negotiates capabilities, starts sessions in the thread's project or scratch directory, and supports text and negotiated image prompts. Agent text, thoughts, tool calls and tool results use Shina's existing transcript reducer. Plans and mode updates remain in per-thread runtime metadata. Advertised commands enter the composer command menu.

When the agent advertises `promptCapabilities.embeddedContext`, file attachments use `resource` blocks for valid UTF-8 text without NULs up to 256 KiB, and `resource_link` blocks for binary files, larger files and folders. Without embedded context, Shina sends absolute path text. Images keep their negotiated typed image blocks. Files remain at their original paths; missing files block sending without clearing the draft.

`session/request_permission` uses Shina's existing confirmation queue. Supervised mode asks before mutations and execution. Read and search requests can proceed. Auto-accept edits also approves tools classified as edits. Full access approves permission requests. Declines and cancellation use the agent's advertised option ids. These are approval policies, not an OS sandbox. The ACP agent still controls whether it requests permission.

Stop sends `session/cancel`. An unresponsive stop triggers Shina's existing ten-second disconnect deadline. ACP v1 has no standard steering API, so messages sent during a run become follow-ups and wait for settlement. Stop recovers queued user input. Shina never retries an accepted prompt after an agent error because tools may already have run.

Shina loads saved sessions with `session/load`, or `session/resume` when advertised instead. Load replay rebuilds the transcript. A resume implementation that does not replay history can continue the agent's context but cannot restore Shina's visible transcript. Agents without either capability cannot reconnect a saved thread, including after idle process reclamation. They show an error instead of starting a replacement session.

Model selection supports advertised legacy `models` and model-category select config options. Reasoning selection, native compaction, conversation forks and rewinds remain Pi-only. Filesystem and terminal client capabilities are explicitly false. Requests for them receive method-not-found responses and a visible thread error. ACP v2, native ACP MCP transport and SSE-only MCP agents are unsupported.

## Shina awareness and process ownership

Every real Pi startup passes through the same harness boundary, including lone threads, captain homes, delegated threads, forks and resumes. That boundary appends `mcp::INSTRUCTIONS` and any captain instructions to Pi's system prompt. The Pi bridge also reads the MCP server's `instructions` and adds them before agent execution if they are absent.

ACP sessions register the thread-scoped Shina MCP URL and bearer header when the agent advertises HTTP MCP. Otherwise Shina supplies a stdio MCP server using its own executable's `--mcp-bridge` mode. An invalid-parameters rejection of HTTP setup also permits one stdio attempt, if supported. Authentication failures never trigger that fallback. The relay forwards JSON-RPC to the authenticated loopback HTTP server without saving its token. An agent that explicitly rejects both transports fails startup.

The MCP initialize response contains the same harness-neutral Shina instructions. ACP also receives a separate `<shina_context>` text block on the first prompt, including any captain instructions. Shina persists delivery per session and recognizes the block in replay, so later prompts and resumes do not repeat it. The block stays out of the visible user transcript.

Codex receives `mcp::INSTRUCTIONS` and the same captain profile, scope and essential-memory prompt as Pi in `developerInstructions` on both `thread/start` and `thread/resume`. This leaves Codex's own base instructions intact. `config["mcp_servers.shina"]` registers the thread's loopback URL and bearer header as a required server. Shina does not save these tokens in its index or diagnostics. Later user-curated memory changes use the existing `<shina_memory_diff>` prompt path, identical to Pi.

`delegate_task.target.harness` accepts `pi`, `codex`, or an installed ACP id. With no target, children inherit their parent's harness and pinned or effective selection. A harness-only target uses that harness's configured default. Explicit models and reasoning levels require discovered metadata. `orchestrator_capabilities` reports harnesses and Codex models and efforts once a Codex session has discovered them. Pi still rejects direct providers.

ACP and Codex use the same bounded connection queues, shutdown registry and per-thread process owner as Pi. Windows assigns the agent and its descendants to a kill-on-close Job Object before allowing execution. Unix uses a process group. Reads and writes run off the UI thread, writes and startup have deadlines, and a failed reap quarantines the owner until restart.

## Validation

`cargo test acp::tests` runs a Python ACP fixture from an owned temporary Shina profile. Python must be installed as `python` on Windows or `python3` on Unix. No third-party agents or external inference run in automated tests. Tests cover initialization, authenticated MCP registration and relay, stream reduction, permission responses and policy, cancellation, load/resume, images, queues, model switching, authentication denial and unsupported capabilities. Layout tests cover the harness picker at 620, 960, 1280 and 1920 pixels.

`cargo test codex` runs a temporary Python app-server fixture and harness integration tests. They cover initialization, streaming text and reasoning, command output, plans, approval responses, read-only write refusal, interrupt, resume, current usage, attachment input, model efforts, awareness and harness delegation. The ignored `real_codex_pong_smoke` test sends one paid prompt in an isolated temporary workspace and passes a temporary `LOCALAPPDATA` to Codex. It never uses the real Shina profile.
