//! The Inspector's Context tab.
//!
//! Step 0 keeps the Brief's document, scope, memory and earlier-conversation
//! sections as they were. Workstream 4 owns this file and reshapes the tab into
//! the context-management layout in Section 4 of the wave 2 spec; the Inspector
//! only ever calls `tab(s, p)` from here.
use super::*;

use super::crew_ui::hairline;

/// The Context tab's body: one scrolling column of `captain_ui` sections on a
/// captain-backed thread, separated by hairlines like the Brief's. A plain
/// thread has no context sources yet, so it gets a quiet note until the
/// Workstream 4 layout lands.
pub(super) fn tab(s: &State, p: Palette) -> El {
    let t = s.current();
    let captain = t
        .captain
        .filter(|_| s.is_captain_home(t))
        .or(t.crew_captain)
        .filter(|captain| *captain < s.captains.len());
    let Some(captain) = captain else {
        return col()
            .key("context-tab")
            .min_h(0.0)
            .child(captain_ui::quiet_note(
                "This thread has no context sources yet.",
                p,
            ));
    };
    let access = s.captain_access(captain);
    let record = s.brief.record.is_some();
    // The same "knowable" rule the Brief used: a host without captain records or
    // the protocol cannot fill these sections, so they stay hidden.
    let knowable = !matches!(
        access,
        crate::model::CaptainAccess::UpdateRequired | crate::model::CaptainAccess::NoRecords
    );
    let sections = [
        knowable.then(|| captain_ui::documents_section(s, p, captain)),
        (!record).then(|| {
            captain_ui::scope_section(s, p, captain)
                .disabled(access != crate::model::CaptainAccess::Full)
        }),
        Some(captain_ui::memory_section(s, p, captain)),
        Some(captain_ui::earlier_section(s, p, captain)),
    ];
    let mut body = col().key("context-tab").min_h(0.0).w_full().pb(8.0);
    for section in sections.into_iter().flatten() {
        body = body
            .child(el().h(1.0).mx(16.0).fill(hairline(p)))
            .child(section);
    }
    col()
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(body)
}
