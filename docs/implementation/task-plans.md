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
than starting those nodes locally by mistake. The desktop needs its plan
controls and a live stacked-launch check before closure. No production provider
credentials or sessions are required for verification.
