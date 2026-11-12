//! Shared caret menu, ported from KSG Code's chat/ComposerCommandMenu.tsx and
//! useComposerMenuItems.ts. Keep the original slash drawer's row and text styles.
use super::*;
use crate::{composer::Trigger, composer_tokens::Token};

#[derive(Clone)]
enum Action {
    Slash(String),
    Pill(Token, String),
}

struct Item {
    label: String,
    description: String,
    source: String,
    action: Action,
}

pub(super) fn trigger(s: &State) -> Option<Trigger> {
    let t = s.current();
    if !t.draft.focused {
        return None;
    }
    let candidate = crate::composer::trigger(&t.draft)?;
    if t.native
        .slash_dismissed
        .as_ref()
        .is_some_and(|d| d.matches(&candidate))
    {
        return None;
    }
    Some(candidate)
}

pub(super) fn clear_dismissed_if_changed(s: &mut State) {
    let candidate = crate::composer::trigger(&s.current().draft);
    let dismissed = s.current().native.slash_dismissed.clone();
    if let Some(d) = dismissed
        && candidate.as_ref().is_none_or(|c| !d.matches(c))
    {
        s.current_mut().native.slash_dismissed = None;
    }
}

fn linked(s: &State) -> bool {
    s.current().machine != "local" || s.client.is_some() && s.machines.selected != "local"
}

fn items(s: &State, trigger: &Trigger) -> (Vec<Item>, Vec<String>) {
    let t = s.current();
    if trigger.marker == '$' {
        let normalized_query = crate::composer::normalize_search_query(&trigger.query, false);
        let mut ranked: Vec<(usize, String, Item)> = t
            .native
            .skills
            .iter()
            .filter_map(|skill| {
                let score = if normalized_query.is_empty() {
                    0
                } else {
                    crate::composer::score_slash_command(
                        &skill.name,
                        &skill.description,
                        &normalized_query,
                        true,
                    )?
                };
                let item = Item {
                    label: format!("${}", skill.name),
                    description: skill.description.clone(),
                    source: t.harness.clone(),
                    action: Action::Pill(
                        Token::Skill {
                            name: skill.name.clone(),
                            harness: t.harness.clone(),
                        },
                        skill.name.clone(),
                    ),
                };
                Some((score, skill.name.to_lowercase(), item))
            })
            .collect();
        if !normalized_query.is_empty() {
            ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        }
        return (
            ranked.into_iter().map(|(_, _, item)| item).collect(),
            vec![],
        );
    }
    if trigger.marker == '/' {
        let mut items: Vec<_> = t
            .slash_matches()
            .into_iter()
            .map(|command| Item {
                label: format!("/{}", command.name),
                description: command.description,
                source: command.source,
                action: Action::Slash(command.name),
            })
            .collect();
        for skill in &t.native.skills {
            if t.native
                .commands
                .iter()
                .any(|c| c.name == format!("skill:{}", skill.name))
            {
                continue;
            }
            if format!("skill:{}", skill.name)
                .to_lowercase()
                .contains(&trigger.query.to_lowercase())
                || skill
                    .description
                    .to_lowercase()
                    .contains(&trigger.query.to_lowercase())
            {
                items.push(Item {
                    label: format!("/skill:{}", skill.name),
                    description: skill.description.clone(),
                    source: t.harness.clone(),
                    action: Action::Pill(
                        Token::Skill {
                            name: skill.name.clone(),
                            harness: t.harness.clone(),
                        },
                        skill.name.clone(),
                    ),
                });
            }
        }
        let notes = if t.native.commands_loaded {
            vec![]
        } else {
            vec![
                t.native
                    .commands_error
                    .clone()
                    .unwrap_or_else(|| "Loading workspace commands...".into()),
            ]
        };
        return (items, notes);
    }
    let query = trigger.query.to_lowercase();
    let mut items = Vec::new();
    let mut notes = Vec::new();
    if linked(s) {
        notes.push("File search needs the machine's own Shina.".into());
    } else if let Some(project) = s.project(t.project) {
        let inventory = s.composer_sources.paths(&project.path, s.wake.clone());
        if inventory.loading {
            notes.push("Searching workspace files...".into());
        }
        for entry in inventory
            .entries
            .iter()
            .filter(|entry| entry.relative.to_lowercase().contains(&query))
            .take(40)
        {
            let label = entry
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            items.push(Item {
                label: label.clone(),
                description: entry.relative.clone(),
                source: if entry.directory { "Folder" } else { "File" }.into(),
                action: Action::Pill(
                    Token::File {
                        path: entry.path.clone(),
                        directory: entry.directory,
                        source: None,
                    },
                    label,
                ),
            });
        }
    }
    for thread in s
        .threads
        .iter()
        .filter(|other| {
            other.id != t.id
                && other.machine == t.machine
                && other.title.to_lowercase().contains(&query)
        })
        .take(40)
    {
        items.push(Item {
            label: thread.title.clone(),
            description: format!("Thread {}", thread.id),
            source: "Thread".into(),
            action: Action::Pill(
                Token::Thread {
                    id: thread.id,
                    title: thread.title.clone(),
                    machine: Some(thread.machine.clone()),
                    extra: Default::default(),
                },
                thread.title.clone(),
            ),
        });
    }
    (items, notes)
}

