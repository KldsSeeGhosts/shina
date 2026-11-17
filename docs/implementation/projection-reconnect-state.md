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
