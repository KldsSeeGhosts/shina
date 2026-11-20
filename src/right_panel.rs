use std::collections::HashMap;

/// KSG `useResizableWidth`'s minimum panel width.
pub const MIN_WIDTH: f32 = 360.0;
/// KSG `useResizableWidth`'s starting width.
pub const DEFAULT_WIDTH: f32 = 420.0;
/// KSG's keyboard resize step for the panel's Left and Right keys. The resize
/// handle's key handler uses it.
pub const KEYBOARD_STEP: f32 = 16.0;
const MAX_FRACTION: f32 = 0.7;
/// KSG keeps the conversation at least this wide beside an inline panel.
const SIBLING_MIN: f32 = 360.0;

/// The key a thread's panel state lives under: machine plus thread id, so the
/// same numeric id on two hosts stays separate.
pub fn key(machine: &str, thread: usize) -> String {
    format!("{machine}:{thread}")
}

/// KSG `getPreviewPanelMaxWidth`: seventy percent of the viewport, further
/// capped so the conversation keeps [`SIBLING_MIN`], never below [`MIN_WIDTH`].
/// `container` is the width of the row the panel shares with the conversation.
/// The panel view and resize handle use it.
pub fn max_width(viewport: f32, container: f32) -> f32 {
    (viewport * MAX_FRACTION)
        .floor()
        .min((container - SIBLING_MIN).floor())
        .max(MIN_WIDTH)
}

/// KSG's `clamp`: a non-finite stored width falls back to the default. The
/// panel view and resize handle use it.
pub fn clamp_width(width: f32, max: f32) -> f32 {
    if !width.is_finite() {
        return DEFAULT_WIDTH;
    }
    width.clamp(MIN_WIDTH, max.max(MIN_WIDTH))
}

/// When the row the panel shares with the conversation is narrower than the
/// panel's minimum plus the conversation's, the panel covers it instead (KSG's
/// sheet mode). The panel view uses it.
pub fn covers(container: f32) -> bool {
    container < MIN_WIDTH + SIBLING_MIN
}

/// The one item the panel's Viewer segment can hold. Opening another replaces
/// the current one; `State::open_captain_document` asks before dropping a dirty
/// draft.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Surface {
    CaptainDocument { captain: usize, name: String },
}

impl Surface {
    pub fn id(&self) -> String {
        match self {
            Self::CaptainDocument { captain, name } => {
                format!("captain-doc:{captain}:{name}")
            }
        }
    }
}

/// The Inspector's four fixed tabs. `right_panel_body` degrades a stored view
/// the open thread does not offer (such as Crew on a plain thread) to
/// `Overview`, so saved state always lands somewhere real.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PanelView {
    #[default]
    Overview,
    Crew,
    Context,
    Diff,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SingletonKind {
    Overview,
    Crew,
    Context,
    Diff,
}

impl SingletonKind {
    fn into_view(self) -> PanelView {
        match self {
            Self::Overview => PanelView::Overview,
            Self::Crew => PanelView::Crew,
            Self::Context => PanelView::Context,
            Self::Diff => PanelView::Diff,
        }
    }

