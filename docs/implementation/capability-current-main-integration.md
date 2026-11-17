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
unit/integration run and mobile TypeScript check are in progress.

The current base still uses protocol minor 45 and static per-coordinator
admission. `capacity.scheduler.implemented` therefore remains false. If the
separate scheduler PR lands first, this branch must integrate its actual
capacity and admission contract before publication. No protocol number or
migration was changed.

Exact-head CI, combined-main review, serialized merge and downloaded release
verification remain required. Issue #32 stays open. Issue #22's unsupported
automatic recovery case is not resolved by this capability contract.
