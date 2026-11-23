use super::*;

/// The opaque strip under the chat header's rule, inside the transcript viewport.
pub(super) const TRANSCRIPT_CAP: f32 = 8.0;

/// The sidebar portraits' animation stage also plays the captain hero, under this row id.
pub(crate) const HERO_PORTRAIT_ROW: usize = usize::MAX;
pub(super) const HERO_PORTRAIT: f32 = 96.0;
/// The captain hero with a one-line heading: portrait, heading, role and status.
pub(super) const CAPTAIN_HERO_HEIGHT: f32 = 210.0;

/// An empty captain home greets like KSG Code's draft hero (`DraftHeroHeadline`: centred
/// `text-3xl font-normal tracking-tight`, `text-2xl` on a narrow pane), with the captain's
/// animated portrait above it. A home with a goal shows the goal; otherwise the captain asks
/// what to look after. The live status keeps one 20 px line, so it never moves the composer.
pub(super) fn captain_hero(s: &State, p: Palette, captain: usize, pane_width: f32) -> El {
    let t = s.current();
    let profile = &s.captains[captain];
    let status = shown_status(s);
    let face = face_for(status);
    let portrait_index = captain_portrait(s, Some(captain));
    let size = HERO_PORTRAIT;
    let pixels = s
        .portraits
        .pixels(portrait_index, face, HERO_PORTRAIT)
        .unwrap_or_else(|| s.artwork.face_pixels(portrait_index, face, HERO_PORTRAIT));
    let portrait = portrait(
        &pixels,
        if s.artwork.is_pixel_art(portrait_index) {
            s.artwork.snap_points(size)
        } else {
            size
        },
        p,
        None,
    )
    .key("captain-hero-portrait")
    .a11y_hidden()
    .on_visible(|s: &mut State, on, tx| {
        s.portraits.set_visible(HERO_PORTRAIT_ROW, on);
        tx.live_only();
        tx.wake_after(std::time::Duration::ZERO);
    });
    let heading = if s.is_captain_home(t) && t.renamed {
        t.title.clone()
    } else {
        format!("What should {} look after?", profile.name)
    };
    let narrow = lane_width(pane_width, true) < 560.0;
    let working = s
        .threads
        .iter()
        .filter(|thread| {
            thread.parent == Some(t.id) && !thread.archived() && thread.status == Status::Running
        })
        .count();
    let (key, line_status, text) = if working > 0 {
        (
            "captain-working-count",
            Status::Running,
            Some(format!(
                "{working} {} working",
                if working == 1 { "thread" } else { "threads" }
            )),
        )
    } else if matches!(status, Status::Ready | Status::Completed) {
        ("captain-status", status, None)
    } else {
        ("captain-status", status, Some(status.label().to_string()))
    };
    let mut status_line = row().key("captain-status-line").h(20.0).center_y();
    if let Some(text) = text {
        status_line = status_line.child(
            row()
                .key(key)
                .label(text.clone())
                .center_y()
                .gap(6.0)
                .opacity_motion(MOTION_SLOW)
                .child(status_mark(line_status, p, 6.0))
                .child(label(
                    text,
                    META.features(TABULAR),
                    status_ink(line_status, p),
                )),
        );
    }
    let role = &profile.role;
    col()
        .key("captain-hero")
        .w_full()
        .items_center()
        .gap(10.0)
        .child(
            el().size(pane_width.min(360.0), size + 6.0)
                .center()
                .child(
                    captain_wash(s.dark, portrait_index, pane_width.min(360.0), size + 6.0)
                        .absolute(0.0, 0.0),
                )
                .child(haloed(s, p, portrait, size, status)),
        )
        .child(
            label(
                heading,
                if narrow { HEADING_NARROW } else { HEADING },
                p.ink,
            )
            .key("captain-goal-heading")
            .align(rune::Align::Center)
            .wrap()
            .w_full()
            .max_w(measure(true)),
        )
        .child(
            // Who and what they look after, as two labels with a gap between them.
            row()
                .key("captain-role")
                .label(format!("Captain {}, {role}", profile.name))
                .max_w(520.0)
                .min_w(0.0)
                .gap(8.0)
                .center_y()
                .child(
                    label(format!("Captain {}", profile.name), SMALL.lh(18.0), p.ink)
                        .shrink()
                        .min_w(0.0),
                )
                .child(
                    label(role, SMALL.lh(18.0), p.muted)
                        .truncate()
                        .shrink()
                        .min_w(0.0),
                ),
        )
        .child(status_line)
}

