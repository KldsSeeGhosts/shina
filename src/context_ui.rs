//! The Inspector's Context tab.
//!
//! Step 0 keeps the Brief's document, scope, memory and earlier-conversation
//! sections as they were. Workstream 4 owns this file and reshapes the tab into
//! the context-management layout in Section 4 of the wave 2 spec; the Inspector
//! only ever calls `tab(s, p)` from here.
use super::*;

use super::crew_ui::hairline;

/// Entry point for the Inspector's Context tab (both the list and the open
/// captain-document viewer route). Workstream 4 owns this seam completely.
pub(super) fn view(s: &State, p: Palette) -> (String, El) {
    if let Some(crate::right_panel::Surface::CaptainDocument { captain, name }) =
        &s.right_panel().viewer
    {
        return (
            format!("right-panel-body-captain-doc:{captain}:{name}"),
            captain_docs_ui::document_panel(s, p, *captain, name),
        );
    }
    ("right-panel-body-context".into(), tab(s, p))
}

/// The Context tab's list body: one scrolling column of `captain_ui` sections on a
/// captain home, separated by hairlines like the Brief's. Worker threads and plain
/// threads wait for Workstream 4's read-only "From the captain" / attachment views
/// instead of mounting home-only document/scope controls on a worker thread.
pub(super) fn tab(s: &State, p: Palette) -> El {
    let t = s.current();
    let captain = captain_ui::home_brief_captain(s);
    let Some(captain) = captain else {
        return col()
            .key("context-tab")
            .min_h(0.0)
            .child(captain_ui::quiet_note(
                if t.crew_captain.is_some() || t.delegated || t.peer_origin.is_some() {
                    "Open the captain's home to inspect shared documents, memory, and workspace scope."
                } else {
                    "This thread has no context sources yet."
                },
                p,
            ));
    };
    let access = s.captain_access(captain);
    // The same "knowable" rule the Brief used: a host without captain records or
    // the protocol cannot fill these sections, so they stay hidden.
    let knowable = !matches!(
        access,
        crate::model::CaptainAccess::UpdateRequired | crate::model::CaptainAccess::NoRecords
    );
    let sections = [
        knowable.then(|| captain_ui::documents_section(s, p, captain)),
        Some(
            captain_ui::scope_section(s, p, captain)
                .disabled(access != crate::model::CaptainAccess::Full),
        ),
        Some(captain_ui::memory_section(s, p, captain)),
        Some(captain_ui::earlier_section(s, p, captain)),
    ];
    let mut body = col().key("context-tab").min_h(0.0).w_full().pb(8.0);
    if let Some(error) = captain_ui::brief_error_banner(s, p) {
        body = body.child(error);
    }
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
