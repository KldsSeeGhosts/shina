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

## Remaining

Ported behavior still missing, with the KSG source that owns it:

- Turn and tool-call diff sources, and the per-turn file list — `apps/web/src/components/DiffPanel.tsx`.
- Commit list and the commits menu — `apps/web/src/components/DiffPanel.tsx` (`review.getBranchCommits`).
- Branch base picker — `apps/web/src/components/DiffPanel.tsx` (`selectGitScope`'s chosen-base path).
- File tree sidebar — `apps/web/src/components/diffs/diffFileTree.logic.ts`; `src/review_tree.rs` already holds the `TreeEntry` builder and sorts.
- File explorer hand-off — `apps/web/src/components/DiffPanel.tsx`.
- Diff annotations and comments — `apps/web/src/components/diffs/`.
- Syntax colouring and word-level highlights inside line rows — `apps/web/src/lib/diffRendering.ts`.
- `fnv1a32` (`src/review_patch.rs`) stays unused until the remaining port needs patch hashing.
