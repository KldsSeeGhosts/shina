# Captain parity implementation

Issue [#17](https://github.com/KldsSeeGhosts/shina/issues/17) is the ordered audit,
not one implementation task. Preserve the existing runtime, durable delivery,
Captain lifecycle, reports and generated digest while implementing its child
issues.

## Publication cadence

Push each reviewed, issue-sized implementation with its regression tests and
open a pull request immediately. Run CI on each slice instead of waiting for
the whole audit. Keep incomplete or rejected candidates out of those pull
requests. Fix CI failures before publishing dependent work as complete.

The existing workflow runs the core gate on pull requests. Merges to main
also build and publish platform updates after the core gate and each
platform's build pass.

## First batch

The first batch starts with independent backend fixes. Implementation and review
are separate. A worker's completion report does not close an issue.

| Issue | Scope | Initial assignment |
| --- | --- | --- |
| [#18](https://github.com/KldsSeeGhosts/shina/issues/18) | Read-only native Claude Captain coordination | Devin SWE-2 |
| [#19](https://github.com/KldsSeeGhosts/shina/issues/19) | Durable local delegation retry identities | Devin SWE-2 |
| [#21](https://github.com/KldsSeeGhosts/shina/issues/21) | Complete authorized remote shared context | Devin SWE-2 |

Review must check the real authenticated tool paths, transaction boundaries,
restart behavior and negative authorization cases. Keep private Captain
instructions out of worker context. Preserve the difference between a saved
operation, provider acceptance and completed work.

### Integration of #19

Reviewed candidate `6901cc6a6a1b0886970ec046d6ce920e079d89ad` and applied it
locally as `548ebdc`. It stores caller-scoped delegation receipts in the same
SQLite transaction as allocation. Canonical intent excludes `wait_seconds`.
Unkeyed calls keep their existing allocation behavior.

The root review added
`local_delegate_mcp_retries_keep_one_worker_after_restart_and_deletion` in
`tests/server_runtime.rs`. This test uses the real authenticated MCP endpoint,
serve processes and fake Pi subprocesses. It checks identical task/thread ids,
changed-intent rejection, one worker prompt, a response-only wait change, and
receipt replay after process restart and worker deletion. The focused test
passes. On the integrated branch, `cargo test` passes with 1,910 unit tests
and 99 server-runtime tests, with 12 opt-in unit tests ignored. `cargo build`
and `cargo fmt --check` also pass. [PR #44](https://github.com/KldsSeeGhosts/shina/pull/44)
passed its core checks and merged at
`c6bcf9783756e9b414709602319f6c502d947bf9`. Main's platform publication runs
separately from the PR gate.

Issue #22 is now assigned to Devin SWE-2 on top of this implementation. It adds
durable send/cancel receipts and attempt fences. Its implementation must retain
the local MCP retry regression.

### First review of #18

Candidate `0473d01520f6992e4de14a44d7f6caca9388cca2` is not integrated.
It admits exact advertised Shina tool names on both native Claude permission
paths and adds role-based capability reporting. Review identified these
remaining checks and corrections:

- A supplied `start.link` and a matching tool name do not independently prove
  the CLI registered that name to the supplied endpoint. Verify same-name MCP
  configuration precedence and registration failures, or fail closed when
  the endpoint binding is uncertain.
- `may_write_captain_documents` must reflect inactive ownership, not just
  primary-home identity.
- Capability and documentation claims must distinguish server authorization
  from each provider's permission prompts. The candidate's statement that
  Shina tools run in every access mode is too broad.

A Devin SWE-2 rework assignment covers endpoint identity, effective rights and
protocol evidence. No frontend change is required for the current access-mode
description.

The reworked implementation `04f61ca9b70e0ebbab1e5480792b9e89905b34fe` now
verifies the CLI's resolved `mcp_status` configuration against the expected
thread endpoint and bearer header. Missing, duplicate or foreign registrations
do not qualify. It retains unrelated MCP servers and reports provider-specific
admission separately from server authority. It also rejects local delegation
from earlier Captain homes.

Root review added a startup-race fix. Exact Shina calls that arrive while
endpoint verification is pending wait for that result instead of receiving a
premature denial. Failure, timeout and cancellation deny held calls. The check
requires a connected HTTP registration, and optional usage failures do not
change the endpoint status. Unit tests cover these cases in addition to the
worker's authenticated dispatch and live serve-process tests. On the integrated
branch, `cargo test` passes with 1,921 unit tests and 100 server-runtime tests,
with 12 opt-in tests ignored. `cargo build`, `cargo fmt --check` and
`git diff --check` also pass. [PR #45](https://github.com/KldsSeeGhosts/shina/pull/45)
passed CI and merged at `aaf3dcc8c5a015fdb5e7e365c2e01335db7e0cb3`.
Release v0.12.50 publishes that exact commit for all three platforms.

### First review of #21

Candidate `14accf5829141bd67838d70941b07099f72580c7` is not integrated.
Its focused unit tests pass according to the worker, but the review found
blocking gaps:

- The origin read handler does not receive the authenticated caller's device.
  It compares a supplied `workerDeviceId` against a saved outbound credential
  identifier. That identifier does not prove who sent the read request.
- The target callback requires an outbound link to the origin. Ordinary
  home-to-target delegation creates only the opposite link, so the supported
  one-way setup cannot read documents through this implementation.
- A page budget smaller than a UTF-8 character can return an empty page with
  the same continuation offset.
- The manifest reports the prefix budget as coverage, although the truncation
  marker consumes part of that budget.

A Devin SWE-2 rework assignment covers these findings, revision/retry behavior,
and a live two-server integration test through the worker's actual MCP path.
The original handler-level tests did not exercise the transport or a foreign
authenticated peer. Do not close #21 based on that candidate.

The rework `6bac14ac8d6b158fe66d22528bea6feedd22e688` replaces the reverse
callback with a parked document request on the existing `peer.thread.wait`
channel. The origin answers through a launching-device-fenced
`peer.captain.document.reply`. It needs no reverse pairing.

Root review added captured runtime generations, immediate denial after peer
revocation, and read-support negotiation on explicit follow-ups for legacy
workers. The follow-up captures support and context together; retries retain
both. The root also added a two-serve-process test using the worker's real
authenticated MCP endpoint. It checks Unicode paging past 8 KiB, omitted
private instructions, exact document content, private and revision denials,
and revocation. The rework's unit tests retain the foreign-device, stale
reply, bounded paging and restart cases. These changes are separate from
the unfinished placement and task-control assignments.

The integrated revision passes 1,935 unit tests and 101 server-runtime tests,
with 12 opt-in tests ignored. Build, formatting and diff checks pass. The
two-process test waits for the fake provider's completed prompt record before
inspecting its brief; this fixes a fixture-readiness race found during review.

The remaining first-milestone work is #20, #22 and #23. Issue #32 covers the
effective cross-harness contract alongside these fixes. The performance track
is #24, then #25, then #26. Captain identity and repository ownership follow in
#27 and #28 before remote crew and the common event inbox in #29 and #30.

The #20 backend assignment uses Devin SWE-2. Its placement and
receipt data must support a separate Claude Opus 5.5 medium UI assignment;
backend completion alone cannot close that issue.

The original rate-limited #20 and #22 assignments resumed in their existing
threads and returned candidate commits. The #20 backend rework adds negotiated
`placement-v1` support at minor 38 and refuses isolated launches on older peers.
Root review adds setup serialization, interrupted-setup recovery, cancellation
checks, pinned base commits, remote upstream-id translation, and a shared reader
using its local parent's dirty worktree without write access. Queued peer
follow-ups no longer duplicate the preceding message during provisioning.

The root's live two-serve-process test proves distinct peer writer checkouts and
branches, stable keyed retries, mirrored placement, and preservation of the
parent's staged and untracked files. Before the final UI corrections the
integrated #20 branch passed 1,962 unit tests and 102 server-runtime tests,
with 12 optional unit tests ignored.

Claude Opus 5.5 medium implemented placement display and crew-draft selection,
with layout tests at 620, 960, 1280 and 1920 pixels. Its separate protocol patch
was not applied because the backend already advertises the contract at the
correct negotiated minor. Review required correct labels for shared readers in
a parent's worktree and access to the base-branch picker in narrow crew drafts.
The first Opus follow-up exhausted Devin's
daily quota before making edits. At the user's direction, its replacement uses
Claude Code with Opus 5.5 at medium effort. Future design and frontend work uses
that provider; SWE-2 implementation stays on Devin. Both old worktrees remain
untouched. No plan or authentication changes were made.

Claude Code commit `b0dd03f`, integrated as `d4c3529`, fixes both findings.
The lock icon and label use the same read-only predicate as the backend.
The branch selector remains in the footer when machine and workspace controls
move into overflow, matching KSG's `BranchToolbar`. Headless tests exercise
pointer and keyboard selection, errors and bounds at all four supported widths.
The worker's full suite passes 1,964 unit tests and 102 server-runtime tests,
with 12 optional tests ignored. Root review inspected the actual 620 and 1280
pixel picker renders. The initial PR #47 head passed CI; the updated UI head
must pass again before merge.

PR #47 merged at `63c820ba7d6e847a34eeb221a7a7a6c07f593d2e` after the final
UI head passed CI. Its main release runs separately.

Issue #22's mandatory cancellation and question-answer fences are in PR #48.
Root review adds receipt-transaction preflight, atomic send/cancel failure
tests, mandatory question fences for legacy children, and refusal of fenced
controls when a legacy peer worker has no recorded attempt. The saved receipt
explicitly distinguishes saved intent from provider acceptance and completed
work. Protocol minor 39 adds task fences while retaining placement at minor 38
and peer document reads at minor 37.

Integration with #20 retains pending-placement follow-up ordering and retry
reservations. A new attempt after cancellation cannot requeue the cancelled
brief while its checkout is still pending; the placement regression now tests
that transition through setup and provider startup. The combined branch passes
1,978 unit tests and 105 server-runtime tests, with 12 optional tests ignored.
Build, formatting and diff checks pass. PR #48 still requires fresh exact-head
CI after this merge. Do not merge it while #47's main release is still running.

Any frontend implementation or design decisions are assigned to Claude Opus
5.5 medium. KSG Code remains the layout and interaction reference, with Shina's
Pixel Voyage skin. Capy observations below are behavioral evidence, not
permission to replace that design contract.

## Capy package investigation

Inspected on November 16, 2026 without installing or launching Capy. The official
download page redirected its Apple Silicon download to
`https://downloads.capy.ai/stable/Capy-0.9.0-arm64.dmg`.

| Artifact | SHA-256 |
| --- | --- |
| DMG | `2272839106531333f96b2b01a88a88844653d168a9d8a77dec8bda7b47f0e860` |
| `Contents/Resources/app.asar` | `1db9307cf212f0eb34a427c7bf228778831ae6619318f8dabd59c1890b372a6d` |
| `Contents/Resources/renderer` tree, REA identity | `f80eca2dd9d220f4bc7e8eb1616b7f7bbe78750b0025626d651609f92446f2d5` |

REA inventoried the DMG and extracted the ASAR. Its DMG extraction operation
did not support this format, so the package was mounted read-only for access to
the ASAR and renderer. No target code ran.

### Finding ledger

These are observations of shipped code. They do not establish live backend
enforcement, delivery guarantees or execution behavior.

| Question | Static observation | Status and relevance |
| --- | --- | --- |
| Does the Captain have a scoped issue view? | `assets/captain-issues-DMBjr7bS.js` exports `CaptainIssuesView`. It passes `jamId` as the issue scope and offers All, Assigned to me, Mentioned, search and state filters. | Answered for this client. Supports #31 and #41. |
| Are instructions scoped beyond one checkout? | `assets/agent-scopes-CKtrkhLA.js` constructs organization, project, user, project-user and Captain instruction scopes. A Captain entry uses `captainJamId`, addresses the Captain and its crew, and computes editor read-only state from `card.userId` versus the current user. | Answered for scope construction. Backend authorization and instruction precedence remain unresolved. Relevant to #37 and #43. |
| Are Captain files addressed independently of a local path? | `assets/drive_.captain._jamId-BjIaGuSs.js` builds a Drive mount with `kind: "captain"` and `jamId`. | Answered for client addressing. Storage, access checks and replication remain unresolved. Relevant to #36. |
| Does a task view distinguish a root from a descendant? | `assets/task-view-CuRKX8iU.js` accepts `rootJamId`, `jamId` and `subtree`, requests missing subtree data, tracks older transcript pages and builds transcript rows with `deliveryTool: "message_agent"`. | Answered for client inputs and row construction. Recursive ownership and message delivery semantics remain unresolved. Relevant to #33, #40 and #41. |
| Are concurrency and nesting represented? | ASAR `out/main/main.js` includes `Organization` schema fields `taskConcurrentLimit` and `taskNestedDepth`. | Answered for schema presence only. Actual scheduler enforcement remains unresolved. Relevant to #23 and #33. |

REA evidence:

- DMG inspection parent
  `ev_69ef724a42b5ed8050327ce0cf6b8fbab1e40f7bcba6da096b55142c77e7ce35`.
- ASAR application analysis parent
  `ev_618b9f57281bd8fe87e5ffad322ac7673923bac4b5afb15c52d2c673becf6601`.
- ASAR extraction
  `ev_11b8117f3a89da18f1305800833d37d4590a5f9b04274223f8695e643d08df82`.
- Renderer application analysis parent
  `ev_e4db6e4984b9a63d18385a84870f281a8cae5ebadd42081d7c854e0844687005`.
- Exact Drive asset identity view
  `ev_d7d6fd2dd9c7cee01633c54e7bc100dddb97524b9e5e23bb9a1cbab74deb552a`.

Renderer analysis selected the four files named above. It left 997 inventoried
files outside analysis scope. The ASAR semantic graph reached its configured
limits. Neither analysis proves complete feature coverage. Direct inspection
of the extracted source establishes the field and function observations in the
ledger. Do not infer a server implementation from these client contracts.

The complete ten-record evidence bundle was exported outside the repository.
Keep proprietary package bytes and the large evidence bundle out of Git.

## Verification baseline

At commit `c710261`, before integration of this batch, `cargo build`,
`cargo fmt --check` and the full `cargo test` suite passed on the local Apple
Silicon Mac. The test run included 98 passing server-runtime integration tests.
Paid provider smoke tests remain opt-in. Re-run the required checks after
integrating implementation commits; this baseline is not verification of those
changes.
