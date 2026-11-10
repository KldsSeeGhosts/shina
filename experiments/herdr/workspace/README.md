# Shina on Herdr

This is the live native workspace. It combines a pinned Herdr Chat backend,
the Herdr T3 layout, and Shina's presentation and interaction changes. The old
`shina-native` mod remains a separate design demo with canned responses.

## Open

```sh
node experiments/herdr/workspace.mjs open
```

The persistent runtime is
`~/Library/Application Support/ShinaHerdr/`. Its app profile, core config,
core state, source checkouts, font tools, and backups live there. The core session
still has the internal name `shina-prototype`; changing that would select a
different core rather than rename this workspace.

The launcher opens the private Herdr app and keeps the last selected thread.
It does not use an embedded browser. Conversation views are GPUI mods.

## What is connected

- Native Pi, Claude Code, and Codex launchers. A live Pi conversation through
  the existing CPA configuration has been verified on this machine. Claude
  and Codex adapters are included, but live prompts through them have not
  been verified in this pass.
- Streaming Markdown, code, tool cards and diffs, attachments, provider-specific
  approvals, model and reasoning controls, session resume and fork, and steer
  and follow-up queues come from Herdr Chat. Its fixture suite exercises these
  protocols. This is not a claim that every control received live end-to-end QA.
- T3's Inbox and Lists, pinning, search, working/snoozed/settled shelves,
  settle undo, navigation, tab ordering, actions, and close confirmations.
  Snooze and settle return Home rather than opening another thread.
- Each space has its own default coding agent. Use `Default · Pi` in the top
  toolbar to choose Pi, Claude Code, or Codex for future threads in that space.
  Choosing an agent in another space does not change this preference.
- The composer has separate coding-agent and model controls. Its agent picker
  changes an untouched thread without discarding its draft or attachments.
  It creates and identifies the replacement before closing the old empty tab.
  A thread with history keeps its agent; choosing another opens a new thread
  and copies only the unsent draft. Conversation history is not transferred
  between agent formats. Busy, split, and subagent threads are never replaced.
- Command-I opens an explicit new-thread agent picker. A one-off choice does not
  change the space default. Command-Shift-N inside chat opens another thread
  with the current thread's agent.
- Shina's palette, rounded composer, Sans UI, Serif answers, crew portraits,
  and native motion. Machine headers in Lists use crew portraits as machine
  identities, not autonomous coordinators. `t3-spaces` layout props can set
  `machineCaptains`, keyed by machine name or `local`.
- Per-chat persisted drafts, including image paths. Reloading the mod or
  restarting the window retains them. A failed send retains its draft.
  An acknowledged send clears only an unchanged draft, not newer edits.
- Native Files and Changes panels. Opening one hides the other. Changes
  shows the current worktree's diffs. Its inherited Commit button stages
  all changes before committing. This implementation did not stage or commit
  repository changes.
- Herdr Chat hosts survive closing the app window. Closing their chat tab
  ends the host; session files remain with the underlying agent.
- Private core templates `shina-pi-chat`, `shina-claude-chat`, and
  `shina-codex-chat`. These support explicitly requested child tasks.
  Native chat subagent rows open their chat rather than a terminal preview.
  Installation does not modify global agent instructions or start subagents.

## Keys

| Key | Action |
| --- | --- |
| Command-T | New thread in the current project, or choose a project from Home |
| Command-I | Choose an agent for a new thread |
| Command-Option-A | Choose the default coding agent for this space |
| Command-Shift-N | New chat with the same agent while in chat |
| Command-P | Find a thread or pane |
| Command-Shift-B | Switch Inbox and Lists |
| Command-Shift-S | Settle the current idle thread, or bring it back |
| Command-Shift-H | Go Home |
| Command-[ and Command-] | Previous and next agent |
| Command-Option-T | Open a terminal |
| Command-Option-Enter | Continue chat in the agent's terminal UI |
| Command-Shift-E | Toggle Files |
| Command-Shift-D | Toggle Changes |
| Command-End | Scroll chat to the latest output |
| Command-L | Focus the composer |
| Enter | Send, or steer while running |
| Option-Enter | Queue a follow-up |
| Shift-Enter | New line |
| Escape | Stop or close the current chat menu |

Chat-specific custom key overrides remain in the private profile's
`data/herdr-chat/keys.json`. They take precedence over the supplied defaults.

## Edit and deploy

```sh
node experiments/herdr/workspace.mjs install
node experiments/herdr/workspace.mjs check
node --test experiments/herdr/tests/*.test.mjs
node experiments/herdr/workspace-smoke.mjs --restart
node experiments/herdr/agent-smoke.mjs
```

`install` pins both source repositories to `upstreams.json`, installs Herdr
Chat's dependencies with package scripts disabled, checks staged mods, and copies
the adapted files into the private profile. It refuses a dirty or different
upstream commit. Make changes here, not in the source checkouts.
`customize.mjs` refuses patches when expected upstream code has changed.

Neither upstream installer runs. They can modify global instructions,
Pi extensions, or other source checkouts. Shina instead writes only to its
private runtime, including its own core templates and `bin/hc` link.
Provider credentials and settings stay with the existing CLIs.

The font step uses a private Python virtual environment and pinned FontTools
4.60.1 to generate static instances from fonts already installed on this Mac.
Herdr's variable-font lookup selected the wrong weight in visual testing.
The generated fonts stay in the private profile. The repository includes no
Anthropic font binaries and installs nothing into macOS's font directories.
See [third-party notices](THIRD_PARTY.md).

The installer records the source pins and an exact backup path in
`profile/shina-workspace-install.json`. It backs up existing affected keys,
layout, and mods before deployment. Conversation data is separate.
To return to Herdr's standard layout without deleting work:

```sh
node experiments/herdr/run.mjs do layout.switch '{"name":"main"}'
```

## Verification and limits

The native smoke check creates and closes only its own test tabs. It sends no
model prompts. It checks hidden-host attachment, draft isolation, mod reload,
model-picker opening, snooze, settlement, undo, theme switching, and optional
window restart with the same surviving host process. These are native mod
commands, not physical mouse or keyboard input.

The agent smoke check uses its own temporary QA spaces and labeled example
history. It checks independent space defaults, default-based thread creation,
agent changes with draft/image transfer, and preservation of existing history.
It sends no model prompts and does not change a real space's default.

Herdr Chat's source fixture suite passed 139 tests with seven opt-in live tests
skipped. Those tests cover the upstream backend, not every Shina UI change.

Remaining differences from T3 and standalone Shina:

- Remote machines have real Herdr spaces and terminal workflows, but native
  structured remote chat is not connected. The private core currently needs
  its own machine connections. No machine enrollment or authentication changed.
- The mobile app does not automatically render these desktop mods. Mobile
  access to this private core has not been tested.
- Shina's autonomous captains, MCP delegation workflow, scheduled wake-ups,
  and existing conversation data have not migrated.
- Native T3 motion remains. This is not a frame-for-frame port of Rune's
  animations. Reduced-motion behavior across the upstream mods is unverified.
- No global Pi session-lock extension was installed. Do not manually open a
  chat's live Pi session in another process. Use the chat's `/tui` handoff.
- Closed-chat draft records stay in the private profile. Draft garbage
  collection and a recovered-drafts interface are not implemented.
- The pinned integration targets Herdr 0.1.14 and core 0.1.19 on macOS.
  Review and test upstream updates before changing the pins.

Pi currently shows an offline cached-catalog refresh warning that includes
HTTP 502. A real model request succeeded despite that warning. This work did
not change CPA's service, authentication, or model catalog.
