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
pixel picker renders. Both the initial and final PR #47 heads passed CI.

PR #47 merged at `63c820ba7d6e847a34eeb221a7a7a6c07f593d2e` after the final
UI head passed CI. Release `v0.12.57` published that exact source commit.
The core gate, three platform builds and publish jobs passed, and the actual
tag target, manifest and downloaded asset hashes were verified.

The #20 acceptance follow-up found two recovery defects. A direct send to a
failed-placement worker could mark it busy without starting a provider, which
then blocked its parent's retry. A Captain crew send also rejected the failed
placement's recoverable queue hold. Direct sends now refuse before starting or
marking the worker busy. New parent or Captain work can retry the same
reservation, while other interrupted-input guards remain enforced and the
original crew draft remains available for review.

The follow-up adds real-Git cleanup tests for dirty tracked, staged and untracked
files, and preserves unmerged branches and commits after clean checkout removal.
The live two-server fixture now selects an existing worktree and launches a
dependent writer at an upstream-only commit. Home and executor numeric thread
ids deliberately differ, so the mapping cannot pass by coincidence. A separate
two-state peer-command test advances the upstream branch after reservation and
proves provisioning still uses the captured commit. These tests supplement the
published placement coverage rather than replacing it. Issue #20 remains open
until this follow-up passes CI and publication.

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
Build, formatting and diff checks pass. PR #48's integrated head passed CI and
merged as `269a2d022539907f839eebd7989ff21d2134766a`. Release `v0.12.59`
publishes that exact source commit. Its core gate, all platform builds and
publish jobs passed; the non-draft release, tag target, checksum manifest and
all three downloaded asset hashes were verified. The still-open #20 and #22
acceptance criteria are being checked separately before closing those issues.

The next implementation batch is #23, #32 and #24 on separate Devin SWE-2
worktrees. Claude Code Opus 5.5 medium prepared the #23 UI source mapping and
will implement against the committed backend contract. DeepSeek V4.1 Flash at
maximum reasoning through Codex is reviewing #20/#22 acceptance and preparing
the #27/#28 migration source map. No worker changes provider authentication or
production CPA configuration.

### Repository identity prerequisite for #28

Root regression tests reproduced four normalization defects: repeated `.git`
suffix removal conflated distinct names, an `@` in an SCP-style path was parsed
as credentials, SCP-style IPv6 hosts were split at their first colon, and URL
queries/fragments were ignored. The correction preserves ordinary SSH/HTTPS
identity keys, strips one suffix, parses the authority separately, preserves
IPv6 and non-default ports, and rejects unsupported URL components rather than
discarding them. The existing live peer delegation/restart test now uses
equivalent SSH and HTTPS origins with an `@` and `.git` in the repository name.
All five new identity tests pass. The complete branch passes 1,983 unit tests
and 105 server-runtime tests, with 12 optional tests ignored.
This is a prerequisite fix, not completion of #28's logical repository catalog,
checkout provisioning or identity migration.

Any frontend implementation or design decisions are assigned to Claude Opus
5.5 medium. KSG Code remains the layout and interaction reference, with Shina's
Pixel Voyage skin. Capy observations below are behavioral evidence, not
permission to replace that design contract.

### Identity storage prerequisite for #27

Candidate `7c26012` adds a schema 8 sidecar and index version 5 with additive
workspace, owner and logical record IDs. Root integration preserves the
original worktree and keeps the candidate's changes separate from review fixes.
This is not completion of #27. Execution leases, generation fencing and
executor changes still need implementation.

The independent review reproduced a future-version overwrite in the SQLite
path and public record enumeration through pairing discovery. It also found
that file-only migration wrote before full model validation. Root regressions
reproduce these gaps before the corrections. Public discovery now retains only
its existing bootstrap fields; authenticated `hello` retains the domain maps.
SQLite rejects future index versions without dropping unknown fields.
File-only migration uses the model's record, history and queued-input validators
before writing, without starting a runtime or opening stores.

Further checks reject distinct canonical checkouts sharing an ID and a changed
controller `server-id`. Migration tests compare every original index field
after stripping only the documented identity additions. Duplicate aliases for
the same checkout remain supported. The owner/controller documentation now
matches their distinct identities.

Protocol minor 42 and migration 008 remain unpublished. Serialize them with
the pending scheduler, operation-evidence and repository changes before merge.
Do not edit a published migration to resolve a later collision.

### Strict subscriber baseline prerequisite for #32 and #24

The #32 parity review reproduced a missing-parent patch after thread creation.
The live database path returned before the legacy `RuntimeState::command`
assignment initially suspected by the review. The actual mismatch came from
scoped index metadata replacing the runtime's full thread list while the
committed row projection retained that list.

The separate prerequisite merges scoped index records by thread ID and uses a
full projection when a scoped poll first discovers a new thread. Regression
tests apply the resulting patches through the production strict parser and
compare committed index metadata with both the runtime and subscriber. Thread
headers compare through the production wire compactor because full history
travels separately.

The live serve-process test client now assembles snapshot pages and calls
`protocol::apply` directly. It does not create missing parents or grow arrays
to accept invalid patches. This prerequisite does not include #32 capability
policy or #24 client item caching, and it makes no performance claim.

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

