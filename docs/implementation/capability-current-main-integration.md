# Effective capability integration on published main

This root-owned checkout preserves returned candidate `4cf85b0` and merges
published main `6eff805175c6001b7c9166bc276827d2c4257210`. The original worker
checkout remains unchanged. The merge applied without conflicts.

The resulting diff against main contains the effective capability contract,
role-specific instructions, adapter preflight and deterministic adapter
fixtures. It retains main's schema 9 task-input ledger, trusted task/peer/thread
reply source, exact required request fences and item-level client projection.
It does not replay the already published server baseline prerequisite.

## Requirement review

| Issue #32 requirement | Evidence |
| --- | --- |
| One declared workflow under the four supported adapters | `capability_contract_binds_advertisement_and_dispatch_to_the_calling_role` starts fake Pi, Claude, Codex and ACP through their actual adapter paths. The fixtures use their configured token endpoint. Claude crew additionally exercises `can_use_tool` and the permission hook. |
| Unsupported combinations fail before allocation | ACP no-MCP and process-startup regressions check refusal and unchanged thread count. Peer executable discovery rejects known unsupported targets at the origin; the executor probes again before allocating its thread. |
| Caller role and lifecycle coverage | Unit and live cases cover the read-only primary home, ordinary crew, delegated leaves, remote workers, earlier homes, archived/retired ownership and revoked document authority. Independent remote crew roots remain explicitly unsupported. |
| Retry, peer mismatch, startup and bounded context | Keyed delegation replay retains the original thread. Paired-server cases check feature negotiation and revocation. Large shared documents use the manifest/read path instead of copying the document into the prompt. |
| Native permission responses remain enforced | Claude read-only policy and hook tests refuse unapproved lookalike tool names. Adapter approval tests remain part of the full suite. No provider permission bypass was added. |
| Discovery examples agree with dispatch | `src/capabilities.rs` probes real dispatcher outcomes for listed tools and checks exact argument-dependent refusals. Live tests use `initialize`, `tools/list` and `tools/call`, including a stale endpoint after a Captain reset. |
| Deterministic end-to-end CI | Four `capability_parity` integration tests run inside the existing `server_runtime` suite. Paid-provider smoke tests remain separate and opt-in. |

This evidence concerns fixture wiring, authorization and dispatch. It does not
prove live provider login, account entitlement or model availability. An
origin's cached peer executable record can become stale. The target refuses
before target-thread allocation, but the origin may already have allocated a
metadata-only task when its cached record still advertised support.

## Verification and publication gates

The combined all-target build, formatting and diff checks pass. The full serial
unit suite and all 126 integration tests pass, including the four live
capability-parity cases, strict projection recovery and source-bound task
evidence. The mobile SDK live test and TypeScript check pass. Locked mobile
dependencies were installed without changing the lockfile. The complete
verification command exited successfully on the merge's source tree; subsequent
changes only record this evidence.

The original base used protocol minor 45 and static per-coordinator admission.
The scheduler composition below replaces that old capacity description.

Exact-head CI, combined-main review, serialized merge and downloaded release
verification remain required. Issue #32 stays open. Issue #22's unsupported
automatic recovery case is not resolved by this capability contract.

## Scheduler composition

This candidate now integrates PR59 head `a8fd656`, which includes PR60's merged
source `970dc0c`. The one source conflict kept the durable scheduler instead of
the obsolete static task-start loop.

Capability capacity now reads the scheduler's actual limits, held occupancy,
caller queue and status. It advertises `admission-scheduler-v1` as implemented,
not the original static-capacity placeholder. The existing capability unit and
live assertions use that composed contract. Protocol minor 46 comes from PR59;
this integration reserves no additional number and changes no migration.

No new tests, provider configuration or feature scope was added. PR59 must
finish its own CI and publication before this candidate can merge into main.

The combined-head Linux CI exposed two older native model-inheritance tests
that depended on Claude/Codex being installed on the build host. They now use
the existing per-state fake connection hook. Native executable discovery and
dispatch share that test-only executor proof. Production probing and the real
fixture-adapter integration tests remain unchanged. No CLI installation,
provider configuration, authentication change or test skip is required.

A later CI run passed all units but exposed overlapping fake-provider turns
between the mobile SDK's receipt/replay and resume subtests. Those cases now
wait for their own committed completion before the next case starts. The
resume, prompt-count and history assertions are unchanged. The original live
SDK integration test and TypeScript check pass with these completion barriers.
