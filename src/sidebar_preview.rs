//! One sidebar preview owns dwell, leave grace and placement for the window.
use super::sidebar_row::RowStatus;
use super::sidebar_ui::{self, RowId};
use super::*;
use rune::Transition;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

const DWELL: Duration = Duration::from_millis(350);
const CHAIN: Duration = Duration::from_millis(100);
const LEAVE: Duration = Duration::from_millis(120);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    Hover,
    Focus,
}

#[derive(Default)]
pub(crate) struct SidebarPreview {
    pending: Option<(RowId, Source, Instant)>,
    open: Option<(RowId, Source)>,
    hot: Option<RowId>,
    keyboard_actions: Option<usize>,
    card_hot: bool,
    muted: Option<RowId>,
    leave: Option<Instant>,
    tab: bool,
    at_scroll: f32,
    pub(super) scroll: Arc<AtomicU32>,
    pub(super) watching_scroll: Arc<AtomicBool>,
}

pub(super) fn row_actions_open(s: &State, id: &RowId) -> bool {
    s.sidebar_preview.hot.as_ref() == Some(id)
        || (id.server == s.machines.selected
            && s.sidebar_preview.keyboard_actions == Some(id.thread))
}

pub(crate) fn dismiss(s: &mut State, mute: bool) -> bool {
    let preview = &mut s.sidebar_preview;
    if mute {
        preview.muted = preview.hot.clone();
    }
    preview.card_hot = false;
    preview.leave = None;
    preview.watching_scroll.store(false, Ordering::Relaxed);
    let pending = preview.pending.take().is_some();
    let open = preview.open.take().is_some();
    pending || open
}

fn begin(s: &mut State, target: RowId, source: Source, tx: &mut rune::Tx) {
    if !s.preview_allowed() || !sidebar_docked(s) {
        return;
    }
    let preview = &mut s.sidebar_preview;
    if preview.muted.as_ref() == Some(&target)
        || preview
            .open
            .as_ref()
            .is_some_and(|(open, _)| open == &target)
    {
        return;
    }
    let wait = if preview.open.is_some() { CHAIN } else { DWELL };
    preview.pending = Some((target, source, Instant::now() + wait));
    preview.at_scroll = f32::from_bits(preview.scroll.load(Ordering::Relaxed));
    preview.watching_scroll.store(true, Ordering::Relaxed);
    tx.wake_after(wait);
}

pub(super) fn row_hover(s: &mut State, target: &RowId, hot: bool, tx: &mut rune::Tx) {
    let preview = &mut s.sidebar_preview;
    if hot {
        preview.hot = Some(target.clone());
        begin(s, target.clone(), Source::Hover, tx);
    } else {
        if preview.muted.as_ref() == Some(target) {
            preview.muted = None;
        }
        if preview.hot.as_ref() == Some(target) {
            preview.hot = None;
        }
        if preview
            .pending
            .as_ref()
            .is_some_and(|(pending, source, _)| pending == target && *source == Source::Hover)
        {
            preview.pending = None;
        }
        if preview
            .open
            .as_ref()
            .is_some_and(|(_, source)| *source == Source::Hover)
            && !preview.card_hot
        {
            preview.leave = Some(Instant::now() + LEAVE);
            tx.wake_after(LEAVE);
        }
    }
}

pub(super) fn keyboard(s: &mut State, target: RowId, tx: &mut rune::Tx) {
    begin(s, target, Source::Focus, tx);
}

pub(super) fn after_tab(s: &mut State, tx: &mut rune::Tx) {
    s.sidebar_preview.tab = true;
    tx.wake_after(Duration::ZERO);
}

pub(crate) fn poll(s: &mut State, tx: &mut rune::Tx) -> bool {
    let keyboard_actions = tx.focused().filter(|_| tx.focus_visible()).and_then(|key| {
        [
            "thread-",
            "thread-park-",
            "thread-snooze-",
            "thread-discard-",
        ]
        .into_iter()
        .find_map(|prefix| key.strip_prefix(prefix)?.parse::<usize>().ok())
    });
    let changed = keyboard_actions != s.sidebar_preview.keyboard_actions;
    s.sidebar_preview.keyboard_actions = keyboard_actions;
    poll_at(s, tx, Instant::now()) || changed
}

