# Shina

A native desktop chat workspace built with [Rune](https://github.com/anomalyco/rune) and the [Pi coding agent](https://github.com/earendil-works/pi). Shina is a working-name prototype, not a T3 Code or Capy client.

The standalone app's interface is entirely Rune. Pi, Codex, or a selected ACP agent runs as a hidden background RPC process. There is no terminal emulator, webview, browser server, or Electron runtime in this app.

Threads default to Pi and its CPA-only provider. A new thread can instead select Codex or an installed ACP agent from the Harness group in the model picker. Settings has a Default harness item for new threads and captain homes. Workers inherit their parent's harness unless a delegation target selects another. Codex uses `codex app-server`, not ACP. Codex and ACP agents use their own configured providers and login. Shina never reads or copies their credentials, changes their configuration, installs agents, or changes their login state. See [thread harnesses](docs/harnesses.md) for discovery, MCP setup and protocol limits.

Captains are assistants with one continuous home, explicit memory, and an ordered scope of zero or more workspaces. Opening a captain always opens its primary home, regardless of the sidebar filter. Earlier homes and reset conversations remain in its records. Captains coordinate read-only work and delegate changes to durable tasks. See [captains, scope and memory](docs/captains.md) for the data model, migration, tools, and backend UI API.

## Run on Windows

The development tools and Pi are already installed on this workstation.

```powershell
cargo run
```

### Experimental local server

Legacy single-process mode remains the default for unmigrated profiles. Slices 1 through 4 of the [server architecture](docs/server-architecture.md) are implemented. Server mode is opt-in for a new profile:

```powershell
.\target\debug\shina.exe serve
.\target\debug\shina.exe --server-client
```

`serve` is headless and owns harnesses, tasks, memory, routines and MCP. The desktop attaches to an authenticated loopback WebSocket endpoint described by the user-only `server.json`. If no server owns the profile, the desktop starts one and supervises it with 0.5 to 10 second restart backoff. Quitting stops only a server that desktop spawned, with a five-second grace. An independently started server keeps running after its clients close. Windows' background tray setting keeps the desktop and its owned server alive even after idle windows close.

Both execution modes take the same `server-owner.sqlite` exclusive transaction before opening writable state. Do not delete that file. A legacy app cannot execute against a profile owned by `serve`. No provider credentials or authentication settings are copied or changed.

Clients render projected records through the existing `State` and `Thread` APIs. Editors, attachments not yet submitted, selection, menus and expansion state stay local. Concurrent desktop launches reserve separate persistent draft slots under `clients/<slot>/ui-state.json`. Existing legacy drafts are read without rewriting their source. Stop leaves recovered input for an `input.claim`. Only the initiating client automatically claims its stop/rewind results, never another client's editor.

Server state now lives in `state.sqlite`, using bundled SQLite, WAL and full synchronous commits. One verified transaction imports `index.json`, its tasks/runtime inputs, the server-owned part of UI state, `memory.json`, `routines.json` and `honcho-outbox.json`. A marker records source hashes. Every legacy file remains unchanged, and a verified import backup is retained. Invalid schemas, corrupt files or database failures block writes without JSON fallback. Legacy mode keeps JSON for unmigrated profiles. Once `state.sqlite` exists, a plain desktop launch automatically attaches to or supervises the local server in client mode. `--server-client` still works. Do not remove the database to bypass migration safety.

Every mutation has a client UUID. State changes, receipts and outbound effects commit together before harness sends. Retrying the same device/command ID returns its receipt; changed payloads fail. Process-bound effects left pending or dispatching by a crash become interrupted and are never automatically repeated. Existing history and task delivery markers still decide what the provider accepted. Honcho retains its stable-ID remote reconciliation. This does not promise exactly-once provider tools.

Protocol 1.3 adds scoped pairing and image uploads to 1.2 restart coordination and 1.1 cursor replay, bounded typed patches and compact snapshots. Protocol 1.0 clients receive full snapshots and upgrade guidance if they cannot coordinate a restart. Transcripts travel through revision-pinned `thread.history` pages, including single oversized messages. Image-file reads use authenticated `/v1/blobs/{id}` offset/limit paging. Pages default to 64 KiB of data, with a 256 KiB maximum. Workspace metadata also pages when needed. The native client's reassembly budget is 64 MiB, replacing the old 2 MiB snapshot cap. Control requests remain limited to 1 MiB, and the runtime queue to 32 commands. Slow clients cannot block harness drains. New commands, including approvals, are refused while disconnected.

Local bootstrap exchange issues durable per-device credentials. The server stores token hashes only; native clients keep their token in their protected draft-slot directory. The `devices.list` protocol method lists IDs, scopes and revocation state. A `devices.revoke` mutation takes `args.deviceId` and a command UUID. It blocks existing sockets and reconnects, including bootstrap reuse of that device ID. A denied client does not silently enroll again. Paired user-device credentials grant read, operate and approve access, without maintenance or peer delegation.

Installed server clients use the same in-app restart action as legacy mode. The server refuses updates while work, configuration, queues, approvals, clones or routines are active. A leased drain fences mutations and routine dispatch while clients save their drafts. A failed save or preparation timeout cancels the drain. Only after every connected local client acknowledges does the helper close clients and the server, wait for their processes, back up state, replace the app, validate server readiness and reopen clients. Backups remain under `Shina/backups`; rollback restores the matching binary and state. Remote clients reconnect without closing their desktop. They cannot authorize host maintenance. Browser connections remain out of scope.

### Opt-in always-on server

Desktop login startup remains off. Hosts that should keep running without a desktop window can opt in:

```text
shina service install
shina service status
shina service start
shina service stop
shina service uninstall
shina update --yes
```

Linux uses a systemd user unit. `service install --enable-linger` explicitly asks `loginctl` to enable lingering without sudo or a password prompt. Without that flag, install leaves lingering unchanged and explains the logout limit. macOS uses a per-user LaunchAgent. Windows uses a per-user logon scheduled task with `InteractiveToken`, least privilege and hidden process startup. No administrator rights or desktop login item are requested. A normal explicit stop stays stopped until the next service start or login. Unexpected failures restart through the platform manager.

Service installation reconciles executable and provider discovery paths, not credentials. It inherits the host's environment and reads an existing `Shina/service.env` on every launch. This file accepts one `NAME=value` assignment per line, optional `export`, comments and quoted values, without shell execution or expansion. Use the host's existing Pi/CPA and Codex configuration and login. Shina never exports authentication into unit files. Logs go to `Shina/service.log`, or `~/Library/Logs/Shina/service.log` on macOS, where launchd cannot open files on other volumes. `service stop`, `service uninstall` and the tray's explicit Stop server can interrupt work. Updating cannot.

`shina update --yes` uses the existing GitHub CLI login and release checksums, including `Shina-linux-x86_64.tar.gz` on headless Linux. The flag explicitly approves an idle-only restart. Authentication denial stops that attempt and asks you to run `gh auth login` yourself. A protocol-major mismatch refuses commands with upgrade guidance; protocol minor versions negotiate down. In server-client mode the Windows tray shows connection status and sends Stop server through the authenticated client connection. Quitting an attached client does not stop an independent service.

For any development GUI check, set `LOCALAPPDATA` to a new temporary directory before launching either process. Never use the real profile for fixtures. `cargo test --test server_runtime` starts real servers and fake Pi/ACP subprocesses in isolated profiles without consuming quota.

## Linking machines

Each installation owns its threads, captains, projects, provider login and host paths. Linking is direct client access, not filesystem or session replication. The desktop keeps its local connection and one independent connection per linked server. The sidebar groups all servers' threads and captain homes under quiet machine labels. The composer's "Run on" menu starts new work on a selected server. Clicking another server's existing row opens it without moving it. New work defaults to Local after desktop launch.

Tailscale must already be installed, signed in, and resolve MagicDNS names. Shina does not change Tailscale or firewall settings. `serve` binds only loopback unless `--tailnet` is explicit. Tailnet mode adds each validated `Self.TailscaleIPs` address, IPv4 `100.64.0.0/10` or IPv6 `fd7a:115c:a1e0::/48`, on TCP port 47329. It never binds a wildcard or LAN/public address. `--port <number>` changes the port for isolated testing or an alternate installation. The loopback bootstrap endpoint refuses requests on the remote listener, even with the correct bootstrap token.

After installing this build on each host, run these commands in its own terminal. These commands opt into an always-on server; do not run a legacy desktop against the same profile.

Windows, `windows-pc`, PowerShell:

```powershell
& "$env:LOCALAPPDATA\Programs\Shina\shina.exe" service install --tailnet
& "$env:LOCALAPPDATA\Programs\Shina\shina.exe" service status
& "$env:LOCALAPPDATA\Programs\Shina\shina.exe" pair
```

macOS, separately on `studio-mac` and `laptop-mac`:

```sh
/Applications/Shina.app/Contents/MacOS/shina service install --tailnet
/Applications/Shina.app/Contents/MacOS/shina service status
/Applications/Shina.app/Contents/MacOS/shina pair
```

Ubuntu, `linux-server`, with the installed `shina` on PATH:

```sh
shina service install --tailnet --enable-linger
shina service status
shina pair
```

Omit `--enable-linger` to leave Linux logout behavior unchanged. To run without a service, use `shina serve --tailnet` in a separate terminal instead. Reinstalling a service without `--tailnet` restores loopback-only service startup. Stop a standalone tailnet server and restart with plain `shina serve` to remove its remote listeners. Coordinated updates retain the running server's listener options.

Create a code on the host you want to access. On the client machine, redeem that code once, within five minutes. For example, after `pair` on `studio-mac`, run on Windows:

```powershell
& "$env:LOCALAPPDATA\Programs\Shina\shina.exe" link studio-mac <code-from-studio-mac>
& "$env:LOCALAPPDATA\Programs\Shina\shina.exe" links
& "$env:LOCALAPPDATA\Programs\Shina\shina.exe" --server-client
```

On either Mac use `/Applications/Shina.app/Contents/MacOS/shina link <host> <code>`. On Ubuntu use `shina link <host> <code>`. Replace `<host>` with `windows-pc`, `studio-mac`, `laptop-mac` or `linux-server`. Generate a fresh code for each client/server link. Linking A to B does not grant B access to A. Repeat for each direction needed; no full mesh is required. CLI links load when the desktop starts. An open desktop can link immediately through Run on > Add to another machine or Settings > Machines.

The dialog also creates a host code, lists authorized devices and revokes them. The CLI equivalents are:

```text
shina devices
shina devices revoke <device-id>
shina devices grant <device-id> update
shina devices revoke-grant <device-id> update
shina links
shina unlink <server-id-or-host>
```

`devices` lists inbound grants on that host; `links` lists this client's outbound server links without tokens. `unlink` first asks the remote server to revoke this client's credential, then removes the local credential file. If the server is offline, it keeps the link and reports that revocation was not acknowledged. Revoke on the host instead when necessary. After a host revocation, automatic reconnect stops. Run `pair` on the host and explicitly `link` again to replace the denied credential.

Server-to-server delegation uses a separate, outbound peer link. Run these commands on the target and home servers respectively, using the installed binary's full path where needed:

```text
# On the target server, through its local maintenance device
shina pair --peer

# On the captain's home server
shina peer link <host> <peer-code>
shina peer list
shina peer unlink <server-id-or-host>
```

Peer codes have the same five-minute lifetime, single-use consumption and host-wide throttling as client codes. A peer has only `read` and `peer-delegate`. It can launch workers and read, send to or stop only threads that its credential created. It cannot subscribe to the workspace, browse arbitrary host folders, approve requests, maintain the host or link onward. Peer credentials live in the protected profile under `Shina/peers/<server-id>/credential.json`; the `peers` SQLite table and projections contain metadata only. Shina checks the answering server's identity before sending a credential, and refuses proxies and redirects.

To replace a revoked peer credential, mint a fresh peer code and repeat `peer link` with the same host. Shina confirms that the old credential is revoked before replacing its protected file. It does not silently move an existing link to another address.

`orchestrator_capabilities` and `list_captains` report peers and their cached reachability, projects, harnesses and models. In `delegate_task`, `target.machine` selects a peer by server id, host or label. `workspace` then names a project id or path on that server. The target repository must belong to the caller's workspace scope, matched by git remote. An explicit selector chooses a target checkout without expanding that scope. Without it, the caller's git remote must match exactly one target project. A stable `client_request_id` retries an uncertain launch without creating another worker. Sleeping machines leave the task pending, with retries between two seconds and five minutes. Cancellation commits at home immediately and remains owed until the target acknowledges it. Recovered results wait for review before delivery. Questions and approvals stay on the target; memory suggestions require the home captain's explicit `remember`.

The Linked machines dialog's Peers section manages the local server's outbound links. On a linked remote view it lists that server's peers without maintenance controls. A remote task opens on its owning server if the desktop also has a client link to it; otherwise it opens the cached result excerpt.

The `update` grant is off for paired devices. Turn it on or off in the target machine's own terminal, or under Settings > Machines > Authorized devices on that machine. Only local maintenance credentials can change it. It permits exactly `server.updateStatus` and `server.requestUpdate`, not restart, stop, pairing, shell execution, or other maintenance methods. Ordinary local maintenance credentials can also use those two update methods.

Protocol 1.6 advertises `linked-updates`. "Update all machines" reviews Local and every linked machine before sending requests. Each eligible machine resolves its own latest GitHub release with its own existing `gh` login and verifies the existing size and SHA-256 checks. The client cannot choose a release, URL, or binary. Requests go directly to each machine in parallel. Local restarts last, after the remote machines reach restart handoff or finish. Offline machines, older protocols, missing update grants, unsupported installations and busy machines are skipped with reasons. Nothing queues an offline update or retries one automatically. Closing the dialog does not cancel an accepted request.

The target refuses active, queued, configuring or approval-blocked work and checks again when the helper prepares its drain. Acceptance consumes a durable ten-minute per-device limit, even if the release check later fails or finds nothing new. Schema 4 adds `update_audit` after a verified backup. Authenticated update attempts, including grant denials and other refusals, record their device, time, update ID and outcome; resolved tags and final outcomes attach to accepted requests. `updates/outcome.json` preserves helper success or the specific failure reason across restart and rollback. Clients receive the last outcome in the projection and a dismissible notice after reconnect. A remembered failed release still requires explicit Retry on the target before that tag can download again.

Codes have 48 bits of OS randomness, expire after five minutes, allow five exchange attempts per minute per host, and work once. Durable tokens have 256 bits of OS randomness. Servers store hashes and scopes in SQLite; clients store secrets outside repositories in `Shina/linked/<server-id>/credential.json`. Windows uses a protected current-user-only DACL. Unix uses a private directory and mode 0600 credential file. Tokens never enter argv or logs. Only the one-time code enters `link` argv. HTTP/WebSocket traffic uses Tailscale's encrypted transport; proxies, redirects and non-tailnet destinations are refused. MCP agents receive only their existing thread-bound MCP token, never a paired user credential, so they cannot answer approvals or mint credentials.

Each desktop draft slot stores per-server UI state and last-known projections separately. Sleeping servers appear offline and reconnect with their own cursor and backoff. Cached threads remain readable. New commands and approvals are not queued offline, and work never falls back to Local. Projects use the selected server's absolute paths; remote workspace dialogs do not browse this client's filesystem. Image attachments upload in bounded, offset-checked chunks and validate their hash and image content before submission. Ordinary local file/folder references cannot be sent to another server.

Cross-server captain delegation remains slice 5. Each captain currently owns and delegates work only on its home server.

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

## Settings

The sidebar gear opens a searchable, keyboard-accessible dialog with General, Threads, Harnesses, Machines, Memory, Updates and About sections. Theme stays in General. Machines contains linking, host codes, authorized devices and the update review. At 620 px the dialog fills the window and its Back control closes it.

General edits the existing profile and background controls, theme, and archive/delete confirmations. Threads edits auto-settle, snooze duration and calendar presets, the default access mode and harness, and provider/model ID/effort defaults per harness. New threads still inherit the current thread's valid choices. Defaults apply when no valid selection is inherited, including the first draft after restoring a profile with no live threads. Snooze accepts comma-separated minutes or `day@HH:MM`, for example `60, tomorrow@09:00, monday@10:30`. It uses the existing DST-aware resolver, with at most eight presets and durations up to 30 days. With linked machines, the dialog shows which machine owns server settings. Its context starts at Local and does not change the open conversation. Profile, keep-awake, thread defaults and auto-settle edits go to that selected machine. The profile editor retains its target until closed. Theme, background tray behavior and confirmation preferences stay on this desktop.

Harnesses shows local discovery paths and bounded `--version` results for built-in agents. Version probes remove Shina's thread credentials and do not forward prompt or server arguments. Opaque ACP binaries are not executed for version discovery; script package metadata may supply a version. ACP entries remain read-only. Shina never installs agents, signs in or edits their configuration. Memory shows Local's Honcho status, queue counts and last error. Reveal controls open the existing local `harnesses.json`, `honcho.json` or data folder in the system file manager. Updates shows the desktop version, last check in this session, and the remembered local failed release with an explicit retry. About includes the bundled licences.

Server-owned settings use receipted commands and the existing index document. Device-only preferences use `ui-state.json`; no second settings store exists. There is no "Continue threads after an update" switch. The shared update guard refuses queued inputs, startup work, approvals and unfinished delegated dispatch, and the drain fences new work. A validated update therefore has no pending thread queue to resume. It never adds a continuation prompt to a finished turn.

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

Cargo.toml holds the development baseline, currently `0.12.0`. CI replaces its zero patch with the workflow run number in the build checkout only, for example `0.12.42`. The executable and Mac bundle embed that exact version. Source commits and credentials do not change. Keep the workflow identity and its run counter; when starting a new major/minor line, keep the baseline patch at zero. Platform assets arrive independently, while `SHA256SUMS` grows to cover uploaded assets. Reruns preserve assets already published for the same commit.

The `dev-release` profile disables LTO, enables incremental compilation, and caches dependencies between main pushes. Tests, formatting and Clippy run only when the manual workflow's `checks` input is enabled. The normal push builds once and publishes. Use `cargo run` for local iteration without waiting for CI. Full optimization remains available manually through the existing release profile.

The workflow uses hosted GitHub runners and the job-scoped `GITHUB_TOKEN`; no CPA enrollment, signing secrets, or workstation access is needed. Mac bundles retain the existing ad-hoc signing rather than Developer ID notarization. Hosted runner usage is billed against the private repository's Actions allowance. New main pushes cancel superseded builds. Checksums, saved drafts, explicit restart and rollback remain in the app; they do not delay building or publication.

Linux ships as `Shina-linux-x86_64.tar.gz`, built on Ubuntu 24.04. Extract and run `./Shina/shina` with a Vulkan-capable desktop and existing Pi/CPA enrollment. It requires glibc 2.39 or newer. Linux hosts can update that executable with `shina update --yes`, including a coordinated service restart. Linux has no desktop installer or automatic in-app download yet. Windows and Mac retain their existing in-app updater.

An installed copy of Shina updates itself without a download step. It checks GitHub Releases 2 seconds after start and every 30 seconds after each check, through the GitHub CLI and your existing `gh` login, because the repository is private. When a newer version exists, Shina downloads it in the background, checks its size and SHA-256 against the release (the asset digest, or the release's `SHA256SUMS` file) and prepares the replacement next to the installed app. The sidebar shows a passive **Downloading update vX.Y.Z** row meanwhile, then one **Restart to update to vX.Y.Z** button and nothing else.

The button is disabled while any thread is running, waiting for you, configuring Pi, or has queued messages or an open question, while a routine is running, while a repository is cloning, and while a rename or captain editor is open. Merely scheduled or paused routines do not block an update. Composer drafts never block, because they are saved. A restart saves and flushes everything first, swaps the executable or app bundle, and reopens Shina on your last workspace.

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
- A native sidebar with warm light/dark palettes, compact tabs, search, and an optional overview pane. Installed Anthropic Sans and Serif are preferred for controls and replies. Bundled Inter and Source Serif 4 cover machines without those fonts. Code keeps the platform's monospace font. The window follows the Pixel Voyage skin in `docs/ui/pixel-voyage.md`: a coral accent, pixel-art icons from Pixelarticons (MIT), and Departure Mono (SIL OFL) for labels, counts and timers. Both licences ship under `assets/`.
- Four crew captains, each an original 40x40 pixel-art portrait that reacts to the crew's live state: Normal when ready, Determined while working, Surprised when a thread needs you, Dizzy after a failure, and Happy when settled. The set is Ember, Juniper, Sable, and Pip.
- Captains are persistent coordinators with one primary home and their own crew, separate from lone threads. Several captains can share a workspace, and each captain can have zero or more scoped workspaces. New-captain creation suggests a workspace without a default captain. Earlier conversations remain in the captain's records. Captain homes never enter thread shelves or thread search. Opening crew highlights the captain. Crew approvals surface on the parent row; questions stay with the coordinator.
- Independent Pi processes for active threads. Switching threads preserves their drafts and leaves other work running. Shina releases processes after 30 idle minutes and resumes their saved session on the next send.
- Running, needs-input, and failed states. `agent_end` does not finish a run; the adapter waits for `agent_settled`. A finished thread goes quiet instead of showing a status, and a stopped run returns to idle. Background threads that finish get an unread badge.
- Steer an active run with Enter or the send button. Ctrl/Command-click or Ctrl/Command+Enter queues a separate turn instead. Queued messages support edit, remove and Send now actions. Stop returns unsent queued messages to the draft and clears Pi's native queue before aborting.
- Attach files and folders with the composer button, clipboard paste or file drop. Shift-click Attach to pick a folder. Each message accepts 20 attachments, including up to eight PNG, JPEG, GIF or WebP images, 10 MiB per image and 20 MiB combined image bytes. Remove attachments from their chips. Ordinary files stay at their original absolute paths for the agent's tools. ACP agents can receive negotiated embedded text resources. Shina validates images before sending typed image parts; text-only models refuse images without clearing the draft.
- The composer context ring shows Pi's current context usage, not total tokens across the session. Hover for usage and capacity, or focus and activate the ring to open its details. It refreshes after startup, completed turns, model changes and compaction. Unknown post-compaction usage has a dashed ring until Pi reports a new measurement.
- Workspace-local `/` completion for Pi extension commands, prompt templates and skills. `/model` opens the picker. `/compact [instructions]` and the context details' Compact context button call Pi's native compaction RPC, without asking the chat model to summarize. Compaction status, summaries and token figures remain visible after resuming the session.
- Fork a conversation or fork before a selected user message from the thread menu. Edit from here rewinds the conversation through Pi's native fork operation and restores the selected prompt and its images. Files are not reverted, and the original Pi session stays on disk. Source threads keep their own processes and sessions.
- Native confirmation, selection, editor and text replies for Pi RPC dialog records, including concurrent requests and expiration. Confirmations require the user, unless they previously chose Allow for session for that exact request. The permission choice lasts only for that Pi process.
- One atomic global index remembers projects, captain homes and scopes, durable tasks, thread ownership, titles, pins, archive state, model/thinking choices, tabs and session references. A separate atomic `memory.json` holds explicit captain memory and the small global user profile. Opening a saved thread restores its transcript, tool summaries and tool results automatically.
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
| `task_status` | Reports a thread's project, machine, parent, durable task id, state, queue position, completion flag, latest reply and error. Accepts `wait_seconds`. |
| `task_cancel` | Clears queued work and aborts a child. Only the parent that delegated it may cancel. |
| `task_send` | Sends a correction or follow-up brief to a child. Only its parent may send. Answers a pending input/editor/select question, starts an idle child or queues after its current work. It never approves a confirmation. |

After delegating, a captain can end its turn. When a child settles, fails or is cancelled, Shina sends its parent one completion message with the child's id, title, final status, last reply and any error. Replies are capped at 6,000 characters and errors at 2,000. Results start a new parent turn if idle, queue after current work if busy, or reconnect a disconnected parent before delivery. Multiple results retain separate queue entries. Messages appear as Shina in the transcript, including after restoring Pi history, and never replace the user's composer draft.

Each coordinator can run at most three children and keep at most eight unfinished tasks. Extra delegations stay queued in order. `task_status` reports a one-based `queue_position` for them. Interrupted tasks count toward the unfinished limit until resumed, completed or dismissed.

Call `orchestrator_capabilities` before selecting a delegation model. Pass an exact advertised ID and reasoning level, for example `target: {"provider":"cpa","model":"<advertised ID>","options":{"reasoning":"max"}}` when that model supports `max`. `thinking` is an alias for `reasoning`. Shina rejects unknown models, unsupported levels and unknown target settings before creating a child. It applies the selection and checks Pi's actual state before sending the brief. If Pi substitutes a model or clamps the level, the child fails without receiving the task. Omitting `target` inherits the caller's pinned or current selection, or the catalog default if the caller has no selection. Thread reads and task status include `selection`.

`wait_seconds` is an integer from 0 to 600, default 0. A nonzero value holds the tool reply until completion or the deadline, then returns the current task status. Shina parks these replies without blocking its UI. Completion wake-ups still arrive when a tool waits, so captains should not poll in a loop.

Tool calls run on Shina's UI thread, so they see and change exactly what the window shows. Delegated threads retain their parent in the saved index. Captain crew appears in its captain's view, never in the lone-thread list. The global index now saves each task's brief, model target, state, timestamps, bounded result and completion delivery id. Completion delivery remains pending until Pi accepts it or its authoritative history proves acceptance. Stable `[delivery ...]` headers prevent resending the same result after an acknowledgment or index write is lost.

At launch, Shina reads the child's active Pi history through `get_entries`. A marked brief followed by a proven final reply recovers as completed. Mid-run, uncertain and never-accepted briefs become interrupted. Shina never automatically replays a brief or a tool action, including a delegation that was still queued at shutdown. Recovered results wait for the parent's next user turn or the explicit resume-results action. They do not start a parent turn at boot. The UI query API marks these parents ready for review. User-only resume and dismiss actions let the user continue an interrupted task or cancel it and leave a durable result for the parent. Resume tells the child to check current files and partial actions before finishing, with the original brief as reference.

Pi reaches the server through a small bridge extension, [`assets/pi/shina-mcp-extension.ts`](assets/pi/shina-mcp-extension.ts). Shina writes it to `%LOCALAPPDATA%\Shina\pi` and loads it with `--extension`, passing the endpoint and token in `SHINA_MCP_URL` and `SHINA_MCP_TOKEN`. The extension registers each tool as `mcp__shina__<tool>` and adds a "Shina" section to the system prompt. Other harnesses can connect to the same endpoint with a bearer header. Only Pi needs the bridge.

## Safety and storage

Pi tools can edit files and execute commands with your account's permissions. The composer permissions menu offers Read only, Supervised, Auto-accept edits and Full access. New captain homes default to Read only. Existing homes and threads retain their saved mode, or Full access if no mode was saved. Read only permits built-in read, grep, find, ls and tool discovery, plus all tools registered by Shina's own bridge. It blocks edit, write, bash and other tools without asking for an override, and tells the model to delegate code changes to a thread. Supervised asks before commands and file changes; Auto-accept edits permits built-in edits and asks before other actions. Users can change the mode. Mode changes restart an idle thread's process. Forks inherit their source's mode. Delegated children inherit their parent's mode except that a Read only parent's children use the default worker mode, currently Full access. These are tool approvals, not an OS sandbox or worktree isolation. Trusted extensions run with your account's permissions and are not confined by the tool hook.

Real conversations now load Pi's configured extensions, MCP servers, skills and prompt templates, subject to Pi's normal project trust flow. Shina also explicitly loads its bridge and the existing CPA provider extension when present. Catalog probes remain offline, ephemeral and resource-restricted. Hosts without a CPA extension can use their existing catalog-generated CPA provider in Pi's `models.json`. Pi's enrollment supplies proxy routes, transport metadata, reasoning capabilities and credentials. No model list or credential is copied into this repository. The adapter does not rewrite provider configuration, project trust or authentication. See [Pi integration](docs/pi-integration.md) for behavior, validation and remaining differences from KSG Code.

Pi must already be enrolled in CPA through its provider extension or a configured CPA provider with models in `~/.pi/agent/models.json`. `PI_CODING_AGENT_DIR` can select a different Pi agent directory. Shina reports missing CPA enrollment instead of silently using a direct provider. The workstation's existing `CPA_API_KEY` and local proxy remain outside Shina. Pi inherits Shina's environment, so the key must be present when Shina starts; a key set only in another shell will not reach an already-running app. Opening a picker refreshes catalog metadata after five minutes. Running processes read changed provider definitions when Shina next starts them, or after an app restart.

### Connection errors after moving CPA

Shina's localhost MCP endpoint controls threads. It is separate from Pi's CPA inference connection. An assistant message such as "Connection error." comes from Pi's provider request, not proof that the Shina MCP failed.

Check the CPA provider extension and the local proxy or tunnel on the machine running Shina. A provider URL such as `http://127.0.0.1:8317` refers to that machine. After moving CPA from `linux-server` to the Mac Studio, a client that uses an SSH tunnel must forward to `studio-mac` instead of the old server. The Mac Studio can use its local CPA listener directly. Catalog discovery URLs and inference URLs are separate; a working model picker does not prove inference works.

Keep endpoint and tunnel configuration in the existing CPA/Pi configuration, outside this repository. Shina does not rewrite authentication or replace the provider extension. After updating the host-owned configuration, restart Shina so new Pi processes load it. A missing Pi extension, a Pi process exit, and a provider connection error have different causes; preserve the full error when reporting a failure.

The following JSON layout describes legacy mode. Server mode imports these records into SQLite as described above.

On Windows, legacy mode stores its version 4 global index in `%LOCALAPPDATA%\Shina\index.json`. Its `tasks` array holds durable delegation records. Older indexes without that array still load. Existing crew without a task record starts as interrupted because the old index cannot prove acceptance or completion. First launch checks the per-workspace indexes for every path in legacy `projects.json` and `last-workspace.txt`, plus Still-era indexes. Missing folders migrate only when their index contains threads. Missing folders under the system temp directory are ignored. Verbatim Windows path prefixes are removed from project paths, and equivalent paths share one project. Colliding thread ids and parent links are remapped together. Sessions, pins, archived state, delegation and captain profiles survive. Each captain's most recently active home becomes primary; other homes remain in Earlier conversations, including the homes v0.8 hid. Their projects contribute to the captain's scope. Project captain fields name a workspace default, not the only allowed captain. Shina never deletes or rewrites legacy indexes. A damaged index, task or lifecycle record blocks saving instead of overwriting it with a partial import. Explicit memory and the global user profile live in a separate atomic `memory.json`; damage there also blocks saving.

`run-marker.json` sits beside the index. Shina writes it at startup, refreshes it about every 60 seconds and removes it when a cleanly saved run exits. A leftover marker reports "Shina closed unexpectedly". Without one, saved running tasks report that Shina was closed while tasks were running. Task timestamps use Unix milliseconds. Task records and delivery ids survive restarts, but Pi remains authoritative for accepted briefs and replies. A disk failure or a crash before Pi's new session path reaches the saved index can leave a task interrupted rather than recoverable as completed. Neither recovery nor explicit resume guarantees that repeating a previous tool action is safe.

Version 3 pin, archive and unread booleans migrate to timestamps. A synthetic completion at the saved activity time preserves a legacy unread flag. New unread state derives from `lastCompletedAt > lastVisitedAt`. Lifecycle timestamps use Unix milliseconds inside each record's `lifecycle` object. Runtime `Completed` means a Pi run finished, not that the thread is settled.

Shelf expansion, multi-project scope, sidebar visibility and width, and drafts live in a separate `ui-state.json`. Draft images use small references to deduplicated files in Shina's local `attachments` folder, not inline base64. Display thumbnails are at most 512 pixels per side; Pi receives the original image bytes. Retry, startup cancellation and discard undo preserve the images. Shina reports missing saved draft images instead of silently treating the draft as complete. Image blobs and thumbnails currently remain on disk after removal. Context usage is not persisted and refreshes from Pi on reconnect and provider streaming usage. The details include Pi's session token total and reported cost. Queued user messages and image references survive in UI state. After a restart, Shina checks authoritative history and recovers undelivered messages into the draft rather than automatically replaying them. Unsent thread metadata still uses the global index so existing thread IDs and tabs remain stable. A malformed UI-state file also blocks saving.

Migrated threads use their saved activity time, their Pi session file's modification time, or the legacy index's modification time, in that order. The sidebar hides unknown times. The Captains section shows assigned coordinators only. Its single assignment control lets you choose an unassigned project.

Each thread starts its harness in its project's folder on its owning server. A project-less lone or worker thread uses that server's `Shina/chats/<thread-id>`, isolated from other chats and reused on resume. A general captain uses `Shina/captains/<captain-id>/scratch`. A scoped captain runs in its first workspace and receives the other scoped paths in its context. Projects and threads record a machine id. Backend subprocess dispatch supports only `local` within the owning server. Linked clients send commands to that server; unknown or offline machines never fall back to local execution.

Pi keeps its session files and credentials in its normal locations outside this repository. Drafts survive thread and project switches and app restarts. Run one legacy instance per profile. Multiple `--server-client` desktops share the server-owned profile and keep separate drafts.

Quitting Shina terminates its owned agent and clone process trees. Closing the window normally quits too, except for the opt-in Windows background mode described below. Windows assigns each subprocess to a kill-on-close Job Object before allowing it to run; Unix uses process groups. Pipe writes run separately from process supervision, and shutdown waits have deadlines. Reconnecting to a saved session waits for its previous Pi owner to exit. A stop that gets no settlement forces a disconnect after ten seconds; model and thinking commands time out after thirty seconds.

### Background work, sleep and routines

Settings includes **Keep working in background**, off by default. On Windows, closing a window with running or waiting threads, queued messages or undelivered tasks hides it. The notification-area icon uses Shina's app mark and offers Open Shina, the working-thread count, and Quit. Quit follows the normal save, flush and process-tree shutdown path. Closing an idle window still quits. If work finishes while hidden, the icon remains available with `0 working` until you open or quit Shina. Shina refuses to hide if Windows cannot add the icon.

Rune's `keep_running()` retains the hidden window, runtime and state. Its poll hook and wake deadlines still run without a visible window. This needs no replacement event loop or detached agent processes. Closing to the notification area is Windows-only; macOS and Linux close normally. The setting and keep-awake policy live in `ui-state.json`.

**Keep awake** defaults to Off. While tasks run requests system sleep inhibition only while a thread is actively running, not while it waits for approval. On AC power adds an AC-power requirement and treats unknown power state as not eligible. Windows uses `SetThreadExecutionState` without preventing display sleep. macOS owns a `caffeinate -i -w <pid>` helper and reads IOKit power-source state. Linux uses `systemd-inhibit --no-ask-password` when installed and permitted, with AC state from `/sys/class/power_supply`. Missing helpers or failed requests appear in background status. Helpers terminate when work stops or Shina exits. The UI watchdog checks native input and power state at most 30 seconds apart; Windows power-change broadcasts also wake it.

Windows receives suspend/resume notifications through its hidden notification-area owner. All platforms also compare system and monotonic clocks for a wall-clock gap over 30 seconds. A manual clock change can trigger this fallback. After resume, Shina holds new prompt, queued-turn and routine dispatch until two consecutive TCP connections succeed, two seconds apart, or 120 seconds pass. It uses the CPA URL reported by Pi's model metadata, CPA endpoint environment variables, or Pi's `models.json`. If none is discoverable, it checks `1.1.1.1:443`. A successful TCP connection proves reachability only, not provider authentication or model availability. Existing accepted turns remain owned by the agent. A final provider `Connection error` within two minutes of resume shows Paused (offline) and Retry. Connectivity returning never retries it. Retry is a user action and may repeat tool effects, so review partial work first.

A captain can create a routine when asked through `create_routine`, and manage it with `list_routines`, `pause_routine` and `delete_routine`. Only its home thread can use those tools for its own routines. Creation needs no approval. All routines are listed in Settings with pause/resume and delete actions. Deleting one removes future runs, not a turn Pi already accepted.

Schedules use local time: `{"kind":"every","minutes":15}` or longer intervals, `{"kind":"daily","at":"09:00"}`, and `{"kind":"weekdays","at":"09:00"}`. Calendar schedules choose the first occurrence of a repeated DST time and the first valid minute after a spring-forward gap. Each local date has at most one calendar slot. `missedPolicy` is `skip` by default or `run_once`. A slot over 60 seconds late counts as missed. Run once dispatches one missed slot and advances to the next future slot; it never replays every missed interval. Due work waits for the captain's approval or question to be answered.

Routines pause after two hours without user input and record "Paused while you were away". Windows uses `GetLastInputInfo`; macOS uses the system event-source idle time. They resume after input, subject to the missed policy. Linux idle-input detection is not supported, so routines there do not automatically pause while you are away. One Rune wake deadline targets the earliest eligible scheduled slot. The separate 30-second power/input watchdog does not spin while a schedule is blocked.

`routines.json` stores schedules, slot cursors and active delivery IDs beside the index, through an atomic flushed replacement. A damaged file blocks routine writes. A run reserves its unique routine/slot ID before queuing a Shina-labelled `Routine: <name>` message in the captain home. An idle home starts; an active home receives a queued follow-up. A restart never replays a reserved slot. If Shina crashes between reservation and delivery, or during a run, the routine reports an interruption for review instead of assuming Pi did nothing. This is at-most-once dispatch, not exactly-once tool execution. Routines require a running Shina process, do not install an OS service or wake a sleeping machine, and may consume provider quota.

Failed index writes retry while idle and on flush, even if the state has not changed. Flush waits at most five seconds per store. A persistent disk failure can still prevent saving. File replacement and directory flushing are best effort against power loss, especially on Windows filesystems that do not support directory flushes.

History images import on the RPC reader rather than the UI thread. Validated blobs and previews are reused. The transcript draws at most the latest 32 image previews and labels older omitted previews. Large project icons use cached 64-pixel thumbnails; oversized or invalid icon sources use the project initial. Thread diagnostics retain at most 128 entries, each limited to about 2 KiB.

The loopback MCP transport accepts fixed-length and chunked POST bodies and `Expect: 100-continue`. It limits bodies to 4 MiB, headers to 32 KiB, individual header lines to 8 KiB, concurrent connections and queued calls to 32, and UI request processing to 16 calls per frame. Header reads have a five-second absolute deadline.

## Validation

```powershell
cargo fmt --check
cargo build
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
cargo run -- --demo --scene captain-worker --snap artifacts\shina-captain-worker.png --size 1280x840 --scale 1
cargo run -- --demo --scene captain-attention --snap artifacts\shina-captain-attention.png --size 960x840 --scale 1
cargo run -- --demo --scene captain-brief --snap artifacts\shina-captain-brief.png --size 1280x840 --scale 1
cargo run -- --demo --scene captain-brief-dark --snap artifacts\shina-captain-brief-dark.png --size 960x840 --scale 1
cargo run -- --demo --scene captain-brief --snap artifacts\shina-captain-brief-narrow.png --size 620x720 --scale 1
cargo run -- --demo --scene settings-profile --snap artifacts\shina-settings-profile.png --size 1280x840 --scale 1
cargo run -- --demo --scene settings-menu --snap artifacts\shina-settings-menu.png --size 1280x840 --scale 1
cargo run -- --demo --scene settings-dark --snap artifacts\shina-settings-dark.png --size 1280x840 --scale 1
cargo run -- --demo --scene settings-threads --snap artifacts\shina-settings-threads.png --size 620x720 --scale 1
cargo run -- --demo --scene update-machines --snap artifacts\shina-update-machines.png --size 1280x840 --scale 1
cargo run -- --demo --scene update-machines-dark --snap artifacts\shina-update-machines-dark.png --size 1280x840 --scale 1
cargo run -- --demo --scene harness-picker --snap artifacts\shina-harness-picker.png --size 1280x840 --scale 1
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

## Implementation and limits

`src/ui.rs` draws the interface. `src/model.rs` reduces Pi events and owns global project/thread state and automatic session lifetimes. `src/session.rs` caches provider metadata independently of thread runtimes. `src/backend.rs` dispatches machine hosts and handles local subprocesses and JSONL framing on worker threads. `src/store.rs` migrates legacy data and saves the global index off the render thread. `src/projects.rs` holds project records, project actions and background clones.

Rune is pre-release and pinned to revision `22fcaf167b5e8d74e2ab173a698b7cf2053c5889`. The app batches incoming records, bounds pipe queues, memoizes rendered Markdown, and uses Rune's event-driven redraw behavior. These are design choices, not proof of production performance. Each active thread has the memory cost of its Pi process until idle reclaim.

The earlier layout's release build reported forced at-rest CPU times below 0.15 ms at p95 for a small static example. That measurement predates the crew portrait pass and is not a benchmark of this revised layout, interactive frame rate, or live Pi work.

The full T3 thread-management port is on hold for a separate spec. Snooze and auto-wake, settle/unsettle rules, collapsible and paged settled shelves, drag between sections and undo notices are not implemented in this release. `Status::Settled` still means the Pi run finished, not that the user settled the thread. Future thread-list lifecycle fields must remain separate from process status.

This is an initial desktop prototype. Login UI, git diffs and worktrees remain unfinished. Paired remote clients and cross-server captain delegation are available as described in "Linking machines". Crash recovery resumes persisted Pi history, but cannot recover unsaved provider output or guarantee that retrying a tool action is safe. The transcript currently displays the latest 100 messages and the activity panel displays the latest 50 calls. Older transcript messages remain in Pi's session. Pi RPC records have a 64 MiB limit, so exceptionally image-heavy saved history can require inspection in Pi or a new Shina thread.

The mark is a 32-cell pixel sailboat drawn by [`src/brand.rs`](src/brand.rs). The light theme shows a day sail with a sun, clouds and the three status dots as foam; the dark theme shows a night sail under a moon and stars. The window icon follows the saved theme at launch. The executable icon, favicons and Mac iconset use the night sail because they cannot follow the theme. Exports keep sharp pixels at every size. Regenerate them after changing the mark:

```sh
cargo run --example export-icons -- assets
```

`assets/project-icon.png` is the high-resolution app mark. `assets/favicon.png` and `assets/favicon.ico` are the project/browser favicons. The macOS installer packages `assets/macos/Shina.iconset` into the app bundle.

## Crew artwork

The twelve captain portraits are original characters drawn as 40x40 pixel art by [`tools/crew_originals.py`](tools/crew_originals.py), framed like a Pokémon Mystery Dungeon bust and presented on the square crew tiles Capy uses. See [the artwork notes](assets/crew/README.md).

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
...same six for juniper, sable, pip, bram, wren, rook, nia, jett, otto, bolt and kit
```

A numeric folder (`assets/crew/0/Normal.png`, up to `11`) also works. `SHINA_SPRITES` wins over the workspace folder. All six faces must exist for a captain to use the override; otherwise that captain keeps the embedded artwork. A set made for the original four therefore still works: the eight later captains have no folder in it and keep their embedded art. The loader accepts any square PNG size and the app scales it into tiles from 20 px to 48 px.

A drop-in set with only the six faces keeps static portraits. To animate it, add all four strips beside the faces: `Idle.strip.png` (4 frames), `Working.strip.png` (4), `Attention.strip.png` (4) and `Happy.strip.png` (3). A strip is square frames side by side, and frame 0 should match the static face. The frame order is described in [the artwork notes](assets/crew/README.md). A missing strip or a strip with the wrong frame count leaves that captain static.