/// The new-thread screen, laid out like KSG Code's draft hero: the composer sits at the
/// vertical centre of the pane and the headline rests 16 px above it. Two equal zero-height
/// boxes do the centring, so attachments or a long draft grow the composer without moving
/// the headline off it.
pub(super) fn empty_thread(
    s: &State,
    model: &ComposerModelDisplay,
    p: Palette,
    wide: bool,
    pane_width: f32,
    hero_fits: bool,
    caret_edges: Option<[bool; 2]>,
) -> El {
    // A captain's hero keeps its height and takes the room from the
    // space below the composer. In a window too short for both, the composer stays on screen
    // and the clip keeps the hero out of the header.
    let floor = if s.current().captain.is_some() && hero_fits {
        CAPTAIN_HERO_HEIGHT
    } else {
        0.0
    };
    let above = col()
        .w_full()
        .h(0.0)
        .grow(1.0)
        .min_h(floor)
        .clip_rounded(0.0)
        .justify_end()
        .items_center()
        .child(conversation(s, p, wide, pane_width));
    let below = col().w_full().h(0.0).grow(1.0).min_h(0.0);
    col()
        .key("empty-thread")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .items_center()
        .child(above)
        .child(composer(s, model, p, wide, pane_width, caret_edges))
        .child(below)
}

/// "What should we build in <project>?" with the project as an inline picker, as in KSG Code.
/// When the sentence does not fit on one line, the project moves under it at body size.
pub(super) fn draft_headline(s: &State, p: Palette, pane_width: f32) -> El {
    // The same words as the thread's tab, so the headline and the tab never disagree.
    let name = s
        .project(s.current().project)
        .map(|project| project.name.clone())
        .unwrap_or_else(|| "No workspace".into());
    // Sans at 30 px averages about 15 px a character, and the line stays inside the lane.
    let available = lane_width(pane_width, true);
    // With no workspace there is nothing to build "in", so the sentence stays whole and the
    // picker sits under it.
    let has_workspace = s.project(s.current().project).is_some();
    let inline = has_workspace
        && pane_width > 0.0
        && ("What should we build in ?".len() + name.chars().count()) as f32 * 15.0 <= available;
    let picker = |style: TextStyle| {
        let mut picker = row()
            .key("draft-project")
            .role("button")
            .cursor(rune::Cursor::Pointer)
            .label(format!("Project: {name}. Choose a project"))
            .help(name.clone())
            .center_y()
            .pl(4.0)
            .pr(1.0)
            .radius(12.0)
            .shrink()
            .min_w(0.0)
            .hover(p.selected)
            .press(p.selected);
        // With no workspace the picker is a choice still to make: a folder icon, then the
        // words at weight 400 in the muted colour, unlike a real project's name.
        if !has_workspace {
            picker = picker
                .gap(6.0)
                .px(6.0)
                .child(icon("folder", p.muted).key("draft-project-icon"));
        }
        picker = picker.child(
            label(
                name.clone(),
                TextStyle {
                    weight: if has_workspace { 500.0 } else { 400.0 },
                    ..style
                },
                if has_workspace { p.ink } else { p.muted },
            )
            .key("draft-project-name")
            .truncate()
            .shrink()
            .min_w(0.0),
        );
        if !s.demo {
            picker = picker
                .focusable(true)
                .on_press(|s: &mut State| s.open_picker(ProjectPicker::NewThread));
        }
        picker
    };
    let has_any_project = s.projects.entries.iter().any(|project| !project.hidden);
    if !has_workspace && !has_any_project && !s.demo {
        // Ported from KSG Code's `NoProjectsHero`: when no workspaces exist yet, show the
        // empty-workspace headline and primary "Add workspace" action instead of an empty
        // project picker.
        return col()
            .key("empty-heading")
            .w_full()
            .items_center()
            .gap(12.0)
            .child(
                label("What should we build?", HEADING_NARROW, p.ink)
                    .align(rune::Align::Center)
                    .wrap()
                    .w_full(),
            )
            .child(
                label(
                    "Add a workspace to start a thread, or start without one.",
                    SMALL,
                    p.muted,
                )
                .align(rune::Align::Center)
                .wrap()
                .w_full(),
            )
            .child(framed(
                row()
                    .key("no-projects-add-workspace")
                    .role("button")
                    .cursor(rune::Cursor::Pointer)
                    .label("Add workspace")
                    .focusable(true)
                    .h(30.0)
                    .px(10.0)
                    .gap(6.0)
                    .center_y()
                    .radius(6.0)
                    .fill(p.coral)
                    .hover(p.coral)
                    .on_press(|s: &mut State| s.open_picker(ProjectPicker::AddProject))
                    .child(icon("plus", p.on_coral))
                    .child(label(
                        "Add workspace",
                        TextStyle::new(13.0, 600.0),
                        p.on_coral,
                    )),
                p,
            ));
    }
    if inline {
        row()
            .key("empty-heading")
            .w_full()
            .justify_center()
            .center_y()
            .child(label("What should we build in", HEADING, p.ink))
            .child(picker(HEADING))
            .child(label("?", HEADING, p.ink))
    } else {
        col()
            .key("empty-heading")
            .w_full()
            .items_center()
            .gap(6.0)
            .child(
                label("What should we build?", HEADING_NARROW, p.ink)
                    .align(rune::Align::Center)
                    .wrap()
                    .w_full(),
            )
            .child(picker(TextStyle::new(16.0, 500.0).lh(24.0)).max_w(measure(true)))
    }
}