fn poll_at(s: &mut State, tx: &mut rune::Tx, now: Instant) -> bool {
    if !s.preview_allowed() || !sidebar_docked(s) {
        return dismiss(s, true);
    }
    let mut changed = false;
    if std::mem::take(&mut s.sidebar_preview.tab) {
        if let Some(target) = tx.focused().and_then(|key| sidebar_ui::focused_row(s, key)) {
            if s.sidebar_preview
                .open
                .as_ref()
                .is_some_and(|(open, _)| open != &target)
            {
                changed = dismiss(s, false);
            }
            keyboard(s, target, tx);
        } else {
            changed = dismiss(s, false);
        }
    }
    let preview = &s.sidebar_preview;
    if preview.watching_scroll.load(Ordering::Relaxed)
        && f32::from_bits(preview.scroll.load(Ordering::Relaxed)) != preview.at_scroll
    {
        return dismiss(s, true) || changed;
    }
    if preview.open.as_ref().is_some_and(|(id, source)| {
        !sidebar_ui::indexed(s, id)
            || *source == Source::Focus && tx.focused() != Some(id.key(s).as_str())
    }) {
        return dismiss(s, false) || changed;
    }
    let preview = &mut s.sidebar_preview;
    if let Some((target, source, deadline)) = &preview.pending {
        let wait = deadline.saturating_duration_since(now);
        if !wait.is_zero() {
            tx.wake_after(wait);
        } else {
            let target = target.clone();
            let source = *source;
            preview.pending = None;
            let still_target = if source == Source::Hover {
                preview.hot.as_ref() == Some(&target)
            } else {
                tx.focused() == Some(target.key(s).as_str())
            };
            if still_target && sidebar_ui::indexed(s, &target) {
                s.sidebar_preview.open = Some((target, source));
                s.sidebar_preview.leave = None;
                changed = true;
            }
        }
    }
    let preview = &mut s.sidebar_preview;
    if let Some(deadline) = preview.leave {
        let wait = deadline.saturating_duration_since(now);
        if wait.is_zero()
            && !preview.card_hot
            && preview
                .open
                .as_ref()
                .is_some_and(|(target, _)| preview.hot.as_ref() != Some(target))
        {
            preview.leave = None;
            changed |= preview.open.take().is_some();
        } else if !wait.is_zero() {
            tx.wake_after(wait);
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_dwell_chain_leave_and_bounds() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = crate::ui::tests::runtime(width);
            rt.state.populate_demo();
            rt.state.compact_sidebar = width < 760.0;
            rt.frame(0.0);
            rt.frame(1.0);
            let list = rt.presented("sidebar-scroll").expect("sidebar").0;
            let order = sidebar_navigation(&rt.state);
            let first = RowId::new(&order[0].0, order[0].1);
            let second = RowId::new(&order[1].0, order[1].1);
            rt.update(|s, tx| row_hover(s, &first, true, tx));
            let at = rt.state.sidebar_preview.pending.as_ref().unwrap().2;
            rt.update(|s, tx| assert!(!poll_at(s, tx, at - Duration::from_millis(50))));
            rt.frame(0.0);
            assert!(rt.presented("sidebar-preview").is_none());
            rt.update(|s, tx| assert!(poll_at(s, tx, at + Duration::from_millis(50))));
            rt.frame(0.0);
            rt.frame(1.0);
            let (card, _) = rt.presented("sidebar-preview").expect("open card");
            assert!(card[0] >= -0.5 && card[0] + card[2] <= width + 0.5);
            assert!(card[1] >= -0.5 && card[1] + card[3] <= 720.5);
            assert_eq!(rt.presented("sidebar-scroll").unwrap().0, list);
            rt.update(|s, tx| {
                row_hover(s, &first, false, tx);
                row_hover(s, &second, true, tx);
            });
            let next = rt.state.sidebar_preview.pending.as_ref().unwrap().2;
            assert!(next.saturating_duration_since(Instant::now()) <= CHAIN);
            rt.update(|s, tx| assert!(poll_at(s, tx, next)));
            assert_eq!(rt.state.sidebar_preview.open.as_ref().unwrap().0, second);
            rt.update(|s, tx| {
                row_hover(s, &second, false, tx);
                s.sidebar_preview.card_hot = true;
                assert!(!poll_at(s, tx, next + LEAVE + LEAVE));
                s.sidebar_preview.card_hot = false;
                assert!(poll_at(s, tx, next + LEAVE + LEAVE));
            });
            rt.frame(1.0);
            assert!(rt.presented("sidebar-preview").is_none());
        }
    }

    #[test]
    fn ordinary_thread_rows_keep_their_portrait_and_actions_clear_of_the_title() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = crate::ui::tests::runtime(width);
            rt.state.compact_sidebar = width < 760.0;
            let mut thread = Thread::new(900);
            thread.title = "An intentionally long sidebar thread title for the hover card".into();
            thread.model = "Claude Fable 5.2".into();
            thread.status = Status::Completed;
            thread.life.latest_user_message_at = Some(lifecycle::now());
            thread.messages.push(crate::model::Message {
                id: "preview-output".into(),
                role: "Pi",
                text: "Raw transcript must not appear in the sidebar preview".into(),
                images: vec![],
                context: None,
            });
            rt.state.threads.push(thread);
            rt.frame(0.0);
            rt.frame(1.0);
            let idle_title = rt.presented("thread-row-title-900").unwrap().0;
            assert!(rt.presented("thread-park-900").unwrap().1 < 0.01, "{width}");
            rt.update(|_, tx| tx.focus("thread-900", true));
            rt.frame(1.0);
            rt.update(|s, tx| assert!(poll(s, tx)));
            rt.frame(1.0);
            assert!(rt.presented("thread-park-900").unwrap().1 > 0.99, "{width}");
            let (x, y) = rt.find("thread-900").unwrap();
            rt.pointer_move(x, y);
            rt.frame(1.0);
            let portrait = rt.presented("thread-provider-portrait-900").unwrap().0;
            let title = rt.presented("thread-row-title-900").unwrap().0;
            let snooze = rt.presented("thread-snooze-900").unwrap().0;
            let settle = rt.presented("thread-park-900").unwrap().0;
            assert!(idle_title[2] > title[2], "{width}");
            assert!(portrait[0] + portrait[2] <= title[0], "{width}");
            assert!(title[0] + title[2] <= snooze[0] + 0.5, "{width}");
            assert!(snooze[0] + snooze[2] <= settle[0] + 0.5, "{width}");
            assert!(
                !rt.accessibility_json()
                    .to_string()
                    .contains("Raw transcript")
            );

            let row = RowId::new("local", 900);
            rt.update(|s, tx| row_hover(s, &row, true, tx));
            let at = rt.state.sidebar_preview.pending.as_ref().unwrap().2;
            rt.update(|s, tx| assert!(poll_at(s, tx, at)));
            rt.frame(0.0);
            rt.frame(1.0);
            assert!(rt.presented("sidebar-preview-title").is_some(), "{width}");
            assert!(
                !rt.accessibility_json()
                    .to_string()
                    .contains("Raw transcript")
            );
        }
    }

    #[test]
    fn preview_mutes_pressed_rows_and_cancels_for_scroll_or_overlays() {
        let mut rt = crate::ui::tests::runtime(1280.0);
        rt.state.populate_demo();
        rt.frame(0.0);
        let (server, id) = sidebar_navigation(&rt.state)[0].clone();
        let row = RowId::new(&server, id);
        rt.update(|s, tx| {
            row_hover(s, &row, true, tx);
            row_hover(s, &row, false, tx);
            assert!(
                s.sidebar_preview.pending.is_none(),
                "a quick pass does nothing"
            );
            row_hover(s, &row, true, tx);
            assert!(dismiss(s, true), "a press suppresses the pending card");
            row_hover(s, &row, true, tx);
            assert!(s.sidebar_preview.pending.is_none());
            row_hover(s, &row, false, tx);
            row_hover(s, &row, true, tx);
            s.sidebar_preview
                .scroll
                .store(42.0f32.to_bits(), Ordering::Relaxed);
            assert!(poll(s, tx), "scroll cancels an active dwell");
            row_hover(s, &row, false, tx);
            s.quick_switcher = true;
            row_hover(s, &row, true, tx);
            assert!(s.sidebar_preview.pending.is_none());
        });
    }

    #[test]
    fn keyboard_tab_starts_a_preview_dwell_without_a_pointer_press() {
        let mut rt = crate::ui::tests::runtime(1280.0);
        rt.state.populate_demo();
        rt.frame(0.0);
        rt.frame(1.0);
        rt.update(|_, tx| tx.focus("assign-captain", true));
        rt.key(rune::KeyEvent {
            key: rune::Key::Tab,
            mods: Default::default(),
        });
        rt.frame(0.0);
        rt.update(|s, tx| {
            poll(s, tx);
        });
        assert!(
            rt.state
                .sidebar_preview
                .pending
                .as_ref()
                .is_some_and(|(_, source, _)| *source == Source::Focus)
        );
        let (row, _, at) = rt.state.sidebar_preview.pending.clone().unwrap();
        rt.update(|s, tx| assert!(poll_at(s, tx, at)));
        rt.frame(1.0);
        assert!(rt.presented("sidebar-preview").is_some());
        let key = row.key(&rt.state);
        crate::ui::tests::press(&mut rt, &key);
        assert!(rt.state.sidebar_preview.open.is_none());
        assert!(
            !rt.state
                .sidebar_preview
                .pending
                .as_ref()
                .is_some_and(|(_, source, _)| *source == Source::Focus)
        );
    }
}