    fn matches_view(self, view: PanelView) -> bool {
        self.into_view() == view
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThreadPanel {
    pub open: bool,
    pub view: PanelView,
    /// The one viewer item, routed inside the Context tab: `Context` shows it
    /// while `Some`, the context list while `None`.
    pub viewer: Option<Surface>,
    pub maximized: bool,
}

/// Keyed by a caller-supplied thread key string.
#[derive(Clone, Debug, Default)]
pub struct RightPanels {
    threads: HashMap<String, ThreadPanel>,
}

impl RightPanels {
    pub fn get(&self, thread: &str) -> ThreadPanel {
        self.threads.get(thread).cloned().unwrap_or_default()
    }

    /// Opens (or shows) a singleton view: the panel's own Diff segment.
    pub fn open(&mut self, thread: &str, kind: SingletonKind) {
        self.update_thread(thread, |current| {
            current.open = true;
            current.view = kind.into_view();
        });
    }

    /// Shows the panel on its stored view, or the default tab on a fresh entry.
    pub fn show(&mut self, thread: &str) {
        self.update_thread(thread, |current| {
            current.open = true;
        });
    }

    /// Replaces the viewer with a Captain document and shows it. The caller
    /// parks the swap behind the dirty-draft confirmation when the current
    /// document has unsaved edits (`State::open_captain_document`).
    pub fn open_captain_document(&mut self, thread: &str, captain: usize, name: &str) {
        self.update_thread(thread, |current| {
            current.open = true;
            current.view = PanelView::Context;
            current.viewer = Some(Surface::CaptainDocument {
                captain,
                name: name.to_string(),
            });
        });
    }

    /// Moves the viewer item between panel keys, keeping its segment selection.
    /// Called when a Captain's home is reset to a new conversation.
    pub fn carry_captain_documents(&mut self, from: &str, to: &str) {
        let source = self.threads.get(from).cloned().unwrap_or_default();
        let Some(viewer) = source.viewer.clone() else {
            return;
        };
        self.update_thread(from, |current| {
            current.viewer = None;
        });
        self.update_thread(to, |current| {
            current.viewer = Some(viewer);
            if source.open && source.view == PanelView::Context {
                current.open = true;
                current.view = PanelView::Context;
            }
        });
    }

    /// Closes the viewer item. The panel stays on Context, which shows the
    /// document list again.
    pub fn close_viewer(&mut self, thread: &str) {
        self.update_thread(thread, |current| {
            current.viewer = None;
        });
    }

    pub fn close(&mut self, thread: &str) {
        self.update_thread(thread, |current| {
            if current.open {
                current.open = false;
            }
        });
    }

    pub fn set_maximized(&mut self, thread: &str, maximized: bool) {
        self.update_thread(thread, |current| {
            if current.maximized != maximized {
                current.maximized = maximized;
            }
        });
    }

    pub fn toggle_visibility(&mut self, thread: &str) {
        self.update_thread(thread, |current| {
            current.open = !current.open;
        });
    }

    /// KSG's surface toggle: an open panel already showing the segment hides;
    /// anything else shows it.
    pub fn toggle(&mut self, thread: &str, kind: SingletonKind) {
        self.update_thread(thread, |current| {
            if current.open && kind.matches_view(current.view) {
                current.open = false;
                return;
            }
            current.open = true;
            current.view = kind.into_view();
        });
    }

    /// The segment on screen, only while the panel is open.
    pub fn active_view(&self, thread: &str) -> Option<PanelView> {
        let state = self.threads.get(thread)?;
        if !state.open {
            return None;
        }
        Some(state.view)
    }

    pub fn forget(&mut self, thread: &str) {
        self.threads.remove(thread);
    }

    fn update_thread(&mut self, thread: &str, f: impl FnOnce(&mut ThreadPanel)) {
        let mut next = self.threads.get(thread).cloned().unwrap_or_default();
        f(&mut next);
        // A closed entry that still carries state (a viewer, the maximized flag,
        // or a non-default last tab) stays so the panel reopens where it left off;
        // a default closed entry is dropped.
        if !next.open
            && next.viewer.is_none()
            && !next.maximized
            && next.view == PanelView::default()
        {
            self.threads.remove(thread);
        } else if self.threads.get(thread) != Some(&next) {
            self.threads.insert(thread.to_string(), next);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const THREAD_A: &str = "env-1:thread-A";
    const THREAD_B: &str = "env-1:thread-B";

    fn doc(captain: usize, name: &str) -> Surface {
        Surface::CaptainDocument {
            captain,
            name: name.into(),
        }
    }

    #[test]
    fn open_overview_on_empty_thread() {
        let mut panels = RightPanels::default();
        panels.open(THREAD_A, SingletonKind::Overview);

        assert_eq!(
            panels.get(THREAD_A),
            ThreadPanel {
                open: true,
                view: PanelView::Overview,
                viewer: None,
                maximized: false,
            }
        );
        assert_eq!(panels.active_view(THREAD_A), Some(PanelView::Overview));
    }

    #[test]
    fn open_overview_twice_keeps_one_segment() {
        let mut panels = RightPanels::default();
        panels.open(THREAD_A, SingletonKind::Overview);
        panels.open(THREAD_A, SingletonKind::Overview);

        let state = panels.get(THREAD_A);
        assert_eq!(state.view, PanelView::Overview);
        assert!(state.viewer.is_none());
        assert!(state.open);
    }

    #[test]
    fn a_second_document_replaces_the_viewer_and_close_returns_to_the_context_list() {
        let mut panels = RightPanels::default();
        panels.open_captain_document(THREAD_A, 0, "NOTES.md");
        panels.open_captain_document(THREAD_A, 0, "AGENTS.md");

        let state = panels.get(THREAD_A);
        assert_eq!(state.viewer, Some(doc(0, "AGENTS.md")));
        assert_eq!(state.view, PanelView::Context);

        panels.close_viewer(THREAD_A);
        let state = panels.get(THREAD_A);
        assert!(state.open);
        assert_eq!(state.view, PanelView::Context);
        assert!(state.viewer.is_none());
    }

    #[test]
    fn toggle_diff_states() {
        let mut panels = RightPanels::default();

        // Closed: opens the Diff segment.
        panels.toggle(THREAD_A, SingletonKind::Diff);
        let state = panels.get(THREAD_A);
        assert!(state.open);
        assert_eq!(state.view, PanelView::Diff);

        // Open on Diff: hides, keeping the segment selection.
        panels.toggle(THREAD_A, SingletonKind::Diff);
        let closed = panels.get(THREAD_A);
        assert!(!closed.open);
        // Hiding keeps the last tab so a re-open lands where the user left off.
        assert_eq!(closed.view, PanelView::Diff);

        // Closed again: reopens on Diff.
        panels.toggle(THREAD_A, SingletonKind::Diff);
        assert!(panels.get(THREAD_A).open);

        // Open on a document in Context: switches to Diff and stays open.
        panels.open_captain_document(THREAD_A, 0, "NOTES.md");
        assert_eq!(panels.get(THREAD_A).view, PanelView::Context);
        panels.toggle(THREAD_A, SingletonKind::Diff);
        let switched = panels.get(THREAD_A);
        assert!(switched.open);
        assert_eq!(switched.view, PanelView::Diff);
        assert_eq!(switched.viewer, Some(doc(0, "NOTES.md")));
    }

    #[test]
    fn toggle_visibility_hides_and_reshows() {
        let mut panels = RightPanels::default();

        // Empty: toggling shows a bare panel on the default tab, and hiding it
        // evicts the entry.
        panels.toggle_visibility(THREAD_A);
        assert!(panels.get(THREAD_A).open);
        panels.toggle_visibility(THREAD_A);
        assert_eq!(panels.get(THREAD_A), ThreadPanel::default());

        // With a viewer: hides and re-shows on the same segment.
        panels.open_captain_document(THREAD_A, 0, "NOTES.md");
        panels.toggle_visibility(THREAD_A);
        let hidden = panels.get(THREAD_A);
        assert!(!hidden.open);
        assert_eq!(hidden.view, PanelView::Context);
        assert_eq!(hidden.viewer, Some(doc(0, "NOTES.md")));
        assert_eq!(panels.active_view(THREAD_A), None);

        panels.toggle_visibility(THREAD_A);
        assert!(panels.get(THREAD_A).open);
        assert_eq!(panels.active_view(THREAD_A), Some(PanelView::Context));
    }

    #[test]
    fn set_maximized_then_close_preserves_maximized() {
        let mut panels = RightPanels::default();
        panels.open(THREAD_A, SingletonKind::Diff);
        panels.set_maximized(THREAD_A, true);
        assert!(panels.get(THREAD_A).maximized);

        panels.close(THREAD_A);
        let state = panels.get(THREAD_A);
        assert!(!state.open);
        assert!(state.maximized);
    }

    #[test]
    fn carry_moves_the_viewer_and_keeps_the_context_tab() {
        let mut panels = RightPanels::default();
        panels.open_captain_document(THREAD_A, 0, "NOTES.md");

        panels.carry_captain_documents(THREAD_A, THREAD_B);
        let from = panels.get(THREAD_A);
        assert!(from.viewer.is_none());
        assert_eq!(from.view, PanelView::Context);
        assert!(from.open, "the context list stays open on the old home");

        let to = panels.get(THREAD_B);
        assert!(to.open);
        assert_eq!(to.view, PanelView::Context);
        assert_eq!(to.viewer, Some(doc(0, "NOTES.md")));

        // A hidden or diff-selected source carries the item without opening it.
        let mut panels = RightPanels::default();
        panels.open_captain_document(THREAD_A, 0, "NOTES.md");
        panels.open(THREAD_A, SingletonKind::Diff);
        panels.close(THREAD_A);
        panels.carry_captain_documents(THREAD_A, THREAD_B);
        let to = panels.get(THREAD_B);
        assert!(!to.open);
        assert_eq!(to.viewer, Some(doc(0, "NOTES.md")));
    }

    #[test]
    fn threads_are_isolated_and_forget_removes_one() {
        let mut panels = RightPanels::default();
        panels.open(THREAD_A, SingletonKind::Overview);
        panels.set_maximized(THREAD_A, true);
        panels.open_captain_document(THREAD_B, 0, "NOTES.md");

        assert_eq!(panels.active_view(THREAD_A), Some(PanelView::Overview));
        assert_eq!(panels.get(THREAD_B).viewer, Some(doc(0, "NOTES.md")));

        panels.forget(THREAD_A);
        assert_eq!(panels.get(THREAD_A), ThreadPanel::default());
        assert_eq!(panels.active_view(THREAD_A), None);
        assert!(panels.get(THREAD_B).open);
    }

    #[test]
    fn empty_closed_entries_are_evicted() {
        let mut panels = RightPanels::default();
        panels.toggle_visibility(THREAD_A);
        panels.toggle_visibility(THREAD_A);
        assert_eq!(panels.get(THREAD_A), ThreadPanel::default());

        panels.open_captain_document(THREAD_A, 0, "NOTES.md");
        panels.close_viewer(THREAD_A);
        panels.close(THREAD_A);
        // The stored Context view stays so the panel reopens on its last tab.
        let closed = panels.get(THREAD_A);
        assert!(!closed.open);
        assert_eq!(closed.view, PanelView::Context);
        assert!(closed.viewer.is_none());

        panels.toggle_visibility(THREAD_A);
        panels.open(THREAD_A, SingletonKind::Overview);
        panels.close(THREAD_A);
        assert_eq!(panels.get(THREAD_A), ThreadPanel::default());
    }

    #[test]
    fn key_scopes_panel_state_to_machine_and_thread() {
        assert_eq!(key("local", 7), "local:7");
        assert_ne!(key("local", 7), key("studio", 7));
    }

    // KSG `getPreviewPanelMaxWidth`: `min(70vw, container - 360)`, never under the 360 minimum.
    #[test]
    fn max_width_is_the_viewport_cap_limited_by_the_conversation() {
        // The viewport cap binds: 0.7 * 1920 = 1344 < 1920 - 360 = 1560.
        assert_eq!(max_width(1920.0, 1920.0), 1344.0);
        // At 1280 the caps are nearly even; the viewport cap still wins: 896 < 920.
        assert_eq!(max_width(1280.0, 1280.0), 896.0);
        // A narrow shared row leaves the conversation its 360 and no more.
        assert_eq!(max_width(960.0, 724.0), 364.0);
        // A container smaller than the minimum plus the conversation's keeps the floor.
        assert_eq!(max_width(620.0, 604.0), MIN_WIDTH);
    }

    // KSG's `clamp`: stored widths clamp into [minimum, maximum].
    #[test]
    fn clamp_width_holds_the_stored_width_inside_the_limits() {
        assert_eq!(clamp_width(200.0, 896.0), MIN_WIDTH);
        assert_eq!(clamp_width(1200.0, 896.0), 896.0);
        assert_eq!(clamp_width(500.0, 896.0), 500.0);
        assert_eq!(clamp_width(f32::NAN, 896.0), DEFAULT_WIDTH);
        // A maximum under the floor still clamps up to the minimum.
        assert_eq!(clamp_width(540.0, 300.0), MIN_WIDTH);
    }

    // KSG's sheet switch: under the panel's minimum plus the conversation's, the panel covers.
    #[test]
    fn covers_only_below_the_shared_minimum() {
        assert!(!covers(MIN_WIDTH + 360.0));
        assert!(covers(MIN_WIDTH + 359.0));
        assert!(covers(604.0));
    }
}