## Issue #32 integration: effective capability contracts

This tree ports the completed candidate `d36694a` onto published main
`5e2eaa88`. The candidate's server projection prerequisite (`6a379b8`) was not
replayed: main's `81c0605` already contains that fix. The port keeps main's
schema 8 identity, schema 9 task operations and peer input intents, protocol 45
`peer-task-evidence-v1`, and strict paged-snapshot test client unchanged.

One `State` effective capability record per caller now drives what each thread
is told and shown. `orchestrator_capabilities` returns the record as `contract`
`shina-effective-capabilities/v1` with `caller` identity, `effective` sections
(coordination rights with precise unavailable reasons, interaction fences,
authorized reads, executable harnesses and models, placement modes, live
capacity, negotiated peer features) and a per-tool `tools` table mirroring the
advertised list. `initialize` and `tools/list` ride the same UI-thread request
queue as `tools/call`, so each bearer token answers from its own thread's
record: coordinators get the full catalog, delegated leaves get worker
instructions and no coordination tools, remote workers get the remote-worker
text and read scope, earlier homes and archived captains lose coordination.
Advertisement stays discovery only; every `tools/call` still revalidates the
gate at dispatch, so a stale `tools/list` can never widen authority.

Role-derived instructions bind through each harness's real injection path:
`Start::shina` prepends to Pi's instruction block, appends to Claude's
`--append-system-prompt`, merges into Codex `developerInstructions`, and fills
ACP's `<shina_context>`. Native Claude endpoint binding and allowlist, Pi
read-only admission, Codex and ACP agent policy differences, AGENTS.md privacy,
delivery receipts and attempt/question fences are unchanged.

The caller role includes lifecycle. Active earlier homes get the earlier-home
instructions, archived homes and crew get archived instructions, retired homes
and crew get retired instructions, crew get their own coordinator text with
authorized reports and `propose_memory`, and ordinary threads get ordinary
thread text. A role change marks a live session for a
`<shina_capability_update>` on its next prompt and clears the harness awareness
flag so a reconnect re-injects the same current text. `initialize` answers from
the current state, including an old endpoint after reset or archive.

Reachability and harness discovery are part of the same record. An ACP harness is
probed with a metadata-only `initialize` before allocation; an agent that
advertises neither HTTP nor stdio MCP is reported as `executable: false` with
its discovery and MCP state, and `delegate_task` or crew creation refuses before
creating a thread. A linked peer must also be reachable and free of a recorded
reachability or authentication error before remote delegation, placement or
document reads are reported available. A revoked origin device closes remote
Captain document reads at discovery and dispatch.

Coverage sits in `src/capabilities.rs` unit tests and
`tests/support/capability_parity.rs`. The runtime tests drive real
`initialize`, `tools/list` and `tools/call` over each thread's token endpoint
recorded by the fake Pi, native Claude, native Codex (`fake_codex.js` app-server
fixture) and ACP fixtures. The four fake agents now issue an actual
`orchestrator_capabilities` and `list_threads` workflow from the MCP
registration they were launched with; Claude's read-only crew additionally
exercises `can_use_tool`, the `shina-policy` hook callback, and denied lookalike
names. Tests compare the exact endpoint and bearer token, cover the primary
read-only Captain home, crew, delegated leaves on all four harnesses, the
earlier home after `captain.reset`, the archived Captain, a revoked bearer
token, keyed `delegate_task` retry replay for lost responses, a `NOTES.md` near
the document limit, stale `tools/list` revalidation at dispatch, unsupported
remote independent roots, ACP MCP preflight refusal, unreachable peers and a
revoked origin device. The remote test pairs two live serves and checks the
negotiated peer feature table plus the remote leaf's contract on the target
machine.

`capacity.scheduler` reports `implemented: false`; admission stays the static
per-coordinator limits until the separate scheduler work lands. The capability
contract is additive under the existing protocol 45; this port changes no
protocol constant, projection schema or migration.

The parity test also keeps a shared `NOTES.md` at 34,019 bytes. The crew prompt
carries its exact revision, size and hash plus a `captain_document_read`
pointer, while the tool returns the complete document. Pi's resumed session
records the actual `--append-system-prompt` text and checks that the former
primary home receives the earlier-home text after reset. A separate fixture
exits before ACP `initialize` and proves dispatch re-probes before allocation;
discovery caching cannot turn that failure into authority.

## Verification

The integrated tree passed on the local Apple Silicon Mac with isolated Cargo
artifacts:

- `cargo fmt --check`
- `cargo check --locked --all-targets`
- `cargo build --locked`
- `cargo test --locked`: 2,049 unit tests passed with 12 ignored; 119
  server-runtime integration tests passed with zero failures. `npm ci` installed
  the locked mobile-client dependencies, so the live TypeScript SDK test ran as
  part of the integration suite.
- `npm --prefix packages/mobile-client run typecheck`

The four harnesses are deterministic local fixtures. They prove endpoint,
bearer, MCP request and permission-policy integration, not live provider login,
account entitlement or model availability. Those paid smoke checks remain
ignored and opt-in. Remote independent crew roots and scheduler admission are
reported unavailable, not simulated.

Paid provider smoke tests remain opt-in. No issue was closed and no PR was
published from this worktree.
