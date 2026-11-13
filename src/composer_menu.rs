//! Shared caret menu, ported from KSG Code's chat/ComposerCommandMenu.tsx and
//! useComposerMenuItems.ts. Keep the original slash drawer's row and text styles.
use super::*;
use crate::{composer::Trigger, composer_tokens::Token};

#[derive(Clone)]
enum Action {
    Slash(String),
    Pill(Token, String),
    Context(crate::composer_context::ComposerContextRecord, String),
}

#[derive(Clone, Copy)]
enum ItemIcon {
    Provider(Vendor),
    Machine { local: bool },
}

struct Item {
    icon: Option<ItemIcon>,
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

/// Async `@` mention search state. Ported from KSG Code's `useComposerMenuItems`:
/// the query is debounced 120 ms, answered by the thread's owning machine, and a
/// response for an older request is discarded. Previous results stay visible
/// while a search is pending.
#[derive(Default)]
pub struct MentionSearch {
    /// `(thread id, owning machine, query)` the debounce and results belong to.
    pub key: Option<(usize, String, String)>,
    pub debounce_until: Option<std::time::Instant>,
    pub request_seq: u64,
    /// The key the last fired request ran for; results apply only while it is current.
    pub pending_key: Option<(usize, String, String)>,
    pub paths: Vec<crate::workspace_search::WorkspaceEntry>,
    pub paths_cwd: String,
    pub paths_pending: bool,
    local_rx: Option<
        std::sync::mpsc::Receiver<(
            u64,
            Result<crate::workspace_search::WorkspaceSearchResult, String>,
        )>,
    >,
}

/// KSG's composer path search debounce, 120 ms.
const MENTION_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(120);
/// KSG's `PROJECT_PATH_SEARCH_LIMIT`-style composer cap.
const MENTION_PATH_LIMIT: usize = 80;

fn apply_path_result(
    search: &mut MentionSearch,
    seq: u64,
    result: Result<crate::workspace_search::WorkspaceSearchResult, String>,
) {
    // A response for an older query or another thread is discarded.
    if seq != search.request_seq || search.pending_key != search.key {
        return;
    }
    search.paths_pending = false;
    search.paths = result.map(|result| result.entries).unwrap_or_default();
}

/// Applies a `mention:*` RPC response. Returns whether the tag was one of ours.
pub(crate) fn apply_mention_result(
    s: &mut State,
    server: &str,
    tag: &str,
    result: Result<serde_json::Value, String>,
) -> bool {
    if let Some(seq) = tag
        .strip_prefix("mention:path:")
        .and_then(|rest| rest.parse::<u64>().ok())
    {
        // Results only apply for the machine they were requested from.
        if s.mention_search
            .pending_key
            .as_ref()
            .is_some_and(|(_, host, _)| host == server)
        {
            apply_path_result(
                &mut s.mention_search,
                seq,
                result.and_then(|v| {
                    serde_json::from_value::<crate::workspace_search::WorkspaceSearchResult>(v)
                        .map_err(|e| e.to_string())
                }),
            );
        }
        return true;
    }
    false
}

fn mention_cwd(
    s: &State,
    machine: &str,
    project: Option<crate::projects::ProjectId>,
) -> Option<String> {
    if machine == "local" {
        s.project(project)
            .map(|project| project.path.to_string_lossy().into_owned())
    } else {
        s.machines.views.get(machine)?["index"]["projects"]
            .as_array()?
            .iter()
            .find(|p| p["id"].as_u64() == project.map(|id| id as u64))?["path"]
            .as_str()
            .map(str::to_owned)
    }
}

/// Debounced trigger for the `@` mention searches. Called from `model::poll`.
pub(crate) fn poll_mention_search(s: &mut State, tx: &mut Tx) -> bool {
    let mut changed = false;
    if let Some(rx) = &s.mention_search.local_rx
        && let Ok((seq, result)) = rx.try_recv()
    {
        s.mention_search.local_rx = None;
        apply_path_result(&mut s.mention_search, seq, result);
        changed = true;
    }
    let Some(active) = trigger(s) else {
        if s.mention_search.key.is_some() {
            s.mention_search = MentionSearch::default();
            changed = true;
        }
        return changed;
    };
    if active.marker != '@' {
        return changed;
    }
    let (thread_id, machine, project) = {
        let t = s.current();
        (t.id, t.machine.clone(), t.project)
    };
    let key = (thread_id, machine, active.query.trim().to_lowercase());
    if s.mention_search.key.as_ref() != Some(&key) {
        s.mention_search.key = Some(key);
        s.mention_search.debounce_until = Some(std::time::Instant::now() + MENTION_DEBOUNCE);
        changed = true;
    }
    let Some(until) = s.mention_search.debounce_until else {
        return changed;
    };
    let now = std::time::Instant::now();
    if now < until {
        tx.wake_after(until - now);
        return changed;
    }
    s.mention_search.debounce_until = None;
    s.mention_search.request_seq += 1;
    let seq = s.mention_search.request_seq;
    let Some(key) = s.mention_search.key.clone() else {
        return changed;
    };
    s.mention_search.pending_key = Some(key.clone());
    let (_, machine, query) = &key;
    s.mention_search.paths_pending = false;
    if let Some(cwd) = mention_cwd(s, machine, project) {
        s.mention_search.paths_cwd = cwd.clone();
        if machine == "local" {
            // The app host is the local machine's server, so the same workspace
            // search runs in-process on a blocking thread.
            let roots: Vec<std::path::PathBuf> = s
                .projects
                .entries
                .iter()
                .map(|project| project.path.clone())
                .collect();
            let (txc, rxc) = std::sync::mpsc::channel();
            let query = query.clone();
            let wake = s.wake.clone();
            std::thread::spawn(move || {
                let result = crate::workspace_search::resolve_cwd(&cwd, &roots).map(|cwd| {
                    crate::workspace_search::search(&cwd, &query, MENTION_PATH_LIMIT, None, false)
                });
                let _ = txc.send((seq, result));
                wake();
            });
            s.mention_search.local_rx = Some(rxc);
            s.mention_search.paths_pending = true;
        } else if let Some(client) = s.client.as_ref().and_then(|c| c.linked.get(machine))
            && client.has_capability(crate::protocol::WORKSPACE_SEARCH)
        {
            client.call_rpc(
                format!("mention:path:{seq}"),
                "workspace.searchEntries",
                serde_json::json!({"cwd": cwd, "query": query, "limit": MENTION_PATH_LIMIT}),
            );
            s.mention_search.paths_pending = true;
        }
    }
    changed
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
                    icon: None,
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
        let mut items: Vec<_> = s
            .slash_matches()
            .into_iter()
            .map(|command| Item {
                icon: None,
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
                    icon: None,
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
        let env_id = s.machine_identity(&thread.machine);
        let record =
            crate::composer_context::thread_context_record(&env_id, thread.id, &thread.title);
        items.push(Item {
            icon: None,
            label: thread.title.clone(),
            description: format!("Thread {}", thread.id),
            source: "Thread".into(),
            action: Action::Context(record, thread.title.clone()),
        });
    }
    for (machine, remote_threads) in &s.machines.threads {
        if machine == &s.machines.selected {
            continue;
        }
        let env_id = s.machine_identity(machine);
        let machine_name = s.machine_label(machine);
        for thread in remote_threads
            .iter()
            .filter(|other| other.title.to_lowercase().contains(&query))
            .take(40)
        {
            let record =
                crate::composer_context::thread_context_record(&env_id, thread.id, &thread.title);
            items.push(Item {
                icon: None,
                label: thread.title.clone(),
                description: format!("Thread {}", thread.id),
                source: format!("Thread · {machine_name}"),
                action: Action::Context(record, thread.title.clone()),
            });
        }
    }
    let trimmed_query = trigger.query.trim().to_lowercase();
    if !trimmed_query.is_empty() {
        let current_identity = s.machine_identity(&t.machine);
        let local_identity = s.machine_identity("local");
        let mut seen_machines = std::collections::HashSet::new();
        let mut machine_candidates = vec!["local".to_owned()];
        for peer in &s.peers {
            machine_candidates.push(peer.server_id.clone());
        }
        if let Some(client) = &s.client {
            for server in client.linked.keys() {
                machine_candidates.push(server.clone());
            }
        }
        for machine in machine_candidates {
            let env_id = s.machine_identity(&machine);
            if machine == t.machine || env_id == current_identity {
                continue;
            }
            if !seen_machines.insert(env_id.clone()) {
                continue;
            }
            let machine_name = s.machine_label(&machine);
            if !machine_name.to_lowercase().contains(&trimmed_query) {
                continue;
            }
            let is_local = machine == "local" || env_id == local_identity;
            let record =
                crate::composer_context::environment_context_record(&env_id, &machine_name);
            items.push(Item {
                icon: Some(ItemIcon::Machine { local: is_local }),
                label: machine_name.clone(),
                description: if is_local {
                    "This machine".into()
                } else {
                    String::new()
                },
                source: "Machine".into(),
                action: Action::Context(record, machine_name),
            });
        }
    }
    let entries = s.provider_instance_entries();
    let models_by_instance = s.picker_models_by_instance();
    let mut model_candidates = Vec::new();
    for entry in &entries {
        if !crate::model_picker::is_provider_instance_picker_ready(entry) {
            continue;
        }
        for option in models_by_instance
            .get(&entry.instance_id)
            .into_iter()
            .flatten()
        {
            if option.is_legacy || option.is_unavailable {
                continue;
            }
            model_candidates.push((entry, option));
        }
    }
    for (entry, option) in crate::model_picker::rank_composer_model_mentions(
        &model_candidates,
        &trigger.query,
        |(entry, option)| crate::model_picker::ModelPickerSearchableModel {
            driver_kind: &entry.driver_kind,
            provider_display_name: &entry.display_name,
            name: &option.name,
            short_name: option.short_name.as_deref(),
            sub_provider: option.sub_provider.as_deref(),
            is_favorite: false,
        },
    ) {
        let display_label = crate::model_picker::get_display_model_name(
            &option.name,
            option.short_name.as_deref(),
            option.sub_provider.as_deref(),
            false,
        );
        let provider_label = option
            .sub_provider
            .clone()
            .unwrap_or_else(|| entry.display_name.clone());
        let vendor = model_vendor_for(&option.slug, &entry.driver_kind, &entry.display_name);
        let record = crate::composer_context::model_context_record(
            &entry.instance_id,
            &option.slug,
            &display_label,
            Some(&provider_label),
        );
        items.push(Item {
            icon: Some(ItemIcon::Provider(vendor)),
            label: display_label.clone(),
            description: provider_label,
            source: "Model".into(),
            action: Action::Context(record, display_label),
        });
    }
    // File rows come from `workspace.searchEntries` on the thread's machine,
    // ranked as KSG ranks them. A linked machine without the capability keeps
    // the old notice and no file rows.
    let machine = t.machine.clone();
    let path_capable = machine == "local"
        || s.client
            .as_ref()
            .and_then(|c| c.linked.get(&machine))
            .is_some_and(|c| c.has_capability(crate::protocol::WORKSPACE_SEARCH));
    if !path_capable {
        notes.push("File search needs the machine's own Shina.".into());
    } else {
        let search = &s.mention_search;
        let active = search
            .key
            .as_ref()
            .is_some_and(|(thread, host, _)| *thread == t.id && *host == machine);
        if active {
            if search.paths_pending && search.paths.is_empty() {
                notes.push("Searching workspace files...".into());
            }
            let cwd = std::path::Path::new(&search.paths_cwd);
            for entry in &search.paths {
                let label = entry
                    .path
                    .rsplit('/')
                    .next()
                    .unwrap_or(&entry.path)
                    .to_owned();
                let directory = entry.kind == "directory";
                items.push(Item {
                    icon: None,
                    label: label.clone(),
                    description: entry.path.clone(),
                    source: if directory { "Folder" } else { "File" }.into(),
                    action: Action::Pill(
                        Token::File {
                            path: cwd.join(&entry.path),
                            directory,
                            source: None,
                        },
                        label,
                    ),
                });
            }
        }
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
        Action::Context(record, label) => {
            let Some(trigger) = trigger(s) else {
                return;
            };
            let pill_id = crate::composer_context::format_composer_context_href(
                record.kind(),
                &record.context_id,
            );
            let thread = s.current_mut();
            if let Some(existing) = thread
                .draft_context
                .iter_mut()
                .find(|r| r.context_id == record.context_id)
            {
                *existing = record;
            } else {
                thread.draft_context.push(record);
            }
            thread.draft.undo_group(|edit| {
                edit.replace_with_pill(trigger.range, pill_id, &label);
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
        let mut row_el = row()
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
            });
        if let Some(item_icon) = item.icon {
            row_el = row_el.child(match item_icon {
                ItemIcon::Provider(vendor) => provider_icon(s, p, vendor, 16.0),
                ItemIcon::Machine { local } => {
                    icon(if local { "monitor" } else { "cloud" }, p.muted).size(16.0, 16.0)
                }
            });
        }
        list = list.child(
            row_el
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
            if !event.mods.shift && !event.mods.ctrl && !event.mods.cmd && !event.mods.alt =>
        {
            select(s, items[at].action.clone());
        }
        _ => return false,
    }
    true
}

fn model_vendor_for(model: &str, provider: &str, display_name: &str) -> Vendor {
    match model_vendor(model, display_name) {
        Vendor::Other => model_vendor(provider, display_name),
        vendor => vendor,
    }
}

pub(super) fn pill_icon(id: &str) -> Option<&'static Icon> {
    if let Some((kind, _)) = crate::composer_context::parse_composer_context_href(id) {
        return Some(context_kind_icon(&kind, false));
    }
    Some(match Token::decode(id)? {
        Token::File {
            directory: true, ..
        } => pixel_icons::glyph("folder"),
        Token::File { .. } => pixel_icons::glyph("file"),
        Token::Thread { .. } => pixel_icons::glyph("thread"),
        Token::Skill { .. } => pixel_icons::glyph("bolt"),
        Token::Model {
            provider_instance,
            model,
            label,
        } => provider_glyph(model_vendor_for(&model, &provider_instance, &label)),
        Token::Machine { machine, .. } => pixel_icons::glyph(if machine == "local" {
            "monitor"
        } else {
            "cloud"
        }),
    })
}

fn context_kind_icon(kind: &str, directory: bool) -> &'static Icon {
    match kind {
        "image" => pixel_icons::glyph("image"),
        "file" | "mention" if directory => pixel_icons::glyph("folder"),
        "file" | "mention" => pixel_icons::glyph("file"),
        "thread" => pixel_icons::glyph("thread"),
        "skill" => pixel_icons::glyph("bolt"),
        "terminal" => pixel_icons::glyph("terminal"),
        "environment" => pixel_icons::glyph("monitor"),
        "model" => pixel_icons::glyph("chip"),
        "preview-annotation" | "element" => pixel_icons::glyph("eye"),
        "review-comment" => pixel_icons::glyph("issue"),
        "response" => pixel_icons::glyph("thread"),
        _ => pixel_icons::glyph("unresolved"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ChipStatus {
    Resolved,
    Unresolved,
    Invalid,
}

pub(super) fn chip_status(s: &State, id: &str) -> ChipStatus {
    let t = s.current();
    if let Some((kind, context_id)) = crate::composer_context::parse_composer_context_href(id) {
        if kind == "image" || kind == "file" {
            let bound_id = t
                .draft_context
                .iter()
                .find(|r| r.context_id == context_id)
                .and_then(|r| match &r.data {
                    crate::composer_context::ComposerContextData::Image {
                        attachment_id, ..
                    }
                    | crate::composer_context::ComposerContextData::File {
                        attachment_id, ..
                    } => Some(attachment_id.as_str()),
                    _ => None,
                });
            let attachment = t.draft_images.iter().find(|a| {
                bound_id.is_some_and(|id| a.id == id)
                    || crate::composer_context::to_kind_scoped_composer_context_id(&kind, &a.id)
                        == context_id
            });
            let Some(attachment) = attachment else {
                return ChipStatus::Unresolved;
            };
            if !attachment.is_image() && attachment.path.as_ref().is_some_and(|path| !path.exists())
            {
                return ChipStatus::Unresolved;
            }
            return ChipStatus::Resolved;
        }
        let Some(record) = t.draft_context.iter().find(|r| r.context_id == context_id) else {
            return ChipStatus::Unresolved;
        };
        if !record.is_known() {
            return ChipStatus::Unresolved;
        }
        let target_server_id = s.machine_identity(&s.machines.selected);
        if let crate::composer_context::ComposerContextData::Thread { environment_id, .. } =
            &record.data
            && crate::composer_context::outside_thread_reason(
                environment_id,
                &s.machines.selected,
                Some(&target_server_id),
                &s.peers,
            )
            .is_some()
        {
            return ChipStatus::Invalid;
        }
        return ChipStatus::Resolved;
    }
    if let Some(token) = Token::decode(id) {
        let target_server_id = s.machine_identity(&s.machines.selected);
        if let Token::Thread {
            machine: Some(owner),
            ..
        } = token
            && crate::composer_context::outside_thread_reason(
                &owner,
                &s.machines.selected,
                Some(&target_server_id),
                &s.peers,
            )
            .is_some()
        {
            return ChipStatus::Invalid;
        }
        return ChipStatus::Resolved;
    }
    if id.starts_with("shina:") {
        return ChipStatus::Invalid;
    }
    ChipStatus::Unresolved
}

pub(super) fn pill_appearance(
    status: ChipStatus,
    id: &str,
    records: &[crate::composer_context::ComposerContextRecord],
    images: &[crate::attachments::Attachment],
    machine: &str,
    p: Palette,
) -> (Color, Color, &'static Icon) {
    match status {
        ChipStatus::Unresolved => (
            p.line.alpha(0.45),
            p.muted,
            pixel_icons::glyph("unresolved"),
        ),
        ChipStatus::Invalid => (
            p.danger.alpha(0.16),
            p.danger,
            if id.contains("thread") {
                pixel_icons::glyph("thread")
            } else {
                pixel_icons::glyph("unresolved")
            },
        ),
        ChipStatus::Resolved => {
            let icon = if let Some((kind, context_id)) =
                crate::composer_context::parse_composer_context_href(id)
            {
                let record = records.iter().rev().find(|r| r.context_id == context_id);
                if let Some(rec) = record {
                    match &rec.data {
                        crate::composer_context::ComposerContextData::Model {
                            instance_id,
                            model,
                            provider_label,
                        } => provider_glyph(model_vendor_for(
                            model,
                            instance_id,
                            provider_label.as_deref().unwrap_or(""),
                        )),
                        crate::composer_context::ComposerContextData::Environment {
                            environment_id,
                        } => pixel_icons::glyph(
                            if environment_id == "local" || rec.label == "Local" {
                                "monitor"
                            } else {
                                "cloud"
                            },
                        ),
                        _ => {
                            let directory = matches!(
                                &rec.data,
                                crate::composer_context::ComposerContextData::Mention { path }
                                    if path.ends_with('/')
                            ) || matches!(
                                &rec.data,
                                crate::composer_context::ComposerContextData::File { mime_type, .. }
                                    if mime_type == "inode/directory"
                            ) || images.iter().any(|a| {
                                crate::composer_context::to_kind_scoped_composer_context_id(
                                    &kind, &a.id,
                                ) == context_id
                                    && a.mime == "inode/directory"
                            });
                            context_kind_icon(&kind, directory)
                        }
                    }
                } else {
                    let directory = images.iter().any(|a| {
                        crate::composer_context::to_kind_scoped_composer_context_id(&kind, &a.id)
                            == context_id
                            && a.mime == "inode/directory"
                    });
                    context_kind_icon(&kind, directory)
                }
            } else if let Some(token) = Token::decode(id) {
                match token {
                    Token::File {
                        directory: true, ..
                    } => pixel_icons::glyph("folder"),
                    Token::File { .. } => pixel_icons::glyph("file"),
                    Token::Thread {
                        machine: Some(ref owner),
                        ..
                    } if owner != machine => pixel_icons::glyph("thread"),
                    Token::Thread { .. } => pixel_icons::glyph("thread"),
                    Token::Skill { .. } => pixel_icons::glyph("bolt"),
                    Token::Model {
                        provider_instance,
                        model,
                        label,
                    } => provider_glyph(model_vendor_for(&model, &provider_instance, &label)),
                    Token::Machine { machine, .. } => pixel_icons::glyph(if machine == "local" {
                        "monitor"
                    } else {
                        "cloud"
                    }),
                }
            } else {
                pixel_icons::glyph("unresolved")
            };
            (p.selected, p.ink, icon)
        }
    }
}

pub(super) fn pill_style(s: &State, p: Palette) -> impl Fn(&str) -> rune::PillStyle + 'static {
    let t = s.current();
    let records = t.draft_context.clone();
    let images = t.draft_images.clone();
    let machine = t.machine.clone();
    let mut statuses = std::collections::HashMap::new();
    for (_, id) in t.draft.pills() {
        statuses.insert(id.to_owned(), chip_status(s, id));
    }
    move |id: &str| {
        let status = statuses.get(id).copied().unwrap_or(ChipStatus::Resolved);
        let (bg, fg, icon) = pill_appearance(status, id, &records, &images, &machine, p);
        rune::PillStyle::default().bg(bg).fg(fg).icon(icon)
    }
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
    fn a_stale_workspace_response_for_an_older_query_is_discarded() {
        let mut state = State::new(".".into(), false);
        state.current_mut().draft.replace_text("@ab");
        state.current_mut().draft.focused = true;
        let (id, machine) = {
            let t = state.current();
            (t.id, t.machine.clone())
        };
        // Query "abc" is now current; its request superseded "ab".
        state.mention_search.key = Some((id, machine.clone(), "abc".into()));
        state.mention_search.pending_key = Some((id, machine.clone(), "abc".into()));
        state.mention_search.request_seq = 2;
        let stale = serde_json::json!({
            "entries": [{"path": "ab.rs", "kind": "file"}],
            "truncated": false
        });
        assert!(apply_mention_result(
            &mut state,
            &machine,
            "mention:path:1",
            Ok(stale)
        ));
        assert!(state.mention_search.paths.is_empty());
        // The in-flight response applies.
        let current = serde_json::json!({
            "entries": [{"path": "abc.rs", "kind": "file"}],
            "truncated": false
        });
        assert!(apply_mention_result(
            &mut state,
            &machine,
            "mention:path:2",
            Ok(current)
        ));
        assert_eq!(state.mention_search.paths[0].path, "abc.rs");
        // A result sent by another machine is ignored.
        state.mention_search.request_seq = 3;
        assert!(apply_mention_result(
            &mut state,
            "other-server",
            "mention:path:3",
            Ok(serde_json::json!({"entries": [{"path": "x.rs", "kind": "file"}]}))
        ));
        assert_eq!(state.mention_search.paths[0].path, "abc.rs");
    }

    #[test]
    fn workspace_search_entries_render_as_file_rows() {
        let mut state = State::new(".".into(), false);
        state.current_mut().draft.replace_text("@model");
        state.current_mut().draft.focused = true;
        let (id, machine) = {
            let t = state.current();
            (t.id, t.machine.clone())
        };
        state.mention_search.key = Some((id, machine, "model".into()));
        state.mention_search.paths_cwd = "/work/proj".into();
        state.mention_search.paths = vec![
            crate::workspace_search::WorkspaceEntry {
                path: "src/model.rs".into(),
                kind: "file".into(),
                ignored: false,
            },
            crate::workspace_search::WorkspaceEntry {
                path: "src/model".into(),
                kind: "directory".into(),
                ignored: false,
            },
        ];
        let trigger = trigger(&state).unwrap();
        let (items, _) = items(&state, &trigger);
        let rows: Vec<_> = items
            .iter()
            .filter(|i| matches!(&i.action, Action::Pill(Token::File { .. }, _)))
            .collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].label, "model.rs");
        assert_eq!(rows[0].source, "File");
        match &rows[0].action {
            Action::Pill(
                Token::File {
                    path, directory, ..
                },
                _,
            ) => {
                assert_eq!(*path, std::path::PathBuf::from("/work/proj/src/model.rs"));
                assert!(!*directory);
            }
            _ => unreachable!(),
        }
        match &rows[1].action {
            Action::Pill(Token::File { directory, .. }, _) => assert!(*directory),
            _ => unreachable!(),
        }
    }

    #[test]
    fn workspace_search_capability_is_a_minor_gate() {
        assert_eq!(crate::protocol::WORKSPACE_SEARCH_MINOR, 24);
        assert!(crate::protocol::WORKSPACE_SEARCH_MINOR <= crate::protocol::MINOR);
        assert!(crate::protocol::WORKSPACE_SEARCH_MINOR > 23);
        assert_eq!(
            crate::protocol::method_grant("workspace.searchEntries"),
            Some("read")
        );
    }

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
            &items[0].action,
            Action::Context(rec, _) if rec.context_id == "thread_123"
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