fn select(s: &mut State, action: Action) {
    match action {
        Action::Slash(name) => s.choose_slash(name),
        Action::Pill(token, label) => {
            let Some(trigger) = trigger(s) else {
                return;
            };
            let edit = &mut s.current_mut().draft;
            edit.undo_group(|edit| {
                edit.replace_with_pill(trigger.range, token.id(), &label);
                edit.insert(" ");
            });
            s.restore_composer_focus(composer_ui::FocusReason::Inserted);
            s.checkpoint();
        }
    }
}

/// 4 pt from the caret line, above by preference. Rune flips and fits at an 8 pt window inset.
pub(super) fn overlay(s: &State, p: Palette, cx: &Cx<State>, width: f32) -> El {
    let Some(trigger) = trigger(s) else {
        return el();
    };
    let (items, notes) = items(s, &trigger);
    let caret = cx.caret("composer-input");
    let zoom = s.zoom.clamp(0.5, 2.0);
    let point = if let Some(rect) = caret {
        el().key("composer-menu-point")
            .absolute(rect[0] / zoom, rect[1] / zoom)
            .size(rect[2] / zoom, rect[3] / zoom)
    } else {
        // On the first frame the input has not drawn a caret yet. Anchor to the field.
        el()
    };
    let menu = menu(s, p, &trigger, items, notes)
        .w(440.0_f32.min((width / zoom - 16.0).max(100.0)))
        .min_h(0.0)
        .shrink()
        .anchored_fit(
            if caret.is_some() {
                "composer-menu-point"
            } else {
                "composer-input"
            },
            rune::Side::Top,
            rune::Align::Start,
            4.0,
        );
    el().key("composer-menu")
        .absolute(0.0, 0.0)
        .size(0.0, 0.0)
        .child(point)
        .child(menu)
}

