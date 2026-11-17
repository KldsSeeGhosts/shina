# Execution leases and controller ownership

Schema 8 index version 5 stores execution leases in the additive
`execution` object. The change needs no SQLite table and does not use migration
9, which is reserved for the repository catalog. The wire contract is the
exact capability `execution-leases-v1` at protocol minor 44. A peer that does
not advertise that capability receives no managed task or lease. Minor 44 alone
does not grant any other reserved feature.

## Identity split

Each lease records these values separately:

- `workspaceId` and `ownerId`: the controller workspace and data owner.
- `runAsOwnerId`: the owner whose work this run performs.
- `controllerId`: the sole authority for Captain intent, membership,
  documents, mailbox and logical thread identity.
- `initiatingDeviceId`: the authenticated device that submitted the intent.
- `executorId`: the machine currently allowed to run the provider turn.
- `logicalCaptainUid` and `logicalThreadUid`: controller-owned stable ids.
- `executorThreadAlias` and `executorThreadUid`: executor-local physical
  aliases. They are never used as controller identity.

These ids are data. They do not authenticate a caller and do not create a
device grant.

## Peer binding

`peer.link_peer` sends the caller's server, workspace, owner and controller
identities in the one-use peer pairing exchange. The target validates them and
stores the binding against the newly issued `peer-delegate` device in SQLite
metadata. The binding is written in the same transaction as the device and its
grant.

Before `peer.thread.launch`, `peer.thread.send`, `peer.thread.stop` or
`peer.execution.fence` reaches the reducer, the runtime compares the
assignment's controller identity and the origin's server identity with that
authenticated binding. A peer-supplied label, owner or controller field is
never accepted on its own. Existing peer credentials without a binding must be
re-paired before they can control executions.

## Lease lifecycle

Leases use one of these phases:

1. `prepared`: controller intent is in the runtime transaction.
2. `dispatched`: the assignment or provider command is committed for dispatch.
3. `executorAccepted`: an authenticated executor committed the assignment.
4. `providerAccepted`: the executor observed a successful provider prompt
   response.
5. `running`: the executor observed provider work start or assistant output.
6. `terminal`: a matching provider terminal event or explicit terminal
   reconciliation was observed.
7. `uncertain`: acknowledgement or connectivity was lost, or the lease
   expired without terminal or fence evidence.
8. `fenced`: an executor proved its provider process absent and its thread
   quiescent.

A peer acknowledgement moves a lease to `executorAccepted` only. It never
means the provider accepted or ran the prompt.

Generations increase monotonically per logical thread. A retry returns the
existing lease when controller, logical thread, run and attempt identities
match. A different controller or conflicting generation is refused. Stops,
steers and follow-ups name the exact lease, generation and run they observed;
an older control cannot affect a newer generation.

## Controller offline and uncertainty

Lease expiry, a missing peer acknowledgement, an executor link loss or a
controller restart does not prove that an offline provider stopped. Those
conditions make the lease `uncertain`.

An uncertain lease blocks retry, resume, placement replacement and a new
controller generation. It does not stop the remote process and it does not
choose another executor. Another generation requires one of:

- a matching provider terminal event;
- an authenticated executor `peer.execution.fence` result after that executor
  verified no provider process, pending prompt or request;
- an explicit local reconciliation with the same evidence.

There is no controller-only force-fence. A controller flag cannot manufacture
quiescence on an offline executor.

The controller remains authoritative while offline. Its saved intent, logical
Captain home, membership, documents, tasks and mailbox do not move. Executor
records retain the controller thread UID beside their physical thread alias.
Starting another executor does not allocate a second home or reparent work.

## Inspection

`execution.inspect` accepts `threadId` or `threadUid` and returns the monotonic
cursor, active lease and full lease history. With `executorId` it returns that
executor's leases. `task_status` exposes the controller run attribution, and
`peer.thread.wait` carries the executor lease so the controller can mirror
only observed phases.

Content transfer, checkout preparation and provider-context portability remain
issue 35 work. This layer never copies a live SQLite file, provider directory,
credential or arbitrary session.
