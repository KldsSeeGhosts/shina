# Reader state through projection recovery

The #24 acceptance audit after release `v0.12.83` left combined reconnect and
device-state preservation unproven. The new headless regression reproduced
disclosure loss after a transport cache reset at main `6eff805`. A normal
reconnect passed; rendering an unloaded frame before fresh history arrived
discarded the previous tool expansion state.

The correction distinguishes hydrated empty history from unloaded history in
the internal thread projection. Before releasing unloaded payloads, the client
retains only disclosure keys and flags. Tools use their ids; thoughts and work
use stable message identities instead of loaded-window positions. Rehydration
consumes the retained keys. The cache does not retain message text, tool output,
or the old field/item Arcs.

The existing single-viewport scroll model captures its visible row and offset
before an unloaded frame removes that row. It defers restoration until history
is hydrated. This follows KSG's `timelineScrollAnchoring.ts` separation of
remembered position/disclosures from transport hydration. No layout or visual
skin changed.

The combined regression exercises a background stream while the device reads
older content, a normal reconnect, and a cache reset with a rendered unloaded
frame followed by resubscription. At widths 620, 960, 1280 and 1920 it checks
tool/raw/group/thought/work disclosure, the unsent draft, active thread, tabs,
sidebar selection, deferred-work markers and the visible row's position. A weak payload
reference proves that the unloaded frame releases the old transcript.

All 15 delta-projection tests, 14 client-history tests, and the existing scroll
anchor and sticky-bottom tests pass. These are headless geometry and state
checks, not hardware frame timings. Exact-head CI and publication still gate
the final #24 acceptance decision. #25 and #26 remain separate.

No wire contract, protocol minor, persistence schema or migration changed.

## Recent-window recovery correction

Exact-head review at `0015b0d` strengthened the test to render the recovered
recent window before fetching older pages. The old test loaded all pages
before that render. The stronger test failed at 620 px: the reader's row moved
24 px after a cache reset.

The fixed-height viewport's layout callback could run with an older recent-only
row list after a newer page had already applied. It could also fail to run again
when older content arrived without resizing the viewport. Retaining the anchor
alone did not fix either event-ordering problem.

Following the Claude Code Opus 5.5 medium review, restoration now runs with the
client projection's scroll transaction. It uses the current row list, requests
older history by the saved message identity when needed, and consumes the anchor
only when that row arrives or history is exhausted. The first-mount callback
uses the same helper without requesting history. No callback retains an old row
list to decide whether the current anchor is missing.

The regression applies projections through a Rune update transaction, matching
production. At 620, 960, 1280 and 1920 px it exercises both settled recent-only
frames and newer pages arriving before older layout events drain. Both paths
preserve the reader row and disclosure state, with no retained transcript
payloads. These stronger cases pass locally; final exact-head CI and publication
remain required.

The review also found positional work-disclosure fallback copying an expanded
flag to a different message when a paged window reused the same local ordinal.
A separate regression failed before the correction. Positional fallback now
applies only when the old work boundary has no message identity; normal work
matches by its stable message id.

Final local validation passes the all-target build, formatting and diff checks,
16 delta-projection tests, 14 client-history tests, the existing prepend/expand/
image/width scroll-anchor test and the streaming sticky-bottom test. Full CI
must run again on the updated PR head. No native frame-time claim is added.

## Dispatch-only projection correction

CI run `44082649873` failed the two-server stale-question test at `5a287d1`.
Its frozen snapshot showed the answer operation still queued while the peer
send waited behind the fixture barrier. Dispatch had already updated the
in-memory operation before the next reducer transaction. Reconciliation staged
that evidence in SQLite but returned no change, so a quiet batch could commit
without publishing the global operation projection.

Reconciliation now reports deferred operation changes to the reducer. The
normal full-projection path publishes them before any peer receipt or unrelated
event arrives. Transport and provider acceptance remain unobserved until their
own evidence arrives. No timeout, fence or test assertion was relaxed.

A deterministic regression fails before the correction and passes afterward.
It checks the change signal, committed dispatch evidence, unchanged transport
and acceptance fields, and one-time publication. The original live two-server
stale-question/restart test also passes locally.
