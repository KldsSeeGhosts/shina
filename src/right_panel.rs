use std::collections::HashMap;

/// KSG `useResizableWidth`'s minimum panel width.
pub const MIN_WIDTH: f32 = 360.0;
/// KSG `useResizableWidth`'s starting width.
pub const DEFAULT_WIDTH: f32 = 540.0;
/// KSG's keyboard resize step for the panel's Left and Right keys. The resize
/// handle's key handler uses it.
#[allow(dead_code)]
pub const KEYBOARD_STEP: f32 = 16.0;
#[allow(dead_code)]
const MAX_FRACTION: f32 = 0.7;
/// KSG keeps the conversation at least this wide beside an inline panel.
#[allow(dead_code)]
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
#[cfg_attr(not(test), allow(dead_code))]
pub fn max_width(viewport: f32, container: f32) -> f32 {
    (viewport * MAX_FRACTION)
        .floor()
        .min((container - SIBLING_MIN).floor())
        .max(MIN_WIDTH)
}

/// KSG's `clamp`: a non-finite stored width falls back to the default. The
/// panel view and resize handle use it.
#[cfg_attr(not(test), allow(dead_code))]
pub fn clamp_width(width: f32, max: f32) -> f32 {
    if !width.is_finite() {
        return DEFAULT_WIDTH;
    }
    width.clamp(MIN_WIDTH, max.max(MIN_WIDTH))
}

