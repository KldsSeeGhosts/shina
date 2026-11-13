# Diff panel

The right panel's Diff tab shows the thread's project diff, ported from KSG
Code's `DiffPanel.tsx`. `src/diff_panel.rs` holds the per-thread state, the
requests to the owning machine, the background parsing, and the view; the state
hooks live in `src/model.rs` and the remote dispatch in `src/client_state.rs`.

## Scopes

Two scopes, matching KSG's git toggle: Uncommitted sends a `WorkingTree`
preview (working tree against HEAD, untracked files included), Branch sends a
`BranchRange` preview with `baseRef: null`, which resolves against the merge
base — KSG's default when the user has not picked a base. The default scope is
Branch, as in KSG's `diffPanelStore`.

## Requests

`review.getDiffPreview` runs with `metadataOnly: true`, like KSG, so the patch
arrives as file stats and per-file patches load through
`review.getDiffFileContents` as `virtual_rows` brings them near the viewport.
Local threads run `review::get_diff_preview` on a worker thread; linked-machine
threads queue an RPC through the client's `linked` commands and dispatch on the
`diff:{request}` tag. A newer request bumps `ThreadDiff::request`, so a late
answer to a replaced request is dropped. Machines without `review-diff-v1`
show the capability notice instead of loading.

Refresh follows KSG's `useLiveRefresh`: a refresh is due on tab mount, scope
change, the refresh button, and after a provider turn finishes or a tool call
reports a file change, limited by `LIVE_REFRESH_MIN_INTERVAL_MS` (three
seconds). No request runs while the tab is hidden.

## Rows

The file list, hunk headers, line rows, markers and the batch boundary flatten
into `Ready::rows` once per data change and draw through `virtual_rows`, so a
large patch builds only visible rows. Collapse state keys on
`review_patch::identity_key`, kept per scope for the session; the header's
collapse-all uses `review_patch::toggle_all`. Hide-tests filters shown files
through `review_tree::is_test_changed_file` and persists as `diffHideTests` in
the UI snapshot, only when on.

## Entry points

KSG keeps the Diff tab's controls in the header and its changed-file rows in
the timeline; Shina matches:

- `header-diff` in the chat header toggles the Diff tab
  (`State::right_panel_toggle_diff`, KSG's `onToggleDiff`). Beside it,
  `header-diff-stat` reads the loaded diff's `+N -N` totals through
  `diff_stat_label`; it hides below the narrow fold and never starts a request.
- `header-right-panel` toggles the whole right panel
  (`State::right_panel_toggle`, KSG's `toggleRightPanel`); it hides when the
  window is compact.
- A provider file-change card's path (`tool-file-path-*` in `work_ui.rs`)
  presses through `diff_panel::reveal_file`: it opens the Diff tab on the
  Uncommitted scope, expands the file if folded, and scrolls its header row to
  the list's top. While the preview is loading the path waits as
  `ThreadDiff::pending_reveal` and applies when the result arrives; a path no
  diff file matches still opens the tab and changes nothing else.
  `file_path_matches` compares the timeline path relative to the preview's
  project directory, folding absolute paths inside it and matching a rename's
  old or new path.

## File tree

`src/diff_tree.rs` ports KSG's `DiffFileTree.tsx` (mounted in the
`FileExplorerPane` aside, right of the diff list). Where KSG feeds Pierre's
`FileTree`, Shina flattens the same model — shown files deduped by path, with
conflicting statuses folding to `Modified`, plus their ancestor directories in
first-seen order — into `Ready::tree_rows`, drawn through `virtual_rows` at
KSG's compact 24 px row height. Single-child directory chains show as one
`a / b / c` row keyed on the deepest directory, folder presses fold the chain
as one node, and `dirs_collapsed` is per thread, session only.

Pressing a file calls `diff_panel::apply_reveal` — the same reveal the
timeline rows take — and its selection sync opens the file's ancestor chain
and scrolls the tree row into view. The diff list's first visible row drives
the current file the other way: `diff_note_visible` marks the file whose
section tops the viewport with `p.selected` and a 2 px coral bar, except while
a reveal is in flight, which owns the position as KSG's
`useCodeViewFileReveal` does. Arrow keys move a tree cursor on `diff-tree`;
Left folds or goes to the parent, Right opens, Enter or Space activates.

KSG's `useResizableWidth` bounds the pane to 160–560 px with a 40%/256 px
default against a 256 px diff reserve; `diff-tree-resize` drags and keys the
same way from the tree's left edge and the width persists as `diffTreeWidth`.
The header's `diff-tree-toggle` (the `folder` glyph — the skin has no
folder-tree drawing) saves as `diffTreeOpen` only when on, matching KSG's
hidden default; KSG never names a hide threshold, so the tree and its toggle
disappear below the lane its own bounds imply (160 + 256 = 416 px).

## Remaining

Ported behavior still missing, with the KSG source that owns it:

- Turn and tool-call diff sources, and the per-turn file list — `apps/web/src/components/DiffPanel.tsx`. KSG's timeline rows open a per-turn diff (`onOpenTurnDiff` selects the turn's checkpoint); Shina's `reveal_file` opens the working-tree diff instead, because turn checkpoints are not ported yet.
- Commit list and the commits menu — `apps/web/src/components/DiffPanel.tsx` (`review.getBranchCommits`).
- Branch base picker — `apps/web/src/components/DiffPanel.tsx` (`selectGitScope`'s chosen-base path).
- File explorer hand-off — `apps/web/src/components/DiffPanel.tsx`.
- Diff annotations and comments — `apps/web/src/components/diffs/`.
- Per-line syntax colouring by file extension is done; word-level change highlights and multi-line constructs (coloured line by line) remain — `apps/web/src/lib/diffRendering.ts`.
- `fnv1a32` (`src/review_patch.rs`) stays unused until the remaining port needs patch hashing.