fn menu(s: &State, p: Palette, trigger: &Trigger, items: Vec<Item>, notes: Vec<String>) -> El {
    let prefix = menu_prefix(trigger.marker);
    let mut list = col()
        .key(format!("{prefix}-menu"))
        .role("listbox")
        .label(match trigger.marker {
            '/' => "Commands",
            '$' => "Skills",
            _ => "Files, folders and threads",
        })
        .max_h(288.0)
        .padding(4.0)
        .radius(6.0)
        .fill(p.card)
        .border(2.0, p.frame)
        .box_shadow(hard_shadow(p))
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    for note in notes {
        list = list.child(label(note, SMALL, p.muted).px(16.0).py(10.0));
    }
    if items.is_empty() {
        list = list.child(
            label(
                match trigger.marker {
                    '/' => "No matching command.",
                    '$' if !s.current().native.commands_loaded => s
                        .current()
                        .native
                        .commands_error
                        .as_deref()
                        .unwrap_or("Loading skills..."),
                    '$' if s.current().native.skills.is_empty() => {
                        "No skills available for this harness."
                    }
                    '$' => "No matching skill.",
                    _ => "No matching files, folders or threads.",
                },
                SMALL,
                p.muted,
            )
            .key(format!("{prefix}-empty"))
            .px(16.0)
            .py(10.0),
        );
    }
    let at = s
        .current()
        .native
        .slash_index
        .min(items.len().saturating_sub(1));
    for (i, item) in items.into_iter().enumerate() {
        list = list.child(
            row()
                .key(format!("{prefix}-item-{i}"))
                .role("option")
                .label(format!("{} {}", item.label, item.description))
                .min_h(32.0)
                .px(8.0)
                .gap(8.0)
                .radius(4.0)
                .center_y()
                .cursor(rune::Cursor::Pointer)
                .fill(if i == at {
                    p.coral_bg
                } else {
                    Color::TRANSPARENT
                })
                .hover(p.selected)
                .on_press_tx(move |s: &mut State, tx| {
                    select(s, item.action.clone());
                    s.restore_composer_focus(composer_ui::FocusReason::Inserted);
                    tx.focus("composer-input", false);
                })
                .child(
                    label(item.label, TextStyle::new(12.0, 500.0), p.ink)
                        .max_w(220.0)
                        .truncate()
                        .shrink(),
                )
                .child(
                    label(item.description, SMALL, p.muted)
                        .truncate()
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(pixel_label(&item.source, p.muted)),
        );
    }
    list
}

pub(super) fn key(s: &mut State, event: &rune::KeyEvent) -> bool {
    let Some(trigger) = trigger(s) else {
        return false;
    };
    if event.key == rune::Key::Escape {
        s.current_mut().native.slash_dismissed = Some(crate::composer::DismissedTrigger {
            marker: trigger.marker,
            range_start: trigger.range.start,
        });
        return true;
    }
    let (items, _) = items(s, &trigger);
    if items.is_empty() {
        return matches!(event.key, rune::Key::Up | rune::Key::Down);
    }
    let at = s.current().native.slash_index.min(items.len() - 1);
    match event.key {
        rune::Key::Down => s.current_mut().native.slash_index = (at + 1).min(items.len() - 1),
        rune::Key::Up => s.current_mut().native.slash_index = at.saturating_sub(1),
        rune::Key::Home => s.current_mut().native.slash_index = 0,
        rune::Key::End => s.current_mut().native.slash_index = items.len() - 1,
        rune::Key::Tab | rune::Key::Enter
            if !event.mods.shift && !event.mods.ctrl && !event.mods.cmd =>
        {
            select(s, items[at].action.clone());
        }
        _ => return false,
    }
    true
}

pub(super) fn pill_icon(id: &str) -> Option<&'static Icon> {
    Some(match Token::decode(id)? {
        Token::File {
            directory: true, ..
        } => pixel_icons::glyph("folder"),
        Token::File { .. } => pixel_icons::glyph("file"),
        Token::Thread { .. } => pixel_icons::glyph("thread"),
        Token::Skill { .. } => pixel_icons::glyph("bolt"),
    })
}

pub(super) fn scroll_selected(s: &State, tx: &mut Tx) {
    if let Some(trigger) = trigger(s) {
        let prefix = menu_prefix(trigger.marker);
        tx.scroll_into_view(format!("{prefix}-item-{}", s.current().native.slash_index));
    }
}

fn menu_prefix(marker: char) -> &'static str {
    match marker {
        '/' => "slash",
        '$' => "skill",
        _ => "mention",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn linked_threads_offer_only_thread_results_and_a_machine_notice() {
        let mut state = State::new(".".into(), false);
        state.current_mut().machine = "linked-machine".into();
        state.current_mut().draft.replace_text("@");
        state.current_mut().draft.focused = true;
        let mut other = Thread::new(123);
        other.machine = "linked-machine".into();
        other.title = "Other thread".into();
        state.threads.push(other);
        let trigger = trigger(&state).unwrap();
        let (items, notes) = items(&state, &trigger);
        assert_eq!(items.len(), 1);
        assert!(matches!(
            items[0].action,
            Action::Pill(Token::Thread { id: 123, .. }, _)
        ));
        assert_eq!(notes, ["File search needs the machine's own Shina."]);
    }

    #[test]
    fn an_empty_skill_inventory_has_one_muted_information_line() {
        let mut state = State::new(".".into(), false);
        state.current_mut().draft.replace_text("$");
        state.current_mut().draft.focused = true;
        state.current_mut().native.commands_loaded = true;
        let active = trigger(&state).unwrap();
        let (matches, notes) = items(&state, &active);
        assert!(matches.is_empty());
        assert!(notes.is_empty());
        let mut rt = rune::Runtime::with_text(
            state,
            |s, _| {
                let active = trigger(s).unwrap();
                let (matches, notes) = items(s, &active);
                menu(s, palette(false), &active, matches, notes).w(440.0)
            },
            (620.0, 720.0),
            rune::Env::new(false, 1.0),
            rune::text::TextSystem::new(FONTS, (SANS_FAMILY, "Cascadia Mono, Consolas")),
        );
        rt.frame(0.0);
        assert!(rt.find("skill-empty").is_some());
        assert!(rt.find("skill-item-0").is_none());
    }
}