/// When the row the panel shares with the conversation is narrower than the
/// panel's minimum plus the conversation's, the panel covers it instead (KSG's
/// sheet mode). The panel view uses it.
#[cfg_attr(not(test), allow(dead_code))]
pub fn covers(container: f32) -> bool {
    container < MIN_WIDTH + SIBLING_MIN
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Surface {
    Diff,
    Files,
    File {
        path: String,
        reveal_line: Option<u32>,
        reveal_request: u64,
    },
    Attachment {
        id: String,
        name: String,
    },
}

impl Surface {
    pub fn id(&self) -> String {
        match self {
            Self::Diff => "diff".to_string(),
            Self::Files => "files".to_string(),
            Self::File { path, .. } => format!("file:{path}"),
            Self::Attachment { id, .. } => format!("attachment:{id}"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SingletonKind {
    Diff,
    // The Files explorer's opener lands in a later change; tests construct it now.
    #[cfg_attr(not(test), allow(dead_code))]
    Files,
}

impl SingletonKind {
    fn into_surface(self) -> Surface {
        match self {
            Self::Diff => Surface::Diff,
            Self::Files => Surface::Files,
        }
    }

    fn matches_surface(self, surface: &Surface) -> bool {
        matches!(
            (self, surface),
            (Self::Diff, Surface::Diff) | (Self::Files, Surface::Files)
        )
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThreadPanel {
    pub open: bool,
    pub active: Option<String>,
    pub surfaces: Vec<Surface>,
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

    pub fn open(&mut self, thread: &str, kind: SingletonKind) {
        self.update_thread(thread, |current| {
            upsert_surface(current, kind.into_surface());
        });
    }

    // The diff rows and file explorer that open files land in a later change; the tests below
    // and the panel's layout tests exercise this now.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn open_file(&mut self, thread: &str, path: &str, line: Option<u32>) {
        self.update_thread(thread, |current| {
            if path == "." {
                upsert_surface(current, Surface::Files);
                return;
            }
            let relative_path = normalize_file_path(path);
            current
                .surfaces
                .retain(|surface| !matches!(surface, Surface::Files));
            let surface_id = format!("file:{relative_path}");
            let existing_request = current.surfaces.iter().find_map(|surface| match surface {
                Surface::File { reveal_request, .. } if surface.id() == surface_id => {
                    Some(*reveal_request)
                }
                _ => None,
            });
            let surface = Surface::File {
                path: relative_path,
                reveal_line: normalize_reveal_line(line),
                reveal_request: existing_request.unwrap_or(0).saturating_add(1),
            };
            current.open = true;
            current.active = Some(surface_id.clone());
            if let Some(slot) = current
                .surfaces
                .iter_mut()
                .find(|entry| matches!(entry, Surface::File { .. }) && entry.id() == surface_id)
            {
                *slot = surface;
            } else {
                current.surfaces.push(surface);
            }
        });
    }

    // Attachment surfaces open from a later change; tests exercise this now.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn open_attachment(&mut self, thread: &str, id: &str, name: &str) {
        self.update_thread(thread, |current| {
            current
                .surfaces
                .retain(|surface| !matches!(surface, Surface::Files));
            upsert_surface(
                current,
                Surface::Attachment {
                    id: id.to_string(),
                    name: name.to_string(),
                },
            );
        });
    }

    // The tab strip's press handler lands with the panel view; tests exercise this now.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn activate(&mut self, thread: &str, surface_id: &str) {
        self.update_thread(thread, |current| {
            if current
                .surfaces
                .iter()
                .any(|surface| surface.id() == surface_id)
            {
                current.open = true;
                current.active = Some(surface_id.to_string());
            }
        });
    }

    pub fn close_surface(&mut self, thread: &str, surface_id: &str) {
        self.update_thread(thread, |current| {
            let Some(index) = current
                .surfaces
                .iter()
                .position(|surface| surface.id() == surface_id)
            else {
                return;
            };
            current.surfaces.remove(index);
            current.open = !current.surfaces.is_empty() && current.open;
            if current.active.as_deref() != Some(surface_id) {
                return;
            }
            current.active = if current.surfaces.is_empty() {
                None
            } else {
                let fallback_index = index.min(current.surfaces.len().saturating_sub(1));
                current.surfaces.get(fallback_index).map(Surface::id)
            };
        });
    }

    // The tab context menu lands with the panel view; tests exercise this now.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn close_others(&mut self, thread: &str, surface_id: &str) {
        self.update_thread(thread, |current| {
            let Some(surface) = current
                .surfaces
                .iter()
                .find(|entry| entry.id() == surface_id)
                .cloned()
            else {
                return;
            };
            if current.surfaces.len() == 1 {
                return;
            }
            current.open = true;
            current.active = Some(surface.id());
            current.surfaces = vec![surface];
        });
    }

    // The tab context menu lands with the panel view; tests exercise this now.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn close_to_right(&mut self, thread: &str, surface_id: &str) {
        self.update_thread(thread, |current| {
            let Some(index) = current
                .surfaces
                .iter()
                .position(|surface| surface.id() == surface_id)
            else {
                return;
            };
            if index + 1 >= current.surfaces.len() {
                return;
            }
            current.surfaces.truncate(index + 1);
            let active_still_exists = current.active.as_deref().is_some_and(|active_id| {
                current
                    .surfaces
                    .iter()
                    .any(|surface| surface.id() == active_id)
            });
            if !active_still_exists {
                current.active = Some(surface_id.to_string());
            }
        });
    }

    // The tab context menu lands with the panel view; tests exercise this now.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn close_all(&mut self, thread: &str) {
        self.update_thread(thread, |current| {
            if current.surfaces.is_empty() {
                return;
            }
            current.open = false;
            current.surfaces.clear();
            current.active = None;
        });
    }

    // KSG reorders tabs by drag; that gesture lands with a later change. Tests exercise this now.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn move_surface(&mut self, thread: &str, surface_id: &str, to_index: usize) {
        self.update_thread(thread, |current| {
            let Some(from_index) = current
                .surfaces
                .iter()
                .position(|surface| surface.id() == surface_id)
            else {
                return;
            };
            if from_index == to_index {
                return;
            }
            let surface = current.surfaces.remove(from_index);
            let target_index = to_index.min(current.surfaces.len());
            current.surfaces.insert(target_index, surface);
        });
    }

    // The close-panel button lands with the panel view; tests exercise this now.
    #[cfg_attr(not(test), allow(dead_code))]
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

    pub fn toggle(&mut self, thread: &str, kind: SingletonKind) {
        self.update_thread(thread, |current| {
            let active_matches = current.active.as_deref().is_some_and(|active_id| {
                current
                    .surfaces
                    .iter()
                    .find(|surface| surface.id() == active_id)
                    .is_some_and(|surface| kind.matches_surface(surface))
            });
            if current.open && active_matches {
                current.open = false;
                return;
            }
            upsert_surface(current, kind.into_surface());
        });
    }

    pub fn active_surface(&self, thread: &str) -> Option<Surface> {
        let state = self.threads.get(thread)?;
        if !state.open {
            return None;
        }
        let active_id = state.active.as_deref()?;
        state
            .surfaces
            .iter()
            .find(|surface| surface.id() == active_id)
            .cloned()
    }

    pub fn forget(&mut self, thread: &str) {
        self.threads.remove(thread);
    }

    fn update_thread(&mut self, thread: &str, f: impl FnOnce(&mut ThreadPanel)) {
        let mut next = self.threads.get(thread).cloned().unwrap_or_default();
        f(&mut next);
        if !next.open && next.active.is_none() && next.surfaces.is_empty() {
            self.threads.remove(thread);
        } else if self.threads.get(thread) != Some(&next) {
            self.threads.insert(thread.to_string(), next);
        }
    }
}

