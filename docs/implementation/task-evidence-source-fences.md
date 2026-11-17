# Task input evidence source and fence checks

Issue #22 remains open until the combined implementation passes its publication
and acceptance checks. PR #56 published durable target input recovery. This
follow-up checks how the controller accepts evidence from those targets.

Two reducer regressions failed against published main `5e2eaa8` before the fix:

- A reply for another task could update an operation by naming its delivery
  command UUID, including when the reply came from a different peer.
- Omitting an expected attempt, request or runtime-generation field could
  bypass its comparison and promote provider acceptance.

These are reproduced evidence-ingestion defects. The tests do not claim an
unauthenticated network exploit or that another peer can discover command UUIDs.

Each pending RPC now retains the server and target thread selected for its
authenticated transport. The controller discards a response if that binding no
longer matches the task. Wait and completion responses carry that trusted source
and task identity through input-evidence collection.

Before changing any operation field, the controller requires the same task,
target server, controller-local child thread and current target thread. It then
compares the saved attempt, request and generation exactly. An expected string
cannot be absent, null or another JSON type. An absent optional fence permits
only an absent or null peer field.

Regressions cover foreign task evidence in both wait and completion replies,
foreign peer and target-thread bindings, late responses after a target change,
missing and malformed fences, unexpected optional fences and valid provider
acceptance. Existing two-server recovery and stale-question tests remain part
of the full suite.

This change adds no wire contract, schema migration or automatic provider replay.
Recovery for adapters without authoritative input history still remains held or
unknown. It does not claim universal exactly-once provider delivery.

## Preserve a restored input's recovery hold

A fresh peer prompt cannot clear a hold left by an older input whose acceptance
is unknown. The target rejects the new prompt before staging it or changing
the attempt. A replay of the original command still returns its saved receipt.

A matching user-input marker can prove acceptance even when the rest of history
is incomplete. The target then removes that exact restored input from its
queue. It releases the thread hold only after all held prompt intents are
resolved. Missing history from a non-authoritative adapter is still unknown,
and never authorizes automatic replay.