fn fact(p: Palette, glyph: &str, title: &str, mono: bool) -> El {
    row()
        .h(20.0)
        .gap(8.0)
        .center_y()
        .child(icon(glyph, p.faint).size(14.0, 14.0))
        .child(
            label(title, if mono { MONO_META } else { SMALL }, p.muted)
                .truncate_middle()
                .grow(1.0)
                .min_w(0.0),
        )
}

/// Ported from KSG Code's `ThreadHoverCardPopup`. Show facts and status;
/// actions stay on the row or in the conversation.
pub(super) fn view(s: &State, p: Palette, sidebar_width: f32) -> Option<El> {
    if !s.preview_allowed() {
        return None;
    }
    let (id, _) = s.sidebar_preview.open.as_ref()?;
    let local = id.server == s.machines.selected;
    let thread = local
        .then(|| s.threads.iter().find(|t| t.id == id.thread))
        .flatten();
    let remote = (!local)
        .then(|| sidebar_ui::preview_record(s, id))
        .flatten();
    let title = thread
        .map(|t| t.title.as_str())
        .or_else(|| remote?.0["title"].as_str())
        .unwrap_or("Conversation");
    let status = thread
        .map(|t| {
            if s.is_captain_home(t) {
                s.home_status(t.id)
            } else {
                t.status
            }
        })
        .or_else(|| {
            remote.and_then(|(_, view)| serde_json::from_value(view.record["status"].clone()).ok())
        })
        .unwrap_or(Status::Ready);
    let request = thread
        .and_then(|t| t.request.as_ref())
        .or_else(|| remote.map(|(_, v)| &v.record["request"]))
        .filter(|request| !request.is_null());
    let state = RowStatus::of(
        if request.is_some() {
            Status::Waiting
        } else {
            status
        },
        request.is_some_and(|request| request["method"] == "confirm"),
        thread.is_some_and(Thread::unread),
        false,
    );
    let captain = thread
        .filter(|t| s.is_captain_home(t))
        .and_then(|t| t.captain);
    let profile = captain.and_then(|captain| s.captains.get(captain));
    let name = profile.map_or(title, |profile| profile.name.as_str());
    let elapsed = thread.and_then(|t| t.started.map(|at| at.elapsed().as_secs()).or(t.elapsed));
    let word = if state == RowStatus::Working {
        elapsed.map_or_else(
            || "Working".into(),
            |elapsed| format!("Working for {}", timer_label(elapsed)),
        )
    } else {
        state.word().into()
    };
    let mut title_block = col().gap(2.0).grow(1.0).min_w(0.0).child(
        label(
            name,
            if profile.is_some() {
                IDENTITY
            } else {
                CARD_TITLE
            },
            p.ink,
        )
        .key("sidebar-preview-title")
        .wrap()
        .max_h(60.0),
    );
    if let Some(profile) = profile {
        title_block = title_block.child(label(&profile.role, SMALL, p.muted));
    }
    title_block = title_block.child(row().center_y().gap(6.0).child(state.mark(p)).child(label(
        word,
        SMALL,
        status_ink(status, p),
    )));
    let mut header = row().center_y().gap(12.0);
    if let Some(profile) = profile {
        header = header.child(haloed(
            s,
            p,
            captain_face(s, p, profile.portrait, face_for(status), 48.0),
            48.0,
            status,
        ));
    }
    let mut card = col()
        .gap(8.0)
        .child(header.child(title_block))
        .child(rule(p));
    if let Some(project) = thread.and_then(|t| s.project(t.project)) {
        card = card.child(fact(p, "folder", &project.name, false));
    }
    if let Some(branch) = thread
        .and_then(|t| t.branch.as_deref())
        .or_else(|| remote?.0["branch"].as_str())
    {
        card = card.child(fact(p, "branch", branch, true));
    }
    card = card.child(fact(p, "monitor", &machine_caption(s, &id.server), false));
    if let Some(model) = thread
        .map(|t| t.model.as_str())
        .or_else(|| remote?.1.record["model"].as_str())
        .filter(|model| !model.is_empty())
    {
        card = card.child(fact(p, "chip", model, false));
    }
    if let Some(t) = thread {
        let runs = crate::agents::runs(t);
        if !runs.is_empty() {
            card = card
                .child(rule(p))
                .child(label("Agents", META_LABEL, p.faint));
            for (i, run) in runs.iter().take(3).enumerate() {
                let (status, word) = agents_ui::status(run.state);
                card = card.child(
                    row()
                        .key(format!("sidebar-preview-agent-{i}"))
                        .h(22.0)
                        .center_y()
                        .gap(6.0)
                        .child(status_mark(status, p, 8.0))
                        .child(
                            label(run.name.as_ref(), SMALL, p.ink)
                                .truncate()
                                .grow(1.0)
                                .min_w(0.0),
                        )
                        .child(label(word, SMALL, status_ink(status, p))),
                );
            }
            if runs.len() > 3 {
                card = card.child(label(format!("{} more", runs.len() - 3), SMALL, p.faint));
            }
        }
        if let Some(captain) = captain {
            let crew = crew_ui::roster_members(s, t.id, captain);
            if !crew.is_empty() {
                card = card
                    .child(rule(p))
                    .child(label("Crew", META_LABEL, p.faint));
                for member in crew.iter().take(4) {
                    card = card.child(
                        label(
                            format!("{} · {}", member.title, member.word),
                            SMALL,
                            p.muted,
                        )
                        .truncate(),
                    );
                }
                if crew.len() > 4 {
                    card = card.child(label(format!("{} more", crew.len() - 4), SMALL, p.faint));
                }
            }
        }
    }
    if let Some(request) = request {
        let summary = request["params"]["message"]
            .as_str()
            .or_else(|| request["message"].as_str())
            .unwrap_or("This conversation is waiting for your reply.");
        card = card.child(
            col()
                .padding(8.0)
                .gap(4.0)
                .radius(8.0)
                .fill(p.brass_bg)
                .child(label(truncate(summary, 140), SMALL, p.brass).wrap()),
        );
    }
    if status == Status::Failed
        && let Some(error) = thread
            .and_then(|t| t.error.as_deref())
            .or_else(|| remote?.1.record["error"].as_str())
    {
        card = card.child(
            label(truncate(error, 140), SMALL, p.danger)
                .key("sidebar-preview-error")
                .wrap(),
        );
    }
    let side = if s.viewport_width - sidebar_width - 16.0 >= 304.0 {
        rune::Side::Right
    } else {
        rune::Side::Bottom
    };
    Some(
        card.key("sidebar-preview")
            .role("tooltip")
            .label(format!("Preview of {name}"))
            .padding(14.0)
            .w((s.viewport_width - 16.0).min(300.0))
            .max_h((s.viewport_height - 32.0).min(520.0))
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .radius(12.0)
            .fill(p.card)
            .border(1.0, p.frame)
            .box_shadow(soft_shadow(p))
            .anchored_fit(id.key(s), side, rune::Align::Start, 4.0)
            .opacity_motion(MOTION_BASE)
            .offset_motion(MOTION_BASE)
            .transition(if s.portraits.reduced_motion() {
                Transition::opacity()
            } else {
                Transition::offset(0.0, 4.0).combined(Transition::opacity())
            })
            .hover_delays(Duration::ZERO, LEAVE)
            .on_hover_change(|s: &mut State, hot, tx| {
                s.sidebar_preview.card_hot = hot;
                if !hot && s.sidebar_preview.hot.is_none() {
                    s.sidebar_preview.leave = Some(Instant::now());
                    tx.wake_after(Duration::ZERO);
                }
            }),
    )
}
