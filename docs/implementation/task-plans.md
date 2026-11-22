# Task plans

Issue #34 separates drafting from starting work. `task_prepare` saves a node
without making a worker, worktree or admission reservation. `task_update`
checks the node revision and cannot change a launched node. `task_plan` reads
the saved drafts, dependency decisions and launch records under the caller's
normal delegation scope.

`task_start` takes an explicit list of nodes and a request key. It checks the
graph, resolves each selected placement and checks capacity for the whole
batch before staging any worker. It saves the selected launches and receipt
before dispatch. Replaying an accepted request returns the same launches.
Expected outputs are included with the brief the worker receives.

A failed or stopped dependency blocks the next node unless the caller provides
an override reason. A stacked node needs a completed local worktree and a Git
commit verified from it. The child receives that exact commit as its isolated
base. An override does not let a stacked node bypass the Git check.

The backend started at Devin commit `0177fae` and was integrated against main
`fb64bc0`. The ordinary delegation path keeps the current harness preflight
and admission rules. Plans currently reject linked executor targets rather
than starting those nodes locally by mistake.

The live server test drafts three nodes through authenticated MCP, revises the
stacked child, and refuses that child before its dependency is ready. It checks
the downstream worktree's actual `HEAD` and file content against a distinct
upstream commit. After a server restart, the same request key returns the same
task and thread without another worker prompt. The third node remains a draft.
Verification uses only a fixture provider and a temporary Git repository.

## Server commands

The desktop no longer has a plan panel. It was removed on 2026-11-22 because
the user never opened it. Captains still stage work through the MCP tools.

The server keeps the `task.plan`, `task.plan.prepare`, `task.plan.update` and
`task.plan.start` commands for other clients. Each carries `{threadId, args}`.
The reducer selects the named home, checks its owner and calls the MCP tool as
that home, so the write guard, scope, revision checks and launch receipts are
the MCP ones. `task.plan` needs the read grant and the others need operate.
The server offers `task-plans-v1` from protocol 1.49 and refuses these methods
to older clients. A `task.plan` read writes no receipt.
