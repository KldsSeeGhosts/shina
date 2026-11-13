# Review backend

Shina ports KSG Code's three review commands used by the diff panel: `review.getBranchCommits`, `review.getDiffPreview`, and `review.getDiffFileContents`. The wire format matches `packages/contracts/src/review.ts` field for field, including source kinds, optional fields, and casing. The implementation lives in `src/review.rs`; `src/server.rs` dispatches the commands off the reducer thread inside `tokio::task::spawn_blocking`, the same pattern `vcs.status` uses.

## Function map

| KSG function | Shina function | Notes |
| --- | --- | --- |
| `GitVcsDriverCore.getReviewBranchCommits` | `review::get_branch_commits` (`get_branch_commits_impl`) | Same `git log -z --no-notes --no-show-signature --format=%H%x00%s%x00%cI --max-count=101 mergeBase..HEAD` invocation, merge-base resolution through `resolve_review_merge_base`, subject cap of 300 bytes, commit cap of 100, `truncated` flag. |
| `GitVcsDriverCore.getReviewDiffPreview` | `review::get_diff_preview` (`get_diff_preview_impl`) | Commit, working-tree, and branch-range sources. Same `git diff --numstat -z` / `--patch` invocations, `--ignore-all-space`/`--minimal` flags, pathspecs, metadata-only mode, 120,000-byte patch cap, 16 MiB metadata cap, 5,000-file untracked cap, `diffHash`, `generatedAt`, `truncated`, and `patchesDeferred`. |
| `GitVcsDriverCore.readReviewFileAtRevision` | `review::read_file_at_revision` | `git show --end-of-options REV:path` with NUL rejection as binary and the 1 MiB cap. |
| `GitVcsDriverCore.readWorkingTreeReviewFile` | `review::read_working_tree_file` | Lexical containment plus canonicalization against the worktree root, so `..` and symlinks escaping the repository are rejected before any read. |
| `GitVcsDriverCore.getReviewDiffFileContents` | `review::get_diff_file_contents` (`get_diff_file_contents_impl`) | Resolves the repository, reads old/new sides at the revision or in the working tree per `sourceKind`, and returns `ReviewDiffFileContentsResult`. |
| `GitVcsDriverCore.prepareUntrackedReviewIndex` / `prepareReviewIndex` | `review::prepare_untracked_index` / `review::prepare_review_index` | Disposable `GIT_INDEX_FILE` with `read-tree --empty`, `update-index --no-split-index`, and `add --intent-to-add --pathspec-from-file=- --pathspec-file-nul` so untracked files appear in the working-tree diff. Large and binary untracked files follow KSG's rule. |
| `resolveRepositoryPathsUncached` / `readWorktreeRoot` / `readGitCommonDir` | `review::resolve_repository_paths` / `read_worktree_root` / `read_git_common_dir` | Repository detection including worktrees with a split `.git` directory. Unborn repositories resolve `HEAD` to the empty tree hash (`read_empty_tree_hash`). |
| `isServerInternalPath` | `review::is_server_internal_path` | Rejects Shina state files (`index.json`, `server-inputs.json`, `ui-state.json`, `memory.json`, `routines.json`, `honcho-outbox.json`) under the state directory. |
| `ReviewService` validation and error mapping | `server.rs` dispatch, `review::RunError` | Cwd checks, input decode, and error wording in the dispatch layer. |
| Workspace authorization (`serverInternalPaths.ts`, workspace service) | `review::resolve_workspace_cwd` | First tries `workspace_search::resolve_cwd` against snapshot project roots, then accepts a registered worktree of any project root. Anything else returns "Workspace path is not inside a project on this server." |
| `RpcAuthorization` grant for the three commands | `protocol::method_grant` → `"read"` | Authorized before and after the blocking git call, matching the existing vcs pattern. |

## Differences

- KSG's `executeGit` buffers stdout in memory. Shina's `run_git` drains the pipe and truncates at the same byte caps, so over-cap reads cannot grow without bound; each KSG truncation flag is still produced.
- KSG relies on a single-version deployment, so it has no capability gate. Shina adds `review-diff-v1` at `REVIEW_DIFF_MINOR` (26) in `src/protocol.rs`. Peers negotiated below that minor get `REVIEW_DIFF_UPDATE_REQUIRED`, and the client refuses locally through `client::supports_review_diff`, as for any unknown command.
- The internal-path denylist names Shina's state files rather than KSG's, since the directories differ.
- Error strings keep KSG's wording with "Shina" substituted for the product name.
