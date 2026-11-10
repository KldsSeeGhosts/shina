# Shina's native Herdr work

The [live Shina workspace](workspace/README.md) now combines Herdr Chat with
the T3 layout. Start that version with:

```sh
node experiments/herdr/workspace.mjs open
```

The sections below document the earlier standalone visual prototype. Its
`shina-native` composer still uses canned replies and is separate from the
live workspace's `chat` mod.

## Earlier visual prototype

This ports Shina's composer, sidebar, portraits, and typography into Herdr's native GPUI mod framework. It is an interactive design prototype, not a complete Shina replacement.

The interface uses JavaScript with `gpui-kit`, `gpui-base`, and Herdr's public API. Herdr owns the actual spaces, tabs, terminal processes, and window. There is no embedded browser or Rune window.

## Open the prototype

From the Shina repository:

```sh
node experiments/herdr/run.mjs open
node experiments/herdr/run.mjs init
```

`init` reuses the prototype space and an existing chat tab. Run `open` through this launcher, not by opening the private app bundle directly. The launcher supplies the separate profile and core session.

The persistent runtime is at:

```text
~/Library/Application Support/ShinaHerdr/
```

It contains the signed app and core under `target/`, the app profile under `profile/`, and separate core config and state. The core session is `shina-prototype`. Runtime sockets use `/tmp/shina-herdr-<uid>` because Unix socket paths must stay short. The prototype itself and its saved data are not temporary.

This Mac has Herdr 0.1.14 and herdr-core 0.1.19. Their archives matched the release manifests, and the desktop app passed strict signature verification for Herdr's Apple team `2W9652Q7QR`. The launcher does not install or upgrade software. To use existing binaries elsewhere, set `SHINA_HERDR_BIN` and `SHINA_HERDR_CORE`; `SHINA_HERDR_HOME` changes the private runtime directory. The default paths currently target macOS.

The first test location was under `~/.local/share`, which points to DevDrive on this host. Processes stalled in filesystem and socket operations there. The active runtime uses the internal Application Support directory instead. No macOS privacy grants or unrelated services were changed.

## What works

- Real Herdr tabs, a Shina-style tab strip, and a sidebar showing the active space's tabs.
- Captain portraits and role labels. Captain buttons open tagged chat threads. They are not yet autonomous coordinators.
- A retained native multiline composer, local message submission, and selectable Markdown replies.
- Separate drafts and conversation state per thread, including after a mod reload or app restart.
- Native tab settlement, sidebar visibility, and light/dark theme switching.
- A disclosure section with a native 220 ms GPUI spring. This demonstrates native motion, not complete parity with Rune's animations or reduced-motion behavior.
- Real terminal tabs through Herdr. The initial Shell tab belongs to this prototype's core session.
- A read-only Pi/CPA metadata probe. It successfully read the configured model and 14 CPA models on this machine without submitting a prompt.

The supplied conversation and local replies say they are examples. The send button produces a local prototype response. Finding Pi does not switch the composer into live AI mode.

The frontend-design pass preserves the current Shina palette and type roles. Installed Anthropic Sans and Serif supply the UI and reading fonts on this Mac. The setup also copies Shina's open-licensed font assets into the private profile, but automatic font fallback on other machines has not been verified. No proprietary font files are bundled in the repository.

## Pi probe safety

The native mod runs `pi-probe.mjs` through Herdr's `exec` API. The helper starts the existing Pi CLI with offline RPC, no session, and unrelated extensions, MCP, skills, and prompt templates disabled. It loads only the existing CPA enrollment extension when present.

Its fixed RPC request list is:

```text
get_state
get_available_models
```

It sends no prompt, writes no Pi setting, and refuses a selected non-CPA provider. A subprocess timeout and an 8 MiB response limit bound discovery. Provider stderr and raw authentication errors are not displayed. An authentication denial requires manual repair of Pi's existing enrollment; the mod disables another probe in that copy.

Credentials remain with Pi outside this repository. Herdr telemetry is disabled in this prototype's settings and launch environment.

## Edit and verify

Edit the source under this directory, then deploy and check it:

```sh
node --test experiments/herdr/tests/*.test.mjs
node experiments/herdr/run.mjs check
node experiments/herdr/smoke.mjs
node experiments/herdr/run.mjs status
```

`check` copies the source into the private profile and runs Herdr's native checker. Herdr hot-reloads the copy. Do not edit the generated `AGENTS.md` or type definitions in the profile.

The smoke check requires an open prototype. It creates two test tabs, tests native textarea focus and the composer submit path, switches between drafts, reloads both mod copies, settles a tab, and toggles the theme. It closes only its own test tabs and restores the previous tab. It sends no AI request. This tests the native control APIs, not synthetic operating-system keystrokes.

One lifecycle detail matters: a mod can initialize before Herdr publishes the new focused tab. Thread state therefore uses a stable `threadId` in the pane props. The mod associates that state with a Herdr tab during an interaction, not during construction. Custom mod commands reach only shown copies.

## Screenshots and example controls

```sh
node experiments/herdr/run.mjs do shina-native.example
node experiments/herdr/run.mjs do shina-native.toggle-work
node experiments/herdr/run.mjs shot /absolute/path/shina-herdr.png
node experiments/herdr/run.mjs do shina-native.theme
node experiments/herdr/run.mjs do shina-native.probe-pi
```

`shot` captures Herdr's native rendered frame. The prototype was visually inspected at 1280×900 and 620×720, with the sidebar both open and hidden. The sidebar toggle is manual in this pass.

## Remaining migration work

The next gating task is a long-lived structured Pi transport. The current probe proves process launch and JSON RPC metadata, not a streamed conversation.

Pi streaming, tool events, approval dialogs, image attachments, model/reasoning changes, session-history restoration, and Shina's MCP delegation and captain wake-ups are not connected. Snooze, grouped undo, bulk selection, and drag/reorder behavior also remain in the standalone Shina app. Closed-tab mod data is not yet garbage-collected.

Herdr's mobile app connects to herdr-core terminals. This custom desktop chat mod does not automatically become a mobile interface, and mobile access to this separate core session has not been tested.

The original Rune implementation remains unchanged by this experiment.

## References

- [Herdr desktop download](https://herdr.dev/download/)
- [Native layout and mod guide](https://github.com/herdrdev/herdr-app/blob/master/defaults/AGENTS.md)
- [Herdr security and privacy](https://github.com/herdrdev/herdr-app/blob/master/docs/security.md)
- [Herdr mobile behavior](https://herdr.dev/mobile/)

The installed profile's generated API types are authoritative for this prototype's version.