fn upsert_surface(panel: &mut ThreadPanel, surface: Surface) {
    let id = surface.id();
    panel.open = true;
    if !panel.surfaces.iter().any(|entry| entry.id() == id) {
        panel.surfaces.push(surface);
    }
    panel.active = Some(id);
}

fn normalize_file_path(requested_path: &str) -> String {
    let is_windows_drive_root = match requested_path.as_bytes() {
        [drive, b':', rest @ ..] => {
            drive.is_ascii_alphabetic() && !rest.is_empty() && rest.iter().all(|&b| b == b'/')
        }
        _ => false,
    };
    if is_windows_drive_root {
        return requested_path.to_string();
    }
    let trimmed = requested_path.trim_end_matches('/');
    if trimmed.is_empty() {
        requested_path.to_string()
    } else {
        trimmed.to_string()
    }
}

/// KSG's `normalizeRevealLine`: a line below 1 reveals line 1.
fn normalize_reveal_line(line: Option<u32>) -> Option<u32> {
    line.map(|value| value.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    const THREAD_A: &str = "env-1:thread-A";
    const THREAD_B: &str = "env-1:thread-B";

    // KSG test: "opens workspace-root links as the singleton files explorer"
    #[test]
    fn ksg_opens_workspace_root_links_as_singleton_files_explorer() {
        let mut panels = RightPanels::default();
        panels.open_file(THREAD_A, "README.md", None);
        panels.open_file(THREAD_A, ".", None);
        panels.open_file(THREAD_A, ".", None);

        let state = panels.get(THREAD_A);
        assert_eq!(state.active.as_deref(), Some("files"));
        let ids: Vec<String> = state.surfaces.iter().map(Surface::id).collect();
        assert_eq!(ids, vec!["file:README.md", "files"]);

        panels.close_surface(THREAD_A, "files");
        assert_eq!(
            panels.get(THREAD_A).active.as_deref(),
            Some("file:README.md")
        );

        panels.open_file(THREAD_A, ".", None);
        assert_eq!(panels.get(THREAD_A).active.as_deref(), Some("files"));
    }

    // KSG test: "replaces the standalone explorer with peer file surfaces"
    #[test]
    fn ksg_replaces_standalone_explorer_with_peer_file_surfaces() {
        let mut panels = RightPanels::default();
        panels.open(THREAD_A, SingletonKind::Files);
        panels.open_file(THREAD_A, "src/index.ts", None);
        panels.open_file(THREAD_A, "src/index.ts", None);
        panels.open_file(THREAD_A, "README.md", None);

        assert_eq!(
            panels.get(THREAD_A),
            ThreadPanel {
                open: true,
                active: Some("file:README.md".to_string()),
                surfaces: vec![
                    Surface::File {
                        path: "src/index.ts".to_string(),
                        reveal_line: None,
                        reveal_request: 2,
                    },
                    Surface::File {
                        path: "README.md".to_string(),
                        reveal_line: None,
                        reveal_request: 1,
                    },
                ],
                maximized: false,
            }
        );
    }

    // KSG test: "keeps %j and %j in separate file tabs"
    #[test]
    fn ksg_keeps_distinct_paths_in_separate_file_tabs() {
        for (first_path, second_path) in [
            ("generated\\", "generated"),
            ("notes/meeting ", "notes/meeting"),
            (" notes/meeting", "notes/meeting"),
        ] {
            let mut panels = RightPanels::default();
            panels.open_file(THREAD_A, first_path, None);
            panels.open_file(THREAD_A, second_path, None);

            assert_eq!(
                panels.get(THREAD_A).surfaces,
                vec![
                    Surface::File {
                        path: first_path.to_string(),
                        reveal_line: None,
                        reveal_request: 1,
                    },
                    Surface::File {
                        path: second_path.to_string(),
                        reveal_line: None,
                        reveal_request: 1,
                    },
                ]
            );
        }
    }

    // KSG test: "keeps attachment and workspace file ids disjoint"
    #[test]
    fn ksg_keeps_attachment_and_workspace_file_ids_disjoint() {
        let mut panels = RightPanels::default();
        panels.open_file(THREAD_A, "attachment:shared-id", None);
        panels.open_attachment(THREAD_A, "shared-id", "report.pdf");

        let ids: Vec<String> = panels
            .get(THREAD_A)
            .surfaces
            .iter()
            .map(Surface::id)
            .collect();
        assert_eq!(
            ids,
            vec!["file:attachment:shared-id", "attachment:shared-id"]
        );
    }

    // KSG test: "updates line reveal requests when reopening a file surface"
    #[test]
    fn ksg_updates_line_reveal_requests_when_reopening_a_file_surface() {
        let mut panels = RightPanels::default();
        panels.open_file(THREAD_A, "src/index.ts", Some(42));
        panels.open_file(THREAD_A, "src/index.ts", Some(87));

        assert_eq!(
            panels.get(THREAD_A),
            ThreadPanel {
                open: true,
                active: Some("file:src/index.ts".to_string()),
                surfaces: vec![Surface::File {
                    path: "src/index.ts".to_string(),
                    reveal_line: Some(87),
                    reveal_request: 2,
                }],
                maximized: false,
            }
        );

        panels.open_file(THREAD_A, "src/index.ts", None);

        assert_eq!(
            panels.get(THREAD_A),
            ThreadPanel {
                open: true,
                active: Some("file:src/index.ts".to_string()),
                surfaces: vec![Surface::File {
                    path: "src/index.ts".to_string(),
                    reveal_line: None,
                    reveal_request: 3,
                }],
                maximized: false,
            }
        );
    }

    // Required test 1
    #[test]
    fn open_diff_on_empty_thread() {
        let mut panels = RightPanels::default();
        panels.open(THREAD_A, SingletonKind::Diff);

        assert_eq!(
            panels.get(THREAD_A),
            ThreadPanel {
                open: true,
                active: Some("diff".to_string()),
                surfaces: vec![Surface::Diff],
                maximized: false,
            }
        );
        assert_eq!(panels.active_surface(THREAD_A), Some(Surface::Diff));
    }

    // Required test 2
    #[test]
    fn open_diff_twice_gives_one_surface() {
        let mut panels = RightPanels::default();
        panels.open(THREAD_A, SingletonKind::Diff);
        panels.open(THREAD_A, SingletonKind::Diff);

        let state = panels.get(THREAD_A);
        assert_eq!(state.surfaces, vec![Surface::Diff]);
        assert_eq!(state.active.as_deref(), Some("diff"));
        assert!(state.open);
    }

    // Required test 3
    #[test]
    fn open_file_two_paths_then_close_active_surface() {
        let mut panels = RightPanels::default();
        panels.open_file(THREAD_A, "src/a.rs", None);
        panels.open_file(THREAD_A, "src/b.rs", None);
        assert_eq!(
            panels.get(THREAD_A).active.as_deref(),
            Some("file:src/b.rs")
        );

        panels.close_surface(THREAD_A, "file:src/b.rs");
        let state = panels.get(THREAD_A);
        assert!(state.open);
        assert_eq!(state.active.as_deref(), Some("file:src/a.rs"));
        assert_eq!(
            state.surfaces,
            vec![Surface::File {
                path: "src/a.rs".to_string(),
                reveal_line: None,
                reveal_request: 1,
            }]
        );

        // Closing a middle active tab activates the neighbor at the same index (to the right).
        panels.open_file(THREAD_A, "src/b.rs", None);
        panels.open_file(THREAD_A, "src/c.rs", None);
        panels.activate(THREAD_A, "file:src/b.rs");
        panels.close_surface(THREAD_A, "file:src/b.rs");
        assert_eq!(
            panels.get(THREAD_A).active.as_deref(),
            Some("file:src/c.rs")
        );
    }

    // Required test 4
    #[test]
    fn close_surface_of_last_surface() {
        let mut panels = RightPanels::default();
        panels.open(THREAD_A, SingletonKind::Diff);
        panels.set_maximized(THREAD_A, true);
        panels.close_surface(THREAD_A, "diff");

        assert_eq!(panels.get(THREAD_A), ThreadPanel::default());
        assert_eq!(panels.active_surface(THREAD_A), None);
    }

    // Required test 5
    #[test]
    fn close_others_close_to_right_and_close_all() {
        let mut panels = RightPanels::default();
        panels.open(THREAD_A, SingletonKind::Diff);
        panels.open_file(THREAD_A, "a.rs", None);
        panels.open_file(THREAD_A, "b.rs", None);
        panels.open_file(THREAD_A, "c.rs", None);

        // close_to_right when active surface is among the closed tabs activates the target tab.
        panels.close_to_right(THREAD_A, "file:a.rs");
        let after_right = panels.get(THREAD_A);
        assert_eq!(
            after_right
                .surfaces
                .iter()
                .map(Surface::id)
                .collect::<Vec<_>>(),
            vec!["diff", "file:a.rs"]
        );
        assert_eq!(after_right.active.as_deref(), Some("file:a.rs"));

        // close_to_right when active surface stays keeps that active surface.
        panels.open_file(THREAD_A, "b.rs", None);
        panels.activate(THREAD_A, "diff");
        panels.close_to_right(THREAD_A, "file:a.rs");
        assert_eq!(panels.get(THREAD_A).active.as_deref(), Some("diff"));

        // close_others keeps only the chosen surface and activates it.
        panels.close_others(THREAD_A, "file:a.rs");
        let after_others = panels.get(THREAD_A);
        assert!(after_others.open);
        assert_eq!(after_others.active.as_deref(), Some("file:a.rs"));
        assert_eq!(
            after_others
                .surfaces
                .iter()
                .map(Surface::id)
                .collect::<Vec<_>>(),
            vec!["file:a.rs"]
        );

        // close_all empties surfaces, clears active, and closes the panel.
        panels.close_all(THREAD_A);
        assert_eq!(panels.get(THREAD_A), ThreadPanel::default());
    }

    // Required test 6
    #[test]
    fn move_surface_edge_cases() {
        let mut panels = RightPanels::default();
        panels.open(THREAD_A, SingletonKind::Diff);
        panels.open_file(THREAD_A, "a.rs", None);
        panels.open_file(THREAD_A, "b.rs", None);

        let ids = |panels: &RightPanels| -> Vec<String> {
            panels
                .get(THREAD_A)
                .surfaces
                .iter()
                .map(Surface::id)
                .collect()
        };

        panels.move_surface(THREAD_A, "file:b.rs", 0);
        assert_eq!(ids(&panels), vec!["file:b.rs", "diff", "file:a.rs"]);

        panels.move_surface(THREAD_A, "file:b.rs", 999);
        assert_eq!(ids(&panels), vec!["diff", "file:a.rs", "file:b.rs"]);

        panels.move_surface(THREAD_A, "file:missing.rs", 0);
        assert_eq!(ids(&panels), vec!["diff", "file:a.rs", "file:b.rs"]);
    }

    // Required test 7
    #[test]
    fn toggle_diff_states() {
        let mut panels = RightPanels::default();

        // When closed (empty): opens diff and activates it.
        panels.toggle(THREAD_A, SingletonKind::Diff);
        let state = panels.get(THREAD_A);
        assert!(state.open);
        assert_eq!(state.active.as_deref(), Some("diff"));
        assert_eq!(state.surfaces, vec![Surface::Diff]);

        // When open on diff: closes panel while keeping diff in surfaces and active.
        panels.toggle(THREAD_A, SingletonKind::Diff);
        let closed_state = panels.get(THREAD_A);
        assert!(!closed_state.open);
        assert_eq!(closed_state.active.as_deref(), Some("diff"));
        assert_eq!(closed_state.surfaces, vec![Surface::Diff]);

        // When closed with diff already present: reopens on diff.
        panels.toggle(THREAD_A, SingletonKind::Diff);
        assert!(panels.get(THREAD_A).open);

        // When open on a file: switches active to diff and keeps panel open.
        panels.open_file(THREAD_A, "main.rs", None);
        assert_eq!(panels.get(THREAD_A).active.as_deref(), Some("file:main.rs"));
        panels.toggle(THREAD_A, SingletonKind::Diff);
        let switched = panels.get(THREAD_A);
        assert!(switched.open);
        assert_eq!(switched.active.as_deref(), Some("diff"));
        assert_eq!(switched.surfaces.len(), 2);
    }

    // Required test 8
    #[test]
    fn toggle_visibility_with_and_without_surfaces() {
        let mut panels = RightPanels::default();

        // Without surfaces: toggles open to true, then back to false (and removes empty entry).
        panels.toggle_visibility(THREAD_A);
        assert_eq!(
            panels.get(THREAD_A),
            ThreadPanel {
                open: true,
                active: None,
                surfaces: vec![],
                maximized: false,
            }
        );
        panels.toggle_visibility(THREAD_A);
        assert_eq!(panels.get(THREAD_A), ThreadPanel::default());

        // With surfaces: hides and re-shows while preserving surfaces and active.
        panels.open(THREAD_A, SingletonKind::Diff);
        panels.toggle_visibility(THREAD_A);
        let hidden = panels.get(THREAD_A);
        assert!(!hidden.open);
        assert_eq!(hidden.active.as_deref(), Some("diff"));
        assert_eq!(hidden.surfaces, vec![Surface::Diff]);
        assert_eq!(panels.active_surface(THREAD_A), None);

        panels.toggle_visibility(THREAD_A);
        assert!(panels.get(THREAD_A).open);
        assert_eq!(panels.active_surface(THREAD_A), Some(Surface::Diff));
    }

    // Required test 9
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

    // Required test 10
    #[test]
    fn open_file_same_path_twice_with_different_lines() {
        let mut panels = RightPanels::default();
        panels.open_file(THREAD_A, "src/lib.rs", Some(10));
        panels.open_file(THREAD_A, "src/lib.rs", Some(25));

        let state = panels.get(THREAD_A);
        assert_eq!(state.surfaces.len(), 1);
        assert_eq!(
            state.surfaces,
            vec![Surface::File {
                path: "src/lib.rs".to_string(),
                reveal_line: Some(25),
                reveal_request: 2,
            }]
        );
    }

    // Required test 11
    #[test]
    fn threads_are_isolated_and_forget_removes_one() {
        let mut panels = RightPanels::default();
        panels.open(THREAD_A, SingletonKind::Diff);
        panels.set_maximized(THREAD_A, true);
        panels.open_file(THREAD_B, "src/main.rs", Some(5));

        assert_eq!(panels.active_surface(THREAD_A), Some(Surface::Diff));
        assert_eq!(
            panels.active_surface(THREAD_B),
            Some(Surface::File {
                path: "src/main.rs".to_string(),
                reveal_line: Some(5),
                reveal_request: 1,
            })
        );

        panels.forget(THREAD_A);
        assert_eq!(panels.get(THREAD_A), ThreadPanel::default());
        assert_eq!(panels.active_surface(THREAD_A), None);
        assert!(panels.get(THREAD_B).open);
    }

    // Required test 12
    #[test]
    fn unknown_surface_ids_change_nothing() {
        let mut panels = RightPanels::default();
        panels.open(THREAD_A, SingletonKind::Diff);
        panels.open_file(THREAD_A, "a.rs", None);
        let before = panels.get(THREAD_A);

        panels.activate(THREAD_A, "file:missing.rs");
        assert_eq!(panels.get(THREAD_A), before);

        panels.close_surface(THREAD_A, "file:missing.rs");
        assert_eq!(panels.get(THREAD_A), before);

        panels.close_others(THREAD_A, "file:missing.rs");
        assert_eq!(panels.get(THREAD_A), before);

        panels.close_to_right(THREAD_A, "file:missing.rs");
        assert_eq!(panels.get(THREAD_A), before);

        // Also verify on an absent thread: none of these create an entry.
        panels.activate(THREAD_B, "diff");
        panels.close_surface(THREAD_B, "diff");
        panels.close_others(THREAD_B, "diff");
        panels.close_to_right(THREAD_B, "diff");
        assert_eq!(panels.get(THREAD_B), ThreadPanel::default());
    }

    #[test]
    fn open_file_normalizes_trailing_slashes_and_line_zero() {
        let mut panels = RightPanels::default();
        panels.open_file(THREAD_A, "src/dir/", Some(0));
        panels.open_file(THREAD_A, "C:/", None);
        panels.open_file(THREAD_A, "/", None);

        assert_eq!(
            panels.get(THREAD_A).surfaces,
            vec![
                Surface::File {
                    path: "src/dir".to_string(),
                    reveal_line: Some(1),
                    reveal_request: 1,
                },
                Surface::File {
                    path: "C:/".to_string(),
                    reveal_line: None,
                    reveal_request: 1,
                },
                Surface::File {
                    path: "/".to_string(),
                    reveal_line: None,
                    reveal_request: 1,
                },
            ]
        );
    }

    #[test]
    fn activate_reopens_closed_panel_and_close_others_on_single_closed_surface_noop() {
        let mut panels = RightPanels::default();
        panels.open(THREAD_A, SingletonKind::Diff);
        panels.close(THREAD_A);
        assert!(!panels.get(THREAD_A).open);

        // close_others on a single-surface closed panel is a no-op.
        panels.close_others(THREAD_A, "diff");
        assert!(!panels.get(THREAD_A).open);

        // activate opens the panel on the existing surface.
        panels.activate(THREAD_A, "diff");
        assert!(panels.get(THREAD_A).open);
        assert_eq!(panels.active_surface(THREAD_A), Some(Surface::Diff));
    }

    #[test]
    fn set_maximized_on_empty_closed_thread_is_evicted() {
        let mut panels = RightPanels::default();
        panels.set_maximized(THREAD_A, true);
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
