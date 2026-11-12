//! KSG Code settings/SettingsSidebarNav, settingsSearch and settingsLayout, in Pixel Voyage.
use super::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const SECTIONS: [&str; 8] = [
    "General",
    "Threads",
    "Harnesses",
    "Machines",
    "Memory",
    "Updates",
    "About",
    "Keybindings",
];

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    /// KSG Code's panelAnimationDurationMs, 0 to 400 ms. Zero disables panel motion.
    pub panel_animation_duration_ms: u32,
    pub confirm_archive: bool,
    pub confirm_delete: bool,
    pub send_shortcut: crate::composer::SendShortcut,
    pub keybindings: Vec<crate::keybindings::KeybindingRule>,
    pub dismissed_updates: Vec<String>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            panel_animation_duration_ms: 0,
            confirm_archive: false,
            confirm_delete: true,
            send_shortcut: Default::default(),
            keybindings: Vec::new(),
            dismissed_updates: vec![],
        }
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelDefault {
    pub provider: String,
    pub model: String,
    pub effort: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CalendarDefault {
    pub day: String,
    pub time: String,
}
impl Default for CalendarDefault {
    fn default() -> Self {
        Self {
            day: "tomorrow".into(),
            time: "09:00".into(),
        }
    }
}
impl CalendarDefault {
    fn preset(&self) -> Option<crate::lifecycle::Preset> {
        use crate::lifecycle::{Day, Preset};
        let day = match self.day.as_str() {
            "today" => Day::Today,
            "tomorrow" => Day::Tomorrow,
            name => Day::Weekday(
                [
                    "monday",
                    "tuesday",
                    "wednesday",
                    "thursday",
                    "friday",
                    "saturday",
                    "sunday",
                ]
                .iter()
                .position(|d| *d == name)? as u32,
            ),
        };
        let (hour, minute) = self.time.split_once(':')?;
        if hour.len() != 2 || minute.len() != 2 {
            return None;
        }
        let hour: u32 = hour.parse().ok()?;
        let minute: u32 = minute.parse().ok()?;
        (hour < 24 && minute < 60).then_some(Preset::At { day, hour, minute })
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ThreadDefaults {
    pub access: String,
    pub snooze_minutes: Vec<u32>,
    pub snooze_calendar: Vec<CalendarDefault>,
    pub models: BTreeMap<String, ModelDefault>,
    /// Starting directory when the Add workspace folder browser opens (KSG Code's `addProjectBaseDirectory`).
    /// Empty defaults to `"~/"`.
    pub add_project_base_directory: String,
}
impl Default for ThreadDefaults {
    fn default() -> Self {
        Self {
            access: "full-access".into(),
            snooze_minutes: vec![60, 180],
            snooze_calendar: vec![
                CalendarDefault {
                    day: "today".into(),
                    time: "18:00".into(),
                },
                CalendarDefault::default(),
                CalendarDefault {
                    day: "monday".into(),
                    time: "09:00".into(),
                },
            ],
            models: BTreeMap::new(),
            add_project_base_directory: String::new(),
        }
    }
}
impl ThreadDefaults {
    pub fn validate(&self) -> Result<(), String> {
        if crate::model::AccessMode::parse(&self.access).is_none()
            || self.snooze_minutes.len() + self.snooze_calendar.len() == 0
            || self.snooze_minutes.len() + self.snooze_calendar.len() > 8
            || self.snooze_minutes.iter().any(|m| *m == 0 || *m > 43200)
            || self.snooze_calendar.iter().any(|p| p.preset().is_none())
            || self.models.len() > 32
            || self.models.iter().any(|(id, m)| {
                !crate::harness::valid_id(id)
                    || [&m.provider, &m.model, &m.effort]
                        .iter()
                        .any(|s| s.len() > 256 || s.chars().any(char::is_control))
            })
            || self.add_project_base_directory.len() > 1024
            || self
                .add_project_base_directory
                .chars()
                .any(char::is_control)
        {
            return Err("Invalid thread defaults.".into());
        }
        Ok(())
    }
    pub fn snooze_presets(&self) -> Vec<crate::lifecycle::Preset> {
        self.snooze_minutes
            .iter()
            .copied()
            .map(crate::lifecycle::Preset::After)
            .chain(
                self.snooze_calendar
                    .iter()
                    .filter_map(CalendarDefault::preset),
            )
            .collect()
    }
    fn snooze_text(&self) -> String {
        self.snooze_minutes
            .iter()
            .map(u32::to_string)
            .chain(
                self.snooze_calendar
                    .iter()
                    .map(|p| format!("{}@{}", p.day, p.time)),
            )
            .collect::<Vec<_>>()
            .join(", ")
    }
    fn set_snooze_text(&mut self, text: &str) -> Result<(), String> {
        let mut minutes = vec![];
        let mut calendar = vec![];
        for entry in text.split(',').map(str::trim).filter(|v| !v.is_empty()) {
            if let Some((day, time)) = entry.split_once('@') {
                calendar.push(CalendarDefault {
                    day: day.to_lowercase(),
                    time: time.into(),
                });
            } else {
                minutes.push(
                    entry
                        .parse::<u32>()
                        .map_err(|_| "Use minutes or day@HH:MM, separated by commas.")?,
                );
            }
        }
        self.snooze_minutes = minutes;
        self.snooze_calendar = calendar;
        self.validate()
    }
}
pub struct Dialog {
    pub section: usize,
    pub search: rune::TextEdit,
    pub shortcut_search: rune::TextEdit,
    pub shortcut_capture: Option<String>,
    pub shortcut_notice: Option<String>,
    pub machine: String,
    pub profile_target: Option<String>,
    pub snooze: rune::TextEdit,
    pub add_project_base: rune::TextEdit,
    pub model: rune::TextEdit,
    pub provider: rune::TextEdit,
    pub effort: rune::TextEdit,
    pub model_harness: String,
    pub notice: Option<String>,
    pub archive: Option<usize>,
    pub archive_targets: Vec<usize>,
    pub licences: bool,
    pub versions: BTreeMap<String, String>,
    version_probe: Option<std::sync::mpsc::Receiver<BTreeMap<String, String>>>,
}
impl Default for Dialog {
    fn default() -> Self {
        Self {
            section: 0,
            search: Default::default(),
            shortcut_search: Default::default(),
            shortcut_capture: None,
            shortcut_notice: None,
            machine: "local".into(),
            profile_target: None,
            snooze: Default::default(),
            add_project_base: Default::default(),
            model: Default::default(),
            provider: Default::default(),
            effort: Default::default(),
            model_harness: "pi".into(),
            notice: None,
            archive: None,
            archive_targets: vec![],
            licences: false,
            versions: BTreeMap::new(),
            version_probe: None,
        }
    }
}

pub(crate) fn handle_shortcut_capture(
    s: &mut State,
    event: &rune::KeyEvent,
    tx: &rune::Tx,
) -> bool {
    let Some(capture_id) = s.settings.shortcut_capture.clone() else {
        return false;
    };
    if matches!(event.key, rune::Key::Modifier { .. }) {
        return true;
    }
    if event.key == rune::Key::Escape {
        s.settings.shortcut_capture = None;
        s.settings.shortcut_notice = None;
        return true;
    }
    let platform = crate::keybindings::Platform::current();
    let Some(next_key) = crate::keybindings::keybinding_from_key_event(
        event,
        platform,
        tx.physical_key(),
        tx.key_base(),
    ) else {
        return true;
    };
    let rows = crate::keybindings::build_keybinding_rows(&s.resolved_keybindings(), platform);
    let Some(row) = rows.into_iter().find(|r| r.id == capture_id) else {
        s.settings.shortcut_capture = None;
        return true;
    };
    let when_opt = (!row.when.is_empty()).then_some(row.when.as_str());
    match crate::keybindings::upsert_custom_rule(
        &mut s.preferences.keybindings,
        &row.command,
        &next_key,
        when_opt,
        Some((&row.command, &row.key, when_opt)),
    ) {
        Ok(()) => {
            s.settings.shortcut_capture = None;
            s.settings.shortcut_notice = None;
            s.checkpoint();
        }
        Err(error) => {
            s.settings.shortcut_notice = Some(error);
        }
    }
    true
}

impl State {
    pub fn resolved_keybindings(&self) -> Vec<crate::keybindings::ResolvedKeybindingRule> {
        crate::keybindings::merge_with_default_keybindings(&self.preferences.keybindings)
    }

    pub fn shortcut_context(&self, composer_focus: bool) -> crate::keybindings::ShortcutContext {
        let t = self.current();
        let editable_focus = composer_focus
            || self.search.focused
            || self.model_search.focused
            || self.picker_query.focused
            || self.captain_picker_query.focused
            || self.rename_edit.focused
            || self.snooze_edit.focused
            || t.reply.focused
            || self.settings.search.focused
            || self.settings.snooze.focused
            || self.settings.add_project_base.focused
            || self.settings.model.focused
            || self.settings.provider.focused
            || self.settings.effort.focused
            || self.settings.shortcut_capture.is_some();
        let mut ctx = crate::keybindings::ShortcutContext::desktop();
        ctx.editable_focus = editable_focus;
        ctx.composer_focus = composer_focus;
        ctx.draft_thread_route = t.is_draft();
        ctx.turn_running = t.status.busy();
        ctx.composer_requires_modifier =
            crate::composer::requires_modifier(self.preferences.send_shortcut, &t.draft.text);
        ctx.model_picker_open = self.settings_picker == Some(SettingsPicker::Model);
        ctx
    }

    pub fn open_settings(&mut self) {
        self.settings_menu = true;
        self.settings.section = 0;
        self.settings.machine = "local".into();
        self.settings.search.replace_text("");
        self.settings.shortcut_search.replace_text("");
        self.settings.shortcut_capture = None;
        self.settings.shortcut_notice = None;
        self.settings.notice = None;
        if !self.demo && !cfg!(test) && self.settings.version_probe.is_none() {
            let (send, receive) = std::sync::mpsc::channel();
            self.settings.version_probe = Some(receive);
            let wake = self.wake.clone();
            std::thread::spawn(move || {
                let mut versions = BTreeMap::new();
                for (name, process) in [
                    ("Pi", crate::backend::discovery_command()),
                    ("Codex", crate::codex::process()),
                    ("Claude", crate::claude::process()),
                ] {
                    let Ok(process) = process else { continue };
                    let mut command = version_command(&process);
                    if let Ok(output) = crate::process::output(
                        &mut command,
                        std::time::Duration::from_secs(2),
                        4096,
                    ) && output.status.success()
                    {
                        let line = String::from_utf8_lossy(&output.stdout)
                            .lines()
                            .next()
                            .unwrap_or("")
                            .chars()
                            .filter(|c| !c.is_control())
                            .take(100)
                            .collect::<String>();
                        if !line.is_empty() {
                            versions.insert(name.into(), line);
                        }
                    }
                }
                let _ = send.send(versions);
                wake();
            });
        }
        let defaults = self.settings_defaults();
        self.settings.snooze.replace_text(&defaults.snooze_text());
        self.settings
            .add_project_base
            .replace_text(&defaults.add_project_base_directory);
        let value = defaults
            .models
            .get(&self.settings.model_harness)
            .cloned()
            .unwrap_or_default();
        self.settings.provider.replace_text(&value.provider);
        self.settings.model.replace_text(&value.model);
        self.settings.effort.replace_text(&value.effort);
    }
    pub fn poll_settings(&mut self) -> bool {
        if let Some(receiver) = &self.settings.version_probe
            && let Ok(versions) = receiver.try_recv()
        {
            self.settings.versions = versions;
            self.settings.version_probe = None;
            return true;
        }
        false
    }
    fn settings_index(&self) -> Value {
        if self.client.is_none() {
            return self.snapshot();
        }
        self.machines
            .views
            .get(&self.settings.machine)
            .map(|v| v["index"].clone())
            .unwrap_or(Value::Null)
    }
    fn settings_defaults(&self) -> ThreadDefaults {
        serde_json::from_value(self.settings_index()["threadDefaults"].clone()).unwrap_or_default()
    }
    pub(crate) fn settings_profile(&self) -> crate::model::UserProfile {
        if self.client.is_none() {
            return self.user_profile();
        }
        self.machines
            .views
            .get(&self.settings.machine)
            .and_then(|view| serde_json::from_value(view["profile"].clone()).ok())
            .unwrap_or_default()
    }
    pub(crate) fn save_settings_profile(
        &mut self,
        profile: crate::model::UserProfile,
    ) -> Result<(), String> {
        if let Some(client) = &self.client {
            let server = self
                .settings
                .profile_target
                .as_deref()
                .unwrap_or(&self.settings.machine);
            return client
                .commands_for(server)
                .ok_or("Machine is unavailable.")?
                .send("profile.put", json!({"profile":profile}))
                .map(|_| ());
        }
        self.set_user_profile(profile)
    }
    pub(crate) fn settings_keep_awake(&self) -> crate::power::KeepAwake {
        if self.client.is_none() {
            return self.background_settings().keep_awake;
        }
        self.machines
            .views
            .get(&self.settings.machine)
            .and_then(|view| serde_json::from_value(view["keepAwake"].clone()).ok())
            .unwrap_or_default()
    }
    pub(crate) fn set_settings_keep_awake(&mut self, policy: crate::power::KeepAwake) {
        if self.client.is_some() {
            self.settings_command("background.keepAwake", json!({"policy":policy}));
        } else {
            self.set_keep_awake(policy);
        }
    }
    fn settings_harness_ids(&self) -> Vec<String> {
        if self.client.is_none() {
            return self.harness_ids();
        }
        self.machines
            .views
            .get(&self.settings.machine)
            .and_then(|view| serde_json::from_value(view["harnessIds"].clone()).ok())
            .unwrap_or_else(|| vec!["pi".into(), "codex".into(), "claude".into()])
    }
    fn set_settings_machine(&mut self, machine: &str) {
        self.settings.machine = machine.into();
        let defaults = self.settings_defaults();
        self.settings.snooze.replace_text(&defaults.snooze_text());
        self.settings
            .add_project_base
            .replace_text(&defaults.add_project_base_directory);
        let value = defaults
            .models
            .get(&self.settings.model_harness)
            .cloned()
            .unwrap_or_default();
        self.settings.provider.replace_text(&value.provider);
        self.settings.model.replace_text(&value.model);
        self.settings.effort.replace_text(&value.effort);
    }
    fn settings_command(&mut self, method: &str, args: Value) {
        let result = if let Some(client) = &self.client {
            client
                .commands_for(&self.settings.machine)
                .ok_or("Machine is unavailable.".into())
                .and_then(|commands| commands.send(method, args).map(|_| ()))
        } else {
            self.execute_command(&crate::protocol::Command::new(method, args))
                .map(|_| ())
        };
        self.settings.notice = result.err();
    }
    fn save_defaults(&mut self, defaults: ThreadDefaults) {
        self.settings_command("settings.threadDefaults", json!(defaults));
    }
    fn reveal_settings_file(&mut self, name: &str) {
        let path = self.data_dir.join(name);
        let target = if path.exists() {
            path
        } else {
            self.data_dir.clone()
        };
        let mut command = std::process::Command::new(if cfg!(target_os = "macos") {
            "open"
        } else if cfg!(windows) {
            "explorer"
        } else {
            "xdg-open"
        });
        if cfg!(target_os = "macos") {
            command.arg("-R");
        }
        if cfg!(windows) {
            command.arg(format!("/select,{}", target.display()));
        } else if cfg!(target_os = "linux") && target.is_file() {
            command.arg(target.parent().unwrap_or(&self.data_dir));
        } else {
            command.arg(target);
        }
        if !self.demo {
            if let Err(e) = command.spawn() {
                self.settings.notice = Some(e.to_string());
            }
        }
    }
}

fn matches(s: &State, title: &str) -> bool {
    let query = s.settings.search.text.trim().to_lowercase();
    query.is_empty() || title.to_lowercase().contains(&query)
}
fn version_command(process: &std::process::Command) -> std::process::Command {
    let mut command = std::process::Command::new(process.get_program());
    if let Some(script) = process.get_args().find(|arg| {
        arg.to_string_lossy().ends_with(".js") || arg.to_string_lossy().ends_with(".mjs")
    }) {
        command.arg(script);
    }
    for (name, value) in process.get_envs() {
        if let Some(value) = value {
            command.env(name, value);
        } else {
            command.env_remove(name);
        }
    }
    command
        .arg("--version")
        .env_remove(crate::mcp::URL_ENV)
        .env_remove(crate::mcp::TOKEN_ENV)
        .env_remove("SHINA_RUNTIME_MODE")
        .env_remove("SHINA_PI_BRIDGE_PATH");
    command
}
fn package_version(script: &std::path::Path) -> Option<String> {
    let script = script.canonicalize().ok()?;
    for parent in script.ancestors().skip(1).take(5) {
        let Some(bytes) = std::fs::read(parent.join("package.json")).ok() else {
            continue;
        };
        let value: Value = serde_json::from_slice(&bytes).ok()?;
        if let Some(version) = value["version"].as_str().filter(|v| {
            v.len() <= 64
                && v.chars()
                    .all(|c| c.is_ascii_alphanumeric() || ".-_+".contains(c))
        }) {
            return Some(version.into());
        }
    }
    None
}

/// KSG settingsLayout SettingsRow: 12 px vertical padding and a trailing control.
fn setting(key: &str, title: &str, description: &str, control: El, p: Palette) -> El {
    col()
        .key(key)
        .w_full()
        .child(
            row()
                .py(12.0)
                .gap(16.0)
                .items_start()
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(4.0)
                        .child(label(title, TextStyle::new(14.0, 500.0), p.ink).wrap())
                        .child(
                            label(description, TextStyle::new(13.0, 400.0).lh(18.0), p.muted)
                                .wrap(),
                        ),
                )
                .child(control),
        )
        .child(el().h(1.0).w_full().fill(p.line))
}
fn info(key: &str, title: &str, description: &str, value: &str, p: Palette) -> El {
    setting(key, title, description, pixel_text(value, p.muted), p)
}
fn add(mut pane: El, s: &State, key: &str, title: &str, note: &str, control: El, p: Palette) -> El {
    if matches(s, title) {
        pane = pane.child(setting(key, title, note, control, p));
    }
    pane
}
fn field(edit: &rune::TextEdit, key: &str, title: &str, p: Palette) -> El {
    text_input(edit, TextStyle::new(14.0, 400.0))
        .key(key)
        .label(title)
        .placeholder(title, p.faint)
        .single_line()
        .w(136.0)
        .h(32.0)
        .padding(8.0)
        .radius(4.0)
        .color(p.ink)
        .field_focus(edit, p)
}

pub(super) fn settings_button(
    key: &str,
    title: &str,
    p: Palette,
    action: impl Fn(&mut State) + 'static,
) -> El {
    framed(
        super::button(key, title, p, action)
            .h(32.0)
            .fill(p.card)
            .press(p.card.mix(Color::BLACK, 0.12)),
        p,
    )
}
pub(super) fn destructive_button(
    key: &str,
    title: &str,
    p: Palette,
    action: impl Fn(&mut State) + 'static,
) -> El {
    settings_button(key, title, Palette { ink: p.danger, ..p }, action)
}

fn default_access(p: Palette, current: &str) -> El {
    let mut group = col()
        .key("settings-access-value")
        .role("radiogroup")
        .label("Default access mode")
        .focus_group()
        .w(160.0)
        .padding(2.0)
        .gap(2.0)
        .radius(4.0)
        .border(1.0, p.line);
    for mode in [
        crate::model::AccessMode::ReadOnly,
        crate::model::AccessMode::Supervised,
        crate::model::AccessMode::Edits,
        crate::model::AccessMode::Full,
    ] {
        let on = mode.id() == current;
        group = group.child(
            row()
                .key(format!("settings-default-access-{}", mode.id()))
                .role("radio")
                .checked(on)
                .label(mode.label())
                .focusable(true)
                .focus_highlight(p.selected)
                .h(28.0)
                .px(8.0)
                .center_y()
                .radius(4.0)
                .fill(if on { p.selected } else { Color::TRANSPARENT })
                .border(1.0, if on { p.frame } else { Color::TRANSPARENT })
                .on_press(move |s: &mut State| {
                    let mut defaults = s.settings_defaults();
                    defaults.access = mode.id().into();
                    s.save_defaults(defaults);
                })
                .child(label(mode.label(), TextStyle::new(12.0, 500.0), p.ink)),
        );
    }
    group
}

fn send_shortcut_picker(s: &State, p: Palette) -> El {
    let current = s.preferences.send_shortcut;
    let mut group = row().gap(4.0).flex_wrap();
    for option in crate::composer::SendShortcut::ALL {
        let on = option == current;
        let label_text = option.label();
        let key_suffix = match option {
            crate::composer::SendShortcut::Enter => "enter",
            crate::composer::SendShortcut::ModEnterMultiline => "mod-enter-multiline",
            crate::composer::SendShortcut::ModEnter => "mod-enter",
        };
        group = group.child(
            row()
                .key(format!("settings-send-shortcut-{key_suffix}"))
                .role("radio")
                .checked(on)
                .label(&label_text)
                .focusable(true)
                .focus_highlight(p.selected)
                .h(28.0)
                .px(8.0)
                .center_y()
                .radius(4.0)
                .fill(if on { p.selected } else { Color::TRANSPARENT })
                .border(1.0, if on { p.frame } else { Color::TRANSPARENT })
                .on_press(move |s: &mut State| {
                    s.preferences.send_shortcut = option;
                    s.checkpoint();
                })
                .child(label(&label_text, TextStyle::new(12.0, 500.0), p.ink)),
        );
    }
    group
}

/// Ported from KSG SettingsSidebarNav and SettingsPanels. The modal size follows Shina's brief.
pub(super) fn dialog(s: &State, p: Palette) -> El {
    let compact = s.viewport_width <= 620.0;
    let mut nav = col()
        .key("settings-sections")
        .role("list")
        .focus_group()
        .w(if compact { 112.0 } else { 168.0 })
        .min_w(0.0)
        .gap(4.0)
        .padding(8.0);
    for (index, name) in SECTIONS.iter().enumerate() {
        let selected = s.settings.section == index;
        nav = nav.child(
            row()
                .key(format!("settings-section-{index}"))
                .role("button")
                .label(*name)
                .h(36.0)
                .w_full()
                .center_y()
                .radius(4.0)
                .focusable(true)
                .focus_highlight(p.selected)
                .fill(if selected {
                    p.selected
                } else {
                    Color::TRANSPARENT
                })
                .child(el().w(2.0).h(24.0).fill(if selected {
                    p.coral
                } else {
                    Color::TRANSPARENT
                }))
                .child(label(*name, TextStyle::new(14.0, 400.0), p.ink).px(8.0))
                .on_press(move |s: &mut State| {
                    s.settings.section = index;
                    s.settings.search.replace_text("");
                    s.settings.shortcut_capture = None;
                    s.settings.shortcut_notice = None;
                    if index == 3 && s.client.is_some() && !s.demo {
                        s.machine_action("devices", "");
                    }
                })
                .on_key(move |s: &mut State, ev, _| {
                    let count = SECTIONS.len();
                    match ev.key {
                        rune::Key::Up => s.settings.section = (index + count - 1) % count,
                        rune::Key::Down => s.settings.section = (index + 1) % count,
                        rune::Key::Home => s.settings.section = 0,
                        rune::Key::End => s.settings.section = count - 1,
                        _ => return false,
                    }
                    s.settings.search.replace_text("");
                    s.settings.shortcut_capture = None;
                    s.settings.shortcut_notice = None;
                    false
                }),
        );
    }
    let search = text_input(&s.settings.search, TextStyle::new(14.0, 400.0))
        .key("settings-search")
        .label("Search settings")
        .placeholder("Search settings", p.muted)
        .autofocus()
        .single_line()
        .w(0.0)
        .grow(1.0)
        .min_w(0.0)
        .h(32.0)
        .padding(8.0)
        .radius(4.0)
        .color(p.ink)
        .field_focus(&s.settings.search, p)
        .on_edit(|s: &mut State| &mut s.settings.search);
    let mut content = col()
        .key("settings-content")
        .grow(1.0)
        .min_w(0.0)
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .px(if compact { 12.0 } else { 24.0 })
        .py(16.0);
    if !s.settings.search.text.trim().is_empty() {
        let mut count = 0;
        for section in 0..SECTIONS.len() {
            let (rows, found) = section_content(s, p, section);
            if found {
                content = content
                    .child(pixel_label(SECTIONS[section], p.muted).pt(12.0))
                    .child(rows);
                count += 1;
            }
        }
        if count == 0 {
            content = content
                .child(empty_list(s, p, "settings-no-results", "No settings found").py(24.0));
        }
    } else {
        content = content.child(label(SECTIONS[s.settings.section], DIALOG_TITLE, p.ink).pb(8.0));
        content = content.child(section_content(s, p, s.settings.section).0);
    }
    if let Some(notice) = &s.settings.notice {
        content = content.child(label(notice, SMALL, p.danger).wrap());
    }
    let card = col()
        .key("settings-menu")
        .role("dialog")
        .label("Settings")
        .w(if compact {
            s.viewport_width
        } else {
            (s.viewport_width - 48.0).min(880.0)
        })
        .h(if compact {
            s.viewport_height
        } else {
            (s.viewport_height - 48.0).min(600.0)
        })
        .radius(6.0)
        .fill(p.card)
        .border(2.0, p.frame)
        .box_shadow(hard_shadow(p))
        .child(
            row()
                .h(56.0)
                .px(16.0)
                .gap(12.0)
                .center_y()
                .child(icon_button(
                    "settings-close",
                    if compact { "Back" } else { "Close settings" },
                    if compact { "back" } else { "close" },
                    p,
                    |s| {
                        s.settings_menu = false;
                        s.settings.shortcut_capture = None;
                    },
                ))
                .child(search),
        )
        .child(el().h(1.0).w_full().fill(p.line))
        .child(
            row()
                .grow(1.0)
                .min_h(0.0)
                .child(nav)
                .child(el().w(1.0).h_full().fill(p.line))
                .child(content),
        );
    el().key("settings-backdrop")
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .fill(Color::BLACK.alpha(0.28))
        .center()
        .modal(false, |s: &mut State| {
            s.settings_menu = false;
            s.settings.shortcut_capture = None;
        })
        .child(card)
}

/// Ported from KSG Code's AppearanceSettingsPanel panel-animation-duration range,
/// 0..400 ms in 25 ms steps. Uses Shina's existing pixel track and framed thumb.
fn panel_animation_control(s: &State, p: Palette) -> El {
    let value = s.preferences.panel_animation_duration_ms.min(400);
    let fraction = value as f32 / 400.0;
    let slider = el()
        .key("panel-animation-duration")
        .role("slider")
        .label(format!("Panel animation duration, {value} milliseconds"))
        .w(132.0)
        .h(24.0)
        .center_y()
        .child(el().w_full().h(6.0).radius(2.0).fill(p.sea_bg))
        .child(
            el().w(8.0 + fraction * 116.0)
                .h(6.0)
                .radius(2.0)
                .fill(p.coral),
        )
        .child(
            el().absolute(fraction * 116.0, 4.0)
                .size(16.0, 16.0)
                .radius(4.0)
                .fill(p.card)
                .border(2.0, p.frame),
        )
        .on_step(|s: &mut State, step, _| {
            s.preferences.panel_animation_duration_ms =
                (s.preferences.panel_animation_duration_ms as i32 + step * 25).clamp(0, 400) as u32;
            s.checkpoint();
        })
        .on_step_limit(|s: &mut State, maximum, _| {
            s.preferences.panel_animation_duration_ms = if maximum { 400 } else { 0 };
            s.checkpoint();
        })
        .on_drag(|s: &mut State, drag, _| {
            if !drag.cancelled() {
                let fraction = ((drag.x - 8.0) / (drag.w - 16.0).max(1.0)).clamp(0.0, 1.0);
                s.preferences.panel_animation_duration_ms = (fraction * 16.0).round() as u32 * 25;
                s.checkpoint();
            }
        });
    row()
        .gap(12.0)
        .center_y()
        .child(outlined(
            pixel_text(format!("{value} ms"), p.ink)
                .w(64.0)
                .px(8.0)
                .py(4.0),
            p,
        ))
        .child(slider)
}

fn section_content(s: &State, p: Palette, section: usize) -> (El, bool) {
    let mut pane = col().w_full().min_w(0.0);
    let titles: &[&str] = match section {
        0 => &[
            "Your profile",
            "Keep working in the background",
            "Keep awake",
            "Theme",
            "Send shortcut",
            "Panel animations",
            "Confirm before archiving a thread",
            "Confirm before deleting a thread",
        ],
        1 => &[
            "Auto-settle interval",
            "Snooze presets",
            "Default access mode",
            "Default harness",
            "Default model and effort",
        ],
        2 => &["Pi", "Codex", "Claude", "ACP harnesses"],
        3 => &["Linked machines", "Update all machines"],
        4 => &["Honcho"],
        5 => &[
            "Current version",
            "Last check",
            "Failed release",
            "Check now",
        ],
        6 => &["Version", "Data folder", "Licences"],
        _ => &[],
    };
    let mut found = titles.iter().any(|t| matches(s, t));
    match section {
        0 => {
            if s.client.as_ref().is_some_and(|c| !c.linked.is_empty()) {
                pane = pane.child(pixel_text(
                    format!(
                        "Server settings on {}",
                        s.machine_label(&s.settings.machine)
                    ),
                    p.muted,
                ));
            }
            pane = add(
                pane,
                s,
                "settings-profile-row",
                "Your profile",
                "Name, time zone and preferences shared with captains.",
                captain_ui::profile_item(s, p).w(160.0),
                p,
            );
            for (index, item) in captain_ui::background_settings(s, p)
                .into_iter()
                .enumerate()
            {
                if matches(s, titles[(index + 1).min(2)]) {
                    pane = pane.child(item);
                }
            }
            pane = add(
                pane,
                s,
                "settings-theme",
                "Theme",
                "Choose the day or night palette.",
                settings_button(
                    "theme-toggle",
                    if s.dark { "Dark" } else { "Light" },
                    p,
                    |s| {
                        s.dark = !s.dark;
                        s.checkpoint();
                    },
                ),
                p,
            );
            pane = add(
                pane,
                s,
                "settings-send-shortcut",
                "Send shortcut",
                "Choose when Enter sends a prompt or inserts a new line",
                send_shortcut_picker(s, p),
                p,
            );
            pane = add(
                pane,
                s,
                "settings-confirm-archive",
                "Confirm before archiving a thread",
                "Ask before moving a thread to the archive.",
                captain_ui::switch(
                    s,
                    "confirm-archive-toggle",
                    "Confirm before archiving a thread",
                    s.preferences.confirm_archive,
                    p,
                    |s| {
                        s.preferences.confirm_archive = !s.preferences.confirm_archive;
                        s.checkpoint();
                    },
                ),
                p,
            );
            pane = add(
                pane,
                s,
                "settings-panel-animations",
                "Panel animations",
                "Set how fast panels open and close.",
                panel_animation_control(s, p),
                p,
            );
            pane = add(
                pane,
                s,
                "settings-confirm-delete",
                "Confirm before deleting a thread",
                "Ask before deleting a thread and its delegated children.",
                captain_ui::switch(
                    s,
                    "confirm-delete-toggle",
                    "Confirm before deleting a thread",
                    s.preferences.confirm_delete,
                    p,
                    |s| {
                        s.preferences.confirm_delete = !s.preferences.confirm_delete;
                        s.checkpoint();
                    },
                ),
                p,
            );
        }
        1 => {
            if s.client.as_ref().is_some_and(|c| !c.linked.is_empty()) {
                let mut machines = row().gap(4.0).flex_wrap().child(pixel_text(
                    format!("Editing {}", s.machine_label(&s.settings.machine)),
                    p.muted,
                ));
                for machine in machine_options(s) {
                    let id = machine.clone();
                    machines = machines.child(settings_button(
                        &format!("settings-machine-{machine}"),
                        &s.machine_label(&machine),
                        p,
                        move |s| s.set_settings_machine(&id),
                    ));
                }
                pane = pane.child(machines);
            }
            let index = s.settings_index();
            let days = index["autoSettleAfterDays"].as_u64().unwrap_or(3);
            pane = add(
                pane,
                s,
                "settings-auto-settle",
                "Auto-settle interval",
                "Settle inactive lone threads on the selected machine.",
                settings_button(
                    "settings-auto-settle-value",
                    &if days == 0 {
                        "Off".into()
                    } else {
                        format!("{days} days")
                    },
                    p,
                    move |s| {
                        let next = match days {
                            0 => 1,
                            1 => 3,
                            3 => 7,
                            7 => 14,
                            14 => 30,
                            _ => 0,
                        };
                        s.settings_command("settings.autoSettle", json!({"days":next}));
                    },
                ),
                p,
            );
            let defaults = s.settings_defaults();
            pane = add(
                pane,
                s,
                "settings-snooze",
                "Snooze presets",
                "Comma-separated minutes or day@HH:MM, such as 60, tomorrow@09:00.",
                row()
                    .gap(4.0)
                    .child(
                        field(
                            &s.settings.snooze,
                            "settings-snooze-input",
                            "Snooze presets",
                            p,
                        )
                        .on_edit(|s: &mut State| &mut s.settings.snooze),
                    )
                    .child(settings_button("settings-snooze-save", "Save", p, |s| {
                        let mut defaults = s.settings_defaults();
                        match defaults.set_snooze_text(&s.settings.snooze.text) {
                            Ok(()) => s.save_defaults(defaults),
                            Err(error) => s.settings.notice = Some(error),
                        }
                    })),
                p,
            );
            let access = defaults.access.clone();
            pane = add(
                pane,
                s,
                "settings-access",
                "Default access mode",
                "Used only when a new thread has nothing to inherit.",
                default_access(p, &access),
                p,
            );
            pane = add(
                pane,
                s,
                "settings-default-harness",
                "Default harness",
                "Used when there is no thread to inherit from.",
                settings_button(
                    "default-harness",
                    index["defaultHarness"].as_str().unwrap_or("pi"),
                    p,
                    |s| {
                        if s.client.is_none() {
                            s.cycle_default_harness();
                            return;
                        }
                        let ids = s.settings_harness_ids();
                        let current = s.settings_index()["defaultHarness"]
                            .as_str()
                            .unwrap_or("pi")
                            .to_owned();
                        let pos = ids.iter().position(|id| id == &current).unwrap_or(0);
                        if !ids.is_empty() {
                            s.settings_command(
                                "settings.defaultHarness",
                                json!({"harness":ids[(pos+1)%ids.len()]}),
                            );
                        }
                    },
                ),
                p,
            );
            if matches(s, "Default model and effort") {
                let mut harnesses = row().gap(4.0).flex_wrap();
                for id in s.settings_harness_ids() {
                    let title = id.clone();
                    harnesses = harnesses.child(settings_button(
                        &format!("default-model-{id}"),
                        &title,
                        p,
                        move |s| {
                            s.settings.model_harness = id.clone();
                            let value = s
                                .settings_defaults()
                                .models
                                .get(&id)
                                .cloned()
                                .unwrap_or_default();
                            s.settings.provider.replace_text(&value.provider);
                            s.settings.model.replace_text(&value.model);
                            s.settings.effort.replace_text(&value.effort);
                        },
                    ));
                }
                pane=pane.child(setting("settings-model","Default model and effort","Provider, model ID and effort for new threads without an inherited choice.",pixel_text(&s.settings.model_harness,p.muted),p))
                    .child(harnesses)
                    .child(row().gap(4.0).flex_wrap()
                        .child(field(&s.settings.provider,"default-provider-input","Provider",p).on_edit(|s: &mut State| &mut s.settings.provider))
                        .child(field(&s.settings.model,"default-model-input","Model",p).on_edit(|s: &mut State| &mut s.settings.model))
                        .child(field(&s.settings.effort,"default-effort-input","Effort",p).on_edit(|s: &mut State| &mut s.settings.effort))
                        .child(settings_button("default-model-save","Save",p,|s| {
                            let mut defaults=s.settings_defaults();
                            let provider=if s.settings.provider.text.trim().is_empty() {if s.settings.model_harness=="pi" {"cpa"} else {&s.settings.model_harness}} else {s.settings.provider.text.trim()};
                            defaults.models.insert(s.settings.model_harness.clone(),ModelDefault{provider:provider.into(),model:s.settings.model.text.trim().into(),effort:s.settings.effort.text.trim().into()});
                            s.save_defaults(defaults);
                        })));
            }
            pane = add(
                pane,
                s,
                "settings-add-project-base",
                "Add project starts in",
                "Leave empty to use \"~/\" when the Add Project browser opens.",
                row()
                    .gap(4.0)
                    .child(
                        field(
                            &s.settings.add_project_base,
                            "settings-add-project-base-input",
                            "~/",
                            p,
                        )
                        .on_edit(|s: &mut State| &mut s.settings.add_project_base),
                    )
                    .child(settings_button(
                        "settings-add-project-base-save",
                        "Save",
                        p,
                        |s| {
                            let mut defaults = s.settings_defaults();
                            defaults.add_project_base_directory =
                                s.settings.add_project_base.text.trim().into();
                            s.save_defaults(defaults);
                        },
                    )),
                p,
            );
        }
        2 => {
            let processes = [
                ("Pi", crate::backend::discovery_command()),
                ("Codex", crate::codex::process()),
                ("Claude", crate::claude::process()),
            ];
            for (name, process) in processes {
                if !matches(s, name) {
                    continue;
                }
                let path = process.ok().and_then(|cmd| {
                    let script = cmd
                        .get_args()
                        .find(|a| a.to_string_lossy().ends_with(".js"))
                        .map(std::path::PathBuf::from);
                    script.or_else(|| {
                        crate::harness::executable(&cmd.get_program().to_string_lossy())
                            .ok()
                            .map(|(path, _)| path)
                    })
                });
                let description = path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or("Not found".into());
                let status = s
                    .settings
                    .versions
                    .get(name)
                    .map(|version| format!("Found, {version}"))
                    .unwrap_or_else(|| {
                        if path.is_some() {
                            "Found".into()
                        } else {
                            "Not found".into()
                        }
                    });
                pane=pane.child(info(&format!("settings-harness-{name}"),name,&format!("{description}. Uses SHINA_{}_CLI, then the host's existing install and PATH.",name.to_uppercase()),&status,p));
            }
            {
                let registry = crate::harness::Registry::read();
                for agent in registry.agents {
                    if !matches(s, "ACP harnesses") && !matches(s, &agent.name) {
                        continue;
                    }
                    found = true;
                    let resolved = crate::harness::executable(&agent.command);
                    let path = resolved
                        .as_ref()
                        .ok()
                        .map(|(exe, script)| script.as_ref().unwrap_or(exe));
                    let version = resolved
                        .as_ref()
                        .ok()
                        .and_then(|(_, script)| script.as_deref())
                        .and_then(package_version);
                    let status = if let Some(version) = version {
                        format!("Found, {version}")
                    } else if resolved.is_ok() {
                        "Found".into()
                    } else {
                        "Not found".into()
                    };
                    let note = format!(
                        "{}. Configured in harnesses.json; Shina resolves its command on PATH.",
                        path.map(|path| path.display().to_string())
                            .unwrap_or_else(|| agent.command.clone())
                    );
                    pane = pane.child(info(
                        &format!("settings-acp-{}", agent.id),
                        &agent.name,
                        &note,
                        &status,
                        p,
                    ));
                }
                if matches(s, "ACP harnesses") {
                    pane = pane.child(setting(
                        "settings-acp-file",
                        "ACP harnesses",
                        "Shina never installs agents, signs in or edits their configuration.",
                        settings_button("reveal-harnesses", "Reveal file", p, |s| {
                            s.reveal_settings_file("harnesses.json")
                        }),
                        p,
                    ));
                }
            }
        }
        3 => {
            if s.client.as_ref().is_some_and(|c| !c.linked.is_empty()) {
                pane = pane.child(
                    row()
                        .gap(8.0)
                        .child(pixel_label("Server settings context", p.muted))
                        .child(pixel_text(s.machine_label(&s.settings.machine), p.ink)),
                );
                let mut context = row().gap(4.0).flex_wrap();
                for machine in machine_options(s) {
                    let id = machine.clone();
                    context = context.child(settings_button(
                        &format!("settings-context-{machine}"),
                        &s.machine_label(&machine),
                        p,
                        move |s| s.set_settings_machine(&id),
                    ));
                }
                pane = pane.child(context);
            }
            if matches(s, "Linked machines") {
                pane = pane.child(setting(
                    "settings-machines",
                    "Linked machines",
                    "Direct connections. Threads and credentials stay on their owning machine.",
                    settings_button("machine-settings", "Refresh devices", p, |s| {
                        s.machine_action("devices", "")
                    }),
                    p,
                ));
                pane = pane.child(machine_panel(s, p, true));
            }
            pane = add(
                pane,
                s,
                "settings-fleet",
                "Update all machines",
                "Review versions and skipped machines before confirming.",
                settings_button("settings-update-all", "Update all machines", p, |s| {
                    s.open_fleet()
                }),
                p,
            );
        }
        4 => {
            let status = if s.client.is_none() {
                s.memory_backend_status()
            } else {
                s.machines
                    .views
                    .get("local")
                    .and_then(|view| serde_json::from_value(view["memoryBackend"].clone()).ok())
                    .unwrap_or_default()
            };
            let (word, note) = match status {
                crate::model::MemoryBackendStatus::Disabled => {
                    ("Disabled".into(), "Honcho is disabled.".into())
                }
                crate::model::MemoryBackendStatus::Connected {
                    queued_count,
                    deriver_pending,
                } => (
                    format!("Connected, {queued_count} queued"),
                    format!(
                        "{} derivations pending.",
                        deriver_pending
                            .map(|n| n.to_string())
                            .unwrap_or("unknown".into())
                    ),
                ),
                crate::model::MemoryBackendStatus::Offline {
                    last_error,
                    queued_count,
                } => (format!("Offline, {queued_count} queued"), last_error),
            };
            pane = add(
                pane,
                s,
                "settings-honcho",
                "Honcho",
                &format!("{word}. {note}"),
                settings_button("reveal-honcho", "Reveal file", p, |s| {
                    s.reveal_settings_file("honcho.json")
                }),
                p,
            );
        }
        5 => {
            if matches(s, "Current version") {
                pane = pane.child(info(
                    "settings-version",
                    "Current version",
                    "This desktop build.",
                    env!("CARGO_PKG_VERSION"),
                    p,
                ));
            }
            if matches(s, "Last check") {
                pane = pane.child(info(
                    "settings-last-check",
                    "Last check",
                    "Release checks use the host's existing gh login.",
                    s.updater.last_check_label(),
                    p,
                ));
            }
            if let Some((tag, reason)) = s.updater.failed_release_detail() {
                pane = add(
                    pane,
                    s,
                    "settings-failed-release",
                    "Failed release",
                    &format!("{tag}: {reason}"),
                    settings_button("settings-retry-update", "Retry", p, |s| {
                        s.settings.notice = s.updater.retry_failed_release().err();
                    }),
                    p,
                );
            } else if matches(s, "Failed release") {
                pane = pane.child(info(
                    "settings-failed-release",
                    "Failed release",
                    "No release is blocked after a failed update.",
                    "None",
                    p,
                ));
            }
            pane = add(
                pane,
                s,
                "settings-check-now",
                "Check now",
                "Check the latest release without installing it.",
                settings_button("settings-update-check", "Check now", p, |s| {
                    s.updater.retry()
                })
                .disabled(matches!(
                    s.updater.status(),
                    crate::update::Status::Disabled
                )),
                p,
            );
        }
        6 => {
            if matches(s, "Version") {
                pane = pane.child(info(
                    "settings-about-version",
                    "Version",
                    "Shina, native Rust and Rune.",
                    env!("CARGO_PKG_VERSION"),
                    p,
                ));
            }
            pane = add(
                pane,
                s,
                "settings-data-folder",
                "Data folder",
                &s.data_dir.display().to_string(),
                settings_button("reveal-data", "Reveal", p, |s| s.reveal_settings_file("")),
                p,
            );
            pane = add(
                pane,
                s,
                "settings-licences",
                "Licences",
                "Shina and Pixelarticons: MIT. Departure Mono: SIL OFL. Inter: SIL OFL. Source Serif 4: SIL OFL.",
                settings_button("settings-show-licences", "View licences", p, |s| {
                    s.settings.licences = !s.settings.licences
                }),
                p,
            );
            if s.settings.licences {
                for (name, body) in [
                    ("Shina", include_str!("../LICENSE")),
                    ("Pixelarticons", include_str!("../assets/icons-LICENSE.txt")),
                    (
                        "Departure Mono",
                        include_str!("../assets/fonts/DepartureMono-LICENSE.txt"),
                    ),
                    ("Inter", include_str!("../assets/fonts/Inter-LICENSE.txt")),
                    (
                        "Source Serif 4",
                        include_str!("../assets/fonts/SourceSerif4-LICENSE.txt"),
                    ),
                ] {
                    pane = pane
                        .child(pixel_label(name, p.muted).pt(12.0))
                        .child(label(body, SMALL, p.muted).wrap());
                }
            }
        }
        _ => {
            return keybindings_section(s, p);
        }
    }
    (pane, found)
}

fn keybindings_section(s: &State, p: Palette) -> (El, bool) {
    use crate::keybindings::{
        KeybindingSource, Platform, build_keybinding_rows, command_label,
        conflict_warning_description, filter_keybinding_rows, format_shortcut_label,
        group_keybinding_rows, keycap_parts, reset_custom_rule,
    };

    let platform = Platform::current();
    let global_query = s.settings.search.text.trim();
    let query = if global_query.is_empty() {
        s.settings.shortcut_search.text.as_str()
    } else {
        global_query
    };

    let mut pane = col()
        .key("settings-keybindings")
        .w_full()
        .min_w(0.0)
        .gap(6.0);
    if global_query.is_empty() {
        pane = pane.child(
            text_input(&s.settings.shortcut_search, TextStyle::new(13.5, 400.0))
                .key("keybindings-search")
                .label("Search keybindings")
                .placeholder("Search keybindings", p.muted)
                .single_line()
                .w_full()
                .h(32.0)
                .padding(8.0)
                .radius(4.0)
                .color(p.ink)
                .fill(field_fill(s, p))
                .field_focus(&s.settings.shortcut_search, p)
                .on_edit(|s: &mut State| &mut s.settings.shortcut_search)
                .on_input_key(|s: &mut State, event, _| {
                    if event.key == rune::Key::Escape && !s.settings.shortcut_search.text.is_empty()
                    {
                        s.settings.shortcut_search.replace_text("");
                        return true;
                    }
                    s.undo_key(event, true)
                }),
        );
    }

    if let Some(notice) = &s.settings.shortcut_notice {
        pane = pane.child(label(notice, SMALL, p.danger).wrap());
    }

    let rows = build_keybinding_rows(&s.resolved_keybindings(), platform);
    let filtered = filter_keybinding_rows(rows, query);
    let found = !filtered.is_empty();
    let groups = group_keybinding_rows(filtered);

    if groups.is_empty() {
        pane = pane.child(
            empty_list(
                s,
                p,
                "keybindings-no-results",
                "No keybindings match your search.",
            )
            .py(24.0),
        );
        return (pane, found);
    }

    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for group in groups {
        pane = pane.child(
            pixel_label(group.title, p.muted)
                .key(format!("keybindings-group-{}", group.id))
                .pt(12.0)
                .pb(4.0),
        );
        for row_item in group.rows {
            let occurrence = seen.entry(row_item.command.clone()).or_insert(0);
            let slug = if *occurrence == 0 {
                row_item.command.clone()
            } else {
                format!("{}-{}", row_item.command, *occurrence + 1)
            };
            *occurrence += 1;

            let title_text = command_label(&row_item.command);
            let when_text = if row_item.when.is_empty() {
                "When Always".to_owned()
            } else {
                format!("When {}", row_item.when)
            };
            let is_custom = row_item.source == KeybindingSource::Custom;
            let is_capturing = s.settings.shortcut_capture.as_deref() == Some(row_item.id.as_str());

            let mut title_row = row()
                .gap(6.0)
                .center_y()
                .flex_wrap()
                .child(label(&title_text, TextStyle::new(14.0, 500.0), p.ink).wrap());
            if is_custom {
                title_row = title_row.child(outlined(
                    pixel_text("Custom", p.muted)
                        .key(format!("keybinding-source-{slug}"))
                        .px(4.0)
                        .py(1.0)
                        .radius(4.0),
                    p,
                ));
            }

            let mut info_col = col().grow(1.0).min_w(0.0).gap(4.0).child(title_row).child(
                label(&when_text, TextStyle::new(12.5, 400.0).lh(17.0), p.muted).wrap_anywhere(),
            );
            if let Some(conflict_msg) = conflict_warning_description(&row_item.conflicts) {
                info_col = info_col.child(
                    label(conflict_msg, TextStyle::new(12.5, 450.0).lh(17.0), p.danger)
                        .key(format!("keybinding-conflict-{slug}"))
                        .wrap(),
                );
            }

            let mut controls = row().gap(6.0).center_y().shrink().flex_wrap();
            if is_custom {
                let reset_cmd = row_item.command.clone();
                let reset_key = row_item.key.clone();
                let reset_when = row_item.when.clone();
                let default_key = row_item.default_key.clone();
                let default_when = row_item.default_when.clone();
                controls = controls.child(
                    settings_button(
                        &format!("keybinding-reset-{slug}"),
                        if default_key.is_some() {
                            "Reset"
                        } else {
                            "Remove"
                        },
                        p,
                        move |s| {
                            let when_opt = (!reset_when.is_empty()).then_some(reset_when.as_str());
                            if let Err(error) = reset_custom_rule(
                                &mut s.preferences.keybindings,
                                &reset_cmd,
                                &reset_key,
                                when_opt,
                                default_key.as_deref(),
                                &default_when,
                            ) {
                                s.settings.shortcut_notice = Some(error);
                            } else {
                                s.settings.shortcut_capture = None;
                                s.settings.shortcut_notice = None;
                                s.checkpoint();
                            }
                        },
                    )
                    .label(format!(
                        "{}: {}",
                        if row_item.default_key.is_some() {
                            "Reset to default"
                        } else {
                            "Remove"
                        },
                        title_text
                    )),
                );
            }

            if is_capturing {
                let capture_key = format!("keybinding-capture-{slug}");
                controls = controls
                    .child(
                        row()
                            .key(capture_key.clone())
                            .role("button")
                            .label(format!("Press shortcut for {title_text}"))
                            .focusable(true)
                            .h(26.0)
                            .px(8.0)
                            .center_y()
                            .radius(4.0)
                            .fill(p.selected)
                            .border(2.0, p.coral)
                            .child(pixel_text("Press shortcut", p.ink))
                            .on_key(move |s: &mut State, event, tx| {
                                handle_shortcut_capture(s, event, tx)
                            }),
                    )
                    .child(settings_button(
                        &format!("keybinding-cancel-{slug}"),
                        "Cancel",
                        p,
                        |s| {
                            s.settings.shortcut_capture = None;
                            s.settings.shortcut_notice = None;
                        },
                    ));
            } else {
                let mut chips = row().gap(4.0).center_y();
                for part in keycap_parts(&row_item.binding.shortcut, platform) {
                    chips = chips.child(keycap_chip(&part, p));
                }
                let row_id = row_item.id.clone();
                let capture_key = format!("keybinding-capture-{slug}");
                let shortcut_label = format_shortcut_label(&row_item.binding.shortcut, platform);
                controls = controls.child(
                    row()
                        .key(format!("keybinding-pill-{slug}"))
                        .role("button")
                        .cursor(rune::Cursor::Pointer)
                        .label(format!("Edit shortcut for {title_text}: {shortcut_label}"))
                        .focusable(true)
                        .focus_highlight(p.selected)
                        .hover(p.selected)
                        .padding(2.0)
                        .radius(4.0)
                        .on_press_tx(move |s: &mut State, tx| {
                            s.settings.shortcut_capture = Some(row_id.clone());
                            s.settings.shortcut_notice = None;
                            tx.focus(&capture_key, false);
                        })
                        .child(chips),
                );
            }

            pane = pane.child(
                col()
                    .key(format!("keybinding-row-{slug}"))
                    .w_full()
                    .child(
                        row()
                            .py(10.0)
                            .gap(12.0)
                            .items_start()
                            .child(info_col)
                            .child(controls),
                    )
                    .child(el().h(1.0).w_full().fill(p.line)),
            );
        }
    }

    (pane, found)
}

pub(super) fn fleet_dialog(s: &State, p: Palette) -> El {
    let mut card=col().key("fleet-update-dialog").role("dialog").label("Update all machines")
        .w((s.viewport_width-32.0).min(760.0)).h((s.viewport_height-48.0).clamp(240.0,660.0))
        .padding(16.0).gap(12.0).radius(6.0).fill(p.card).border(2.0,p.frame).box_shadow(hard_shadow(p))
        .child(row().center_y().child(label("Update all machines",DIALOG_TITLE,p.ink)).child(spacer())
            .child(icon_button("fleet-close","Close","close",p,|s| s.fleet.open=false)))
        .child(label("Each machine checks its own latest release. Offline machines are skipped. Closing this dialog does not cancel accepted updates.",SMALL,p.muted).wrap());
    let mut rows = col().h(0.0).grow(1.0).min_h(0.0).scroll(ScrollSpec {
        stick_bottom: false,
    });
    for (index, machine) in s.fleet.rows.iter().enumerate() {
        let (word, detail) = machine
            .state
            .split_once(": ")
            .map(|(word, detail)| (word, Some(detail)))
            .unwrap_or((&machine.state, None));
        // The status word and its reason share the right edge.
        let mut control = col()
            .gap(4.0)
            .items_end()
            .max_w(240.0)
            .child(pixel_text(word, if machine.failed { p.danger } else { p.muted }).wrap());
        if let Some(detail) = detail {
            control =
                control.child(label(detail, TextStyle::new(13.0, 400.0), p.muted).wrap_anywhere());
        }
        if machine.failed {
            control = control.child(settings_button(
                &format!("fleet-retry-{index}"),
                "Retry",
                p,
                move |s| s.start_fleet_row(index),
            ));
        }
        rows = rows.child(
            col()
                .key(format!("fleet-row-{index}"))
                .w_full()
                .child(
                    row()
                        .py(12.0)
                        .gap(16.0)
                        .items_start()
                        .child(
                            col()
                                .grow(1.0)
                                .min_w(0.0)
                                .gap(4.0)
                                .child(
                                    label(&machine.name, TextStyle::new(14.0, 500.0), p.ink).wrap(),
                                )
                                .child(
                                    row()
                                        .gap(8.0)
                                        .flex_wrap()
                                        .child(pixel_text(&machine.current, p.muted))
                                        .child(label("to", TextStyle::new(13.0, 400.0), p.muted))
                                        .child(pixel_text(&machine.target, p.muted)),
                                ),
                        )
                        .child(control),
                )
                .child(el().h(1.0).w_full().fill(p.line)),
        );
    }
    card = card.child(rows).child(
        primary_button("fleet-confirm", "Update eligible machines", p, |s| {
            s.confirm_fleet()
        })
        .disabled(
            s.fleet.running
                || s.fleet.rows.iter().any(|r| r.state == "checking")
                || !s.fleet.rows.iter().any(|r| r.eligible),
        ),
    );
    el().key("fleet-backdrop")
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .fill(Color::BLACK.alpha(0.28))
        .center()
        .modal(false, |s: &mut State| s.fleet.open = false)
        .child(card)
}

pub(super) fn archive_dialog(s: &State, p: Palette) -> El {
    let id = s.settings.archive.unwrap();
    el().absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .fill(Color::BLACK.alpha(0.28))
        .center()
        .modal(false, |s: &mut State| s.settings.archive = None)
        .child(
            col()
                .key("archive-confirm")
                .w(360.0)
                .padding(16.0)
                .gap(12.0)
                .radius(6.0)
                .fill(p.card)
                .border(2.0, p.frame)
                .box_shadow(hard_shadow(p))
                .child(label("Archive this thread?", DIALOG_TITLE, p.ink))
                .child(
                    row()
                        .gap(8.0)
                        .child(settings_button("archive-cancel", "Cancel", p, |s| {
                            s.settings.archive = None
                        }))
                        .child(primary_button("archive-accept", "Archive", p, move |s| {
                            s.settings.archive = None;
                            let targets = std::mem::take(&mut s.settings.archive_targets);
                            if targets.is_empty() {
                                s.archive_confirmed(id);
                            } else {
                                for id in targets {
                                    s.archive_confirmed(id);
                                }
                            }
                        })),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::tests::runtime;

    #[test]
    fn every_settings_section_and_search_fit_the_supported_widths() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            rt.state.open_settings();
            for (section, key) in [
                "settings-profile-row",
                "settings-auto-settle",
                "settings-harness-Pi",
                "settings-machines",
                "settings-honcho",
                "settings-version",
                "settings-about-version",
                "keybindings-search",
            ]
            .iter()
            .enumerate()
            {
                rt.state.settings.section = section;
                rt.invalidate();
                rt.frame(0.0);
                rt.frame(0.2);
                for key in [
                    "settings-menu",
                    "settings-search",
                    "settings-sections",
                    "settings-content",
                    key,
                ] {
                    let (rect, _) = rt
                        .presented(key)
                        .unwrap_or_else(|| panic!("{width}, section {section}: {key}"));
                    assert!(
                        rect[0] >= 0.0 && rect[0] + rect[2] <= width + 0.5,
                        "{width} {key}: {rect:?}"
                    );
                    assert!(
                        rect[1] >= 0.0 && rect[1] + rect[3] <= 720.5,
                        "{width} {key}: {rect:?}"
                    );
                }
                let (card, _) = rt.presented("settings-menu").unwrap();
                if width >= 1280.0 {
                    assert_eq!((card[2], card[3]), (880.0, 600.0));
                }
                if width == 620.0 {
                    assert_eq!((card[2], card[3]), (620.0, 720.0));
                }
            }
            for key in [
                "settings-send-shortcut",
                "settings-send-shortcut-enter",
                "settings-send-shortcut-mod-enter-multiline",
                "settings-send-shortcut-mod-enter",
            ] {
                rt.state.settings.section = 0;
                rt.state.settings.search.replace_text("");
                rt.invalidate();
                rt.frame(0.0);
                let (rect, _) = rt
                    .presented(key)
                    .unwrap_or_else(|| panic!("{width}: {key}"));
                assert!(
                    rect[0] >= 0.0 && rect[0] + rect[2] <= width + 0.5,
                    "{width} {key}: {rect:?}"
                );
                assert!(
                    rect[1] >= 0.0 && rect[1] + rect[3] <= 720.5,
                    "{width} {key}: {rect:?}"
                );
            }
            rt.state.settings.search.replace_text("Theme");
            rt.invalidate();
            rt.frame(0.0);
            assert!(rt.presented("settings-theme").is_some());
            assert!(rt.find("settings-auto-settle").is_none());
            rt.state.settings.search.replace_text("no such setting");
            rt.invalidate();
            rt.frame(0.0);
            assert!(rt.presented("settings-no-results").is_some());
        }
    }

    #[test]
    fn fleet_dialog_represents_every_state_without_window_overflow() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            rt.state.demo_fleet();
            rt.frame(0.0);
            rt.frame(0.2);
            for key in ["fleet-update-dialog", "fleet-close", "fleet-confirm"] {
                let (rect, _) = rt
                    .presented(key)
                    .unwrap_or_else(|| panic!("{width}: {key}"));
                assert!(
                    rect[0] >= 0.0 && rect[0] + rect[2] <= width + 0.5,
                    "{width} {key}: {rect:?}"
                );
                assert!(
                    rect[1] >= 0.0 && rect[1] + rect[3] <= 720.5,
                    "{width} {key}: {rect:?}"
                );
            }
            for index in 0..rt.state.fleet.rows.len() {
                assert!(rt.presented(&format!("fleet-row-{index}")).is_some());
            }
        }
    }

    #[test]
    fn server_settings_commands_use_the_dialog_machine_not_the_open_thread() {
        let mut state = State::new(".".into(), false);
        let (mut local, mut local_calls) =
            crate::client::Client::test_connected(state.data_dir.clone());
        let (remote, mut remote_calls) =
            crate::client::Client::test_connected(state.data_dir.clone());
        local.linked.insert("remote".into(), remote);
        state.client = Some(local);
        state.settings.machine = "remote".into();
        state.settings_command("settings.autoSettle", json!({"days":7}));
        assert_eq!(remote_calls.try_recv().unwrap().params["days"], 7);
        state.machines.views.insert(
            "remote".into(),
            json!({
                "profile":{"name":"Remote user","time_zone":"UTC","preferences":"Remote preferences"},
                "keepAwake":"on_ac_power"
            }),
        );
        assert_eq!(state.settings_profile().name, "Remote user");
        assert_eq!(
            state.settings_keep_awake(),
            crate::power::KeepAwake::OnAcPower
        );
        state.set_settings_keep_awake(crate::power::KeepAwake::WhileRunning);
        let call = remote_calls.try_recv().unwrap();
        assert_eq!(call.method, "background.keepAwake");
        assert_eq!(
            call.params["policy"],
            json!(crate::power::KeepAwake::WhileRunning)
        );
        state.settings_menu = true;
        captain_ui::open_profile(&mut state);
        assert_eq!(state.brief.profile_name.text, "Remote user");
        assert_eq!(state.settings.profile_target.as_deref(), Some("remote"));
        // The editor keeps its target even if the settings context changes meanwhile.
        state.settings.machine = "local".into();
        state
            .save_settings_profile(crate::model::UserProfile {
                name: "Updated remote user".into(),
                ..Default::default()
            })
            .unwrap();
        let call = remote_calls.try_recv().unwrap();
        assert_eq!(call.method, "profile.put");
        assert_eq!(call.params["profile"]["name"], "Updated remote user");
        assert!(local_calls.try_recv().is_err());
        state.open_settings();
        assert_eq!(state.settings.machine, "local");
        state.settings_command("settings.autoSettle", json!({"days":1}));
        assert_eq!(local_calls.try_recv().unwrap().params["days"], 1);
    }

    #[test]
    fn old_profiles_default_and_invalid_settings_fail_validation() {
        let defaults: ThreadDefaults = serde_json::from_value(json!({})).unwrap();
        assert_eq!(defaults.access, "full-access");
        assert_eq!(defaults.snooze_minutes, vec![60, 180]);
        let preferences: Preferences = serde_json::from_value(json!({})).unwrap();
        assert!(preferences.confirm_delete);
        assert!(!preferences.confirm_archive);
        assert_eq!(
            preferences.send_shortcut,
            crate::composer::SendShortcut::Enter
        );
        let mut invalid = defaults;
        invalid.snooze_minutes = vec![0];
        assert!(invalid.validate().is_err());
    }
    #[test]
    fn version_probes_drop_thread_credentials_and_never_forward_prompt_or_server_arguments() {
        let mut original = std::process::Command::new("node");
        original
            .arg("/tmp/fixture-agent.mjs")
            .arg("app-server")
            .arg("a prompt");
        original.env(crate::mcp::TOKEN_ENV, "not-a-real-token");
        let probe = version_command(&original);
        assert_eq!(
            probe
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            vec!["/tmp/fixture-agent.mjs", "--version"]
        );
        for name in [
            crate::mcp::TOKEN_ENV,
            crate::mcp::URL_ENV,
            "SHINA_RUNTIME_MODE",
            "SHINA_PI_BRIDGE_PATH",
        ] {
            assert!(
                probe
                    .get_envs()
                    .any(|(key, value)| key == name && value.is_none())
            );
        }
    }
    #[test]
    fn snooze_editor_preserves_duration_and_calendar_kinds_with_bounded_values() {
        let mut defaults = ThreadDefaults::default();
        defaults
            .set_snooze_text("30, tomorrow@10:15, monday@09:00")
            .unwrap();
        assert_eq!(defaults.snooze_minutes, vec![30]);
        assert_eq!(defaults.snooze_calendar.len(), 2);
        assert_eq!(defaults.snooze_presets().len(), 3);
        let text = defaults.snooze_text();
        assert_eq!(text, "30, tomorrow@10:15, monday@09:00");
        assert!(defaults.set_snooze_text("60000").is_err());
        assert!(defaults.set_snooze_text("tomorrow@25:00").is_err());
        assert!(defaults.set_snooze_text("never@09:00").is_err());
    }

    #[test]
    fn keybindings_section_groups_rebinds_resets_and_reports_conflicts_at_supported_widths() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            rt.on_key = Some(crate::key);
            rt.state.open_settings();
            rt.frame(0.0);
            crate::ui::tests::press(&mut rt, "settings-section-7");
            assert_eq!(rt.state.settings.section, 7);

            let (kb_rect, _) = rt
                .presented("settings-keybindings")
                .unwrap_or_else(|| panic!("{width}: settings-keybindings"));
            assert!(
                kb_rect[0] >= 0.0 && kb_rect[0] + kb_rect[2] <= width + 0.5,
                "{width} settings-keybindings: {kb_rect:?}"
            );
            for key in [
                "keybindings-search",
                "keybindings-group-navigation",
                "keybinding-row-commandPalette.toggle",
                "keybinding-pill-commandPalette.toggle",
            ] {
                let (rect, _) = rt
                    .presented(key)
                    .unwrap_or_else(|| panic!("{width}: {key}"));
                assert!(
                    rect[0] >= 0.0 && rect[0] + rect[2] <= width + 0.5,
                    "{width} {key}: {rect:?}"
                );
                assert!(
                    rect[1] >= 0.0 && rect[1] + rect[3] <= 720.5,
                    "{width} {key}: {rect:?}"
                );
            }

            // Group order matches KSG: Navigation, Threads, Composer, Terminal, Preview & diff,
            // Appearance, Other.
            let group_keys: Vec<_> = rt
                .elements()
                .into_iter()
                .filter_map(|e| e.key)
                .filter(|k| k.starts_with("keybindings-group-"))
                .collect();
            assert_eq!(
                group_keys,
                vec![
                    "keybindings-group-navigation",
                    "keybindings-group-threads",
                    "keybindings-group-composer",
                    "keybindings-group-terminal",
                    "keybindings-group-preview",
                    "keybindings-group-appearance",
                    "keybindings-group-other",
                ]
            );

            // Click the pill to start recording, then Escape cancels without closing Settings.
            crate::ui::tests::press(&mut rt, "keybinding-pill-commandPalette.toggle");
            assert!(
                rt.presented("keybinding-capture-commandPalette.toggle")
                    .is_some()
            );
            rt.key(rune::KeyEvent {
                key: rune::Key::Escape,
                mods: Default::default(),
            });
            rt.frame(0.0);
            assert!(rt.state.settings_menu);
            assert!(rt.state.settings.shortcut_capture.is_none());
            assert!(
                rt.presented("keybinding-pill-commandPalette.toggle")
                    .is_some()
            );

            // Click again and rebind commandPalette.toggle to mod+b, conflicting with sidebar.toggle.
            crate::ui::tests::press(&mut rt, "keybinding-pill-commandPalette.toggle");
            rt.key(rune::KeyEvent {
                key: rune::Key::Command('b'),
                mods: rune::Mods {
                    cmd: true,
                    ..Default::default()
                },
            });
            rt.frame(0.0);
            assert_eq!(
                rt.state.preferences.keybindings,
                vec![crate::keybindings::KeybindingRule {
                    key: "mod+b".into(),
                    command: "commandPalette.toggle".into(),
                    when: Some("!terminalFocus".into()),
                }]
            );
            assert!(
                rt.presented("keybinding-source-commandPalette.toggle")
                    .is_some()
            );
            assert!(
                rt.presented("keybinding-reset-commandPalette.toggle")
                    .is_some()
            );
            let (conflict_rect, _) = rt
                .presented("keybinding-conflict-commandPalette.toggle")
                .unwrap_or_else(|| panic!("{width}: conflict warning missing"));
            assert!(
                conflict_rect[0] >= 0.0 && conflict_rect[0] + conflict_rect[2] <= width + 0.5,
                "{width} conflict: {conflict_rect:?}"
            );
            let conflict_el = rt
                .elements()
                .into_iter()
                .find(|e| e.key.as_deref() == Some("keybinding-conflict-commandPalette.toggle"))
                .unwrap();
            assert_eq!(
                conflict_el.text.as_deref(),
                Some(
                    "Conflicts with Sidebar: Toggle. The most recent matching binding wins when both conditions can apply."
                )
            );

            // Reset to default removes the override and clears the conflict.
            crate::ui::tests::press(&mut rt, "keybinding-reset-commandPalette.toggle");
            assert!(rt.state.preferences.keybindings.is_empty());
            assert!(
                rt.find("keybinding-conflict-commandPalette.toggle")
                    .is_none()
            );
            assert!(rt.find("keybinding-reset-commandPalette.toggle").is_none());

            // Search filters the list and shows KSG's empty message on no match.
            rt.state
                .settings
                .shortcut_search
                .replace_text("modelPicker.toggle");
            rt.invalidate();
            rt.frame(0.0);
            assert!(rt.presented("keybinding-row-modelPicker.toggle").is_some());
            assert!(rt.find("keybinding-row-commandPalette.toggle").is_none());

            rt.state
                .settings
                .shortcut_search
                .replace_text("no-such-keybinding-xyz");
            rt.invalidate();
            rt.frame(0.0);
            assert!(rt.presented("keybindings-no-results").is_some());
        }
    }
}
