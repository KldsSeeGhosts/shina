# Thread harnesses

A thread uses `pi` by default or an ACP agent id. Choose Harness at the top of the model picker before the first send. The choice stays fixed after sending. Older indexes without a `harness` field restore as Pi. Saved ACP sessions use an agent session id, not a Pi session-file path.

Pi remains CPA-only. ACP agents use their own configured providers and authentication. Shina does not install agents, sign in, configure providers, copy credentials, or edit their authentication files. Configure and authenticate an ACP agent outside Shina before using it. An authentication denial stops startup without retries.

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

`id` must be unique, lowercase, and contain only letters, numbers, dots, underscores or hyphens. Its first character must be a letter or number. `pi` is reserved. `name` and `command` are required. `args` and `env` default to empty arrays. `env` contains names to inherit at launch, never secret values. The ordinary OS environment needed for process launch and the agent's own configuration directories is also inherited. Other environment variables, including provider tokens, require an explicit name in `env`. Shina's MCP variables cannot be overridden this way.

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

The MCP initialize response contains the same harness-neutral Shina instructions. ACP also receives a separate `<shina_context>` text block on the first prompt, including any captain instructions. Shina persists delivery per session and recognizes the block in replay, so later prompts and resumes do not repeat it. The block stays out of the visible user transcript. Delegation currently creates local CPA-backed Pi children, even when their parent uses ACP.

ACP uses the same bounded connection queues, shutdown registry and per-thread process owner as Pi. Windows assigns the agent and its descendants to a kill-on-close Job Object before allowing execution. Unix uses a process group. Reads and writes run off the UI thread, writes and startup have deadlines, and a failed reap quarantines the owner until restart.

## Validation

`cargo test acp::tests` runs a Python ACP fixture from an owned temporary Shina profile. Python must be installed as `python` on Windows or `python3` on Unix. No third-party agents or external inference run in automated tests. Tests cover initialization, authenticated MCP registration and relay, stream reduction, permission responses and policy, cancellation, load/resume, images, queues, model switching, authentication denial and unsupported capabilities. Layout tests cover the harness picker at 620, 960, 1280 and 1920 pixels.
