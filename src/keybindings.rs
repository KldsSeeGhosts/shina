//! Keybinding registry, chord parsing, `when` evaluation, conflict detection, and override
//! persistence, ported from KSG Code (`packages/contracts/src/keybindings.ts`,
//! `packages/shared/src/keybindings.ts`, `packages/shared/src/keybindingMatching.ts`,
//! `apps/web/src/keybindings.ts`, and `apps/web/src/components/settings/KeybindingsSettings.logic.ts`).

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

pub const MAX_KEYBINDING_VALUE_LENGTH: usize = 64;
pub const MAX_KEYBINDING_WHEN_LENGTH: usize = 256;
pub const MAX_WHEN_EXPRESSION_DEPTH: usize = 64;
pub const MAX_KEYBINDINGS_COUNT: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Mac,
    Windows,
    Linux,
}

impl Platform {
    pub const fn current() -> Self {
        if cfg!(any(target_os = "macos", target_os = "ios")) {
            Self::Mac
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Linux
        }
    }

    #[allow(dead_code)]
    pub fn from_name(name: &str) -> Self {
        let lower = name.to_ascii_lowercase();
        if lower.contains("mac")
            || lower.contains("iphone")
            || lower.contains("ipad")
            || lower.contains("ipod")
        {
            Self::Mac
        } else if lower.contains("win") {
            Self::Windows
        } else {
            Self::Linux
        }
    }

    pub const fn is_mac(self) -> bool {
        matches!(self, Self::Mac)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeybindingShortcut {
    pub key: String,
    pub meta_key: bool,
    pub ctrl_key: bool,
    pub shift_key: bool,
    pub alt_key: bool,
    pub mod_key: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum WhenNode {
    Identifier {
        name: String,
    },
    Not {
        node: Box<WhenNode>,
    },
    And {
        left: Box<WhenNode>,
        right: Box<WhenNode>,
    },
    Or {
        left: Box<WhenNode>,
        right: Box<WhenNode>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeybindingRule {
    pub key: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedKeybindingRule {
    pub command: String,
    pub shortcut: KeybindingShortcut,
    pub when_ast: Option<WhenNode>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShortcutContext {
    pub terminal_focus: bool,
    pub terminal_open: bool,
    pub preview_focus: bool,
    pub preview_open: bool,
    pub is_web: bool,
    pub is_desktop: bool,
    pub editable_focus: bool,
    pub composer_focus: bool,
    pub draft_thread_route: bool,
    pub turn_running: bool,
    pub composer_requires_modifier: bool,
    pub model_picker_open: bool,
    pub right_panel_open: bool,
    pub usage_page_open: bool,
}

impl ShortcutContext {
    pub fn desktop() -> Self {
        Self {
            is_desktop: true,
            is_web: false,
            ..Default::default()
        }
    }

    pub fn get(&self, name: &str) -> bool {
        match name {
            "true" => true,
            "false" => false,
            "terminalFocus" => self.terminal_focus,
            "terminalOpen" => self.terminal_open,
            "previewFocus" => self.preview_focus,
            "previewOpen" => self.preview_open,
            "isWeb" => self.is_web,
            "isDesktop" => self.is_desktop,
            "editableFocus" => self.editable_focus,
            "composerFocus" => self.composer_focus,
            "draftThreadRoute" => self.draft_thread_route,
            "turnRunning" => self.turn_running,
            "composerRequiresModifier" => self.composer_requires_modifier,
            "modelPickerOpen" => self.model_picker_open,
            "rightPanelOpen" => self.right_panel_open,
            "usagePageOpen" => self.usage_page_open,
            _ => false,
        }
    }
}

pub const THREAD_JUMP_COMMANDS: [&str; 9] = [
    "thread.jump.1",
    "thread.jump.2",
    "thread.jump.3",
    "thread.jump.4",
    "thread.jump.5",
    "thread.jump.6",
    "thread.jump.7",
    "thread.jump.8",
    "thread.jump.9",
];

pub const MODEL_PICKER_JUMP_COMMANDS: [&str; 9] = [
    "modelPicker.jump.1",
    "modelPicker.jump.2",
    "modelPicker.jump.3",
    "modelPicker.jump.4",
    "modelPicker.jump.5",
    "modelPicker.jump.6",
    "modelPicker.jump.7",
    "modelPicker.jump.8",
    "modelPicker.jump.9",
];

/// Every static command id from KSG Code (`packages/contracts/src/keybindings.ts`),
/// plus Shina's existing global commands that have no KSG id (`composer.focus`, `view.zoomIn`,
/// `view.zoomOut`, `view.resetZoom`).
pub const STATIC_COMMANDS: &[&str] = &[
    "workspace.focusNext",
    "workspace.focusPrevious",
    "sidebar.toggle",
    "navigation.back",
    "navigation.forward",
    "terminal.toggle",
    "terminal.split",
    "terminal.splitVertical",
    "terminal.new",
    "terminal.close",
    "rightPanel.toggle",
    "rightPanel.new",
    "threadPanel.toggle",
    "rightPanel.toggleMaximized",
    "rightPanel.close",
    "view.reopenClosed",
    "pullRequest.copyNumber",
    "diff.toggle",
    "preview.toggle",
    "preview.refresh",
    "preview.focusUrl",
    "preview.zoomIn",
    "preview.zoomOut",
    "preview.resetZoom",
    "preview.find",
    "commandPalette.toggle",
    "filePicker.toggle",
    "projectSearch.toggle",
    "usage.open",
    "theme.select",
    "appearance.cycle",
    "themeEditor.toggle",
    "composer.stash",
    "composer.sendAlternate",
    "composer.sendBackground",
    "composer.sendAndNewThread",
    "composer.dictation",
    "composer.host",
    "composer.cycleHost",
    "composer.effort",
    "composer.increaseEffort",
    "composer.decreaseEffort",
    "composer.mode",
    "composer.workspace",
    "composer.previousWorktree",
    "composer.branch",
    "chat.new",
    "chat.newLocal",
    "chat.newWithoutProject",
    "chat.find",
    "editor.openFavorite",
    "usage.cost",
    "usage.tokens",
    "usage.limits",
    "usage.period.day",
    "usage.period.week",
    "usage.period.month",
    "usage.period.quarter",
    "modelPicker.toggle",
    "modelPicker.previousProvider",
    "modelPicker.nextProvider",
    "modelPicker.jump.1",
    "modelPicker.jump.2",
    "modelPicker.jump.3",
    "modelPicker.jump.4",
    "modelPicker.jump.5",
    "modelPicker.jump.6",
    "modelPicker.jump.7",
    "modelPicker.jump.8",
    "modelPicker.jump.9",
    "thread.stop",
    "thread.steerQueuedMessage",
    "thread.editQueuedMessage",
    "thread.previous",
    "thread.next",
    "thread.copyReference",
    "thread.settle",
    "thread.pin",
    "thread.regenerateTitle",
    "thread.undo",
    "thread.jump.1",
    "thread.jump.2",
    "thread.jump.3",
    "thread.jump.4",
    "thread.jump.5",
    "thread.jump.6",
    "thread.jump.7",
    "thread.jump.8",
    "thread.jump.9",
    // Shina-only global commands that preserve existing shortcuts in src/main.rs
    "composer.focus",
    "view.zoomIn",
    "view.zoomOut",
    "view.resetZoom",
];

pub fn is_valid_command(command: &str) -> bool {
    if STATIC_COMMANDS.contains(&command) {
        return true;
    }
    let Some(id) = command
        .strip_prefix("script.")
        .and_then(|rest| rest.strip_suffix(".run"))
    else {
        return false;
    };
    !id.is_empty()
        && id.len() <= 24
        && (id.as_bytes()[0].is_ascii_lowercase() || id.as_bytes()[0].is_ascii_digit())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

pub fn default_keybinding_rules() -> Vec<KeybindingRule> {
    let mut rules = vec![
        rule("mod+b", "sidebar.toggle", None),
        rule("f6", "workspace.focusNext", None),
        rule("shift+f6", "workspace.focusPrevious", None),
        rule("mod+f6", "workspace.focusNext", None),
        rule("mod+shift+f6", "workspace.focusPrevious", None),
        rule("mod+[", "navigation.back", Some("!terminalFocus")),
        rule("mod+]", "navigation.forward", Some("!terminalFocus")),
        rule("mod+j", "terminal.toggle", None),
        rule("mod+alt+b", "rightPanel.toggle", None),
        rule("mod+shift+b", "threadPanel.toggle", Some("!terminalFocus")),
        rule(
            "mod+t",
            "rightPanel.new",
            Some("rightPanelOpen && !terminalFocus"),
        ),
        rule("mod+d", "terminal.split", Some("terminalFocus")),
        rule(
            "mod+shift+d",
            "terminal.splitVertical",
            Some("terminalFocus"),
        ),
        rule("mod+shift+d", "composer.dictation", Some("!terminalFocus")),
        rule("mod+n", "terminal.new", Some("terminalFocus")),
        rule("mod+w", "terminal.close", Some("terminalFocus")),
        rule("mod+w", "rightPanel.close", Some("!terminalFocus")),
        rule("mod+shift+t", "view.reopenClosed", None),
        rule("mod+d", "diff.toggle", Some("!terminalFocus")),
        rule("mod+shift+j", "preview.toggle", None),
        rule("mod+r", "preview.refresh", Some("previewFocus")),
        rule("mod+l", "preview.focusUrl", Some("previewFocus")),
        rule("mod+=", "preview.zoomIn", Some("previewFocus")),
        rule("mod++", "preview.zoomIn", Some("previewFocus")),
        rule("mod+-", "preview.zoomOut", Some("previewFocus")),
        rule("mod+0", "preview.resetZoom", Some("previewFocus")),
        rule("mod+f", "preview.find", Some("previewFocus")),
        rule("mod+k", "commandPalette.toggle", Some("!terminalFocus")),
        rule("mod+p", "filePicker.toggle", Some("!terminalFocus")),
        rule(
            "mod+shift+f",
            "projectSearch.toggle",
            Some("!terminalFocus"),
        ),
        rule("mod+u", "usage.open", Some("!terminalFocus")),
        rule("mod+alt+a", "theme.select", Some("!terminalFocus")),
        rule(
            "mod+alt+shift+a",
            "appearance.cycle",
            Some("!terminalFocus"),
        ),
        rule("mod+alt+shift+t", "themeEditor.toggle", None),
        rule("mod+s", "composer.stash", Some("!terminalFocus")),
        rule(
            "mod+shift+enter",
            "thread.steerQueuedMessage",
            Some("!terminalFocus"),
        ),
        rule(
            "alt+arrowup",
            "thread.editQueuedMessage",
            Some("composerFocus"),
        ),
        rule(
            "mod+enter",
            "composer.sendAlternate",
            Some("composerFocus && turnRunning"),
        ),
        rule(
            "mod+enter",
            "composer.sendBackground",
            Some("composerFocus && draftThreadRoute"),
        ),
        rule(
            "mod+alt+enter",
            "composer.sendBackground",
            Some("composerFocus && draftThreadRoute"),
        ),
        rule(
            "mod+alt+enter",
            "composer.sendAndNewThread",
            Some("composerFocus && !draftThreadRoute"),
        ),
        rule("mod+n", "chat.new", Some("!terminalFocus")),
        rule("mod+shift+o", "chat.new", Some("!terminalFocus")),
        rule("mod+shift+n", "chat.newLocal", Some("!terminalFocus")),
        rule(
            "mod+alt+n",
            "chat.newWithoutProject",
            Some("!terminalFocus"),
        ),
        rule(
            "mod+f",
            "chat.find",
            Some("!terminalFocus && !previewFocus"),
        ),
        rule("mod+shift+m", "modelPicker.toggle", Some("!terminalFocus")),
        rule("mod+shift+h", "composer.host", Some("!terminalFocus")),
        rule("mod+shift+e", "composer.effort", Some("!terminalFocus")),
        rule("mod+shift+a", "composer.mode", Some("!terminalFocus")),
        rule("mod+shift+x", "composer.workspace", Some("!terminalFocus")),
        rule("mod+shift+g", "composer.branch", Some("!terminalFocus")),
        rule(
            "mod+shift+l",
            "composer.previousWorktree",
            Some("!terminalFocus"),
        ),
        rule(
            "mod+shift+k",
            "pullRequest.copyNumber",
            Some("!terminalFocus"),
        ),
        rule(
            "mod+shift+arrowup",
            "modelPicker.previousProvider",
            Some("modelPickerOpen"),
        ),
        rule(
            "mod+shift+arrowdown",
            "modelPicker.nextProvider",
            Some("modelPickerOpen"),
        ),
        rule("mod+o", "editor.openFavorite", None),
        rule("mod+shift+[", "thread.previous", None),
        rule("mod+shift+]", "thread.next", None),
        rule(
            "mod+shift+c",
            "thread.copyReference",
            Some("!terminalFocus"),
        ),
        rule("mod+shift+s", "thread.settle", Some("!terminalFocus")),
        rule("mod+shift+p", "thread.pin", Some("!terminalFocus")),
        rule(
            "mod+z",
            "thread.undo",
            Some("!terminalFocus && !editableFocus"),
        ),
    ];
    for (index, command) in THREAD_JUMP_COMMANDS.iter().enumerate() {
        rules.push(rule(
            &format!("mod+{}", index + 1),
            command,
            Some("!modelPickerOpen && isDesktop"),
        ));
    }
    for (index, command) in MODEL_PICKER_JUMP_COMMANDS.iter().enumerate() {
        rules.push(rule(
            &format!("mod+{}", index + 1),
            command,
            Some("modelPickerOpen && isDesktop"),
        ));
    }
    rules.extend([
        rule("c", "usage.cost", Some("usagePageOpen")),
        rule("t", "usage.tokens", Some("usagePageOpen")),
        rule("l", "usage.limits", Some("usagePageOpen")),
        rule("mod+shift+1", "usage.period.day", Some("usagePageOpen")),
        rule("mod+shift+2", "usage.period.week", Some("usagePageOpen")),
        rule("mod+shift+3", "usage.period.month", Some("usagePageOpen")),
        rule("mod+shift+4", "usage.period.quarter", Some("usagePageOpen")),
        // Shina-only global commands preserving existing shortcuts in src/main.rs
        rule("mod+l", "composer.focus", Some("!previewFocus")),
        rule("mod+=", "view.zoomIn", Some("!previewFocus")),
        rule("mod++", "view.zoomIn", Some("!previewFocus")),
        rule("mod+shift+=", "view.zoomIn", Some("!previewFocus")),
        rule("mod+-", "view.zoomOut", Some("!previewFocus")),
        rule("mod+shift+-", "view.zoomOut", Some("!previewFocus")),
        rule("mod+0", "view.resetZoom", Some("!previewFocus")),
        rule(
            "escape",
            "thread.stop",
            Some("turnRunning && (!editableFocus || composerFocus)"),
        ),
    ]);
    rules
}

fn rule(key: &str, command: &str, when: Option<&str>) -> KeybindingRule {
    KeybindingRule {
        key: key.into(),
        command: command.into(),
        when: when.map(str::to_owned),
    }
}

pub fn default_resolved_keybindings() -> Vec<ResolvedKeybindingRule> {
    compile_resolved_keybindings(&default_keybinding_rules())
}

fn normalize_key_token(token: &str) -> String {
    match token {
        "space" => " ".into(),
        "esc" => "escape".into(),
        other => other.into(),
    }
}

pub fn parse_keybinding_shortcut(value: &str) -> Option<KeybindingShortcut> {
    let trimmed_value = value.trim();
    if trimmed_value.is_empty() || trimmed_value.len() > MAX_KEYBINDING_VALUE_LENGTH {
        return None;
    }
    let mut tokens: Vec<String> = trimmed_value
        .to_ascii_lowercase()
        .split('+')
        .map(|token| token.trim().to_owned())
        .collect();
    let mut trailing_empty_count = 0usize;
    while tokens.last().is_some_and(String::is_empty) {
        trailing_empty_count += 1;
        tokens.pop();
    }
    if trailing_empty_count > 0 {
        tokens.push("+".into());
    }
    if tokens.is_empty() || tokens.iter().any(String::is_empty) {
        return None;
    }

    let mut key: Option<String> = None;
    let mut meta_key = false;
    let mut ctrl_key = false;
    let mut shift_key = false;
    let mut alt_key = false;
    let mut mod_key = false;

    for token in tokens {
        match token.as_str() {
            "cmd" | "meta" => meta_key = true,
            "ctrl" | "control" => ctrl_key = true,
            "shift" => shift_key = true,
            "alt" | "option" => alt_key = true,
            "mod" => mod_key = true,
            _ => {
                if key.is_some() {
                    return None;
                }
                key = Some(normalize_key_token(&token));
            }
        }
    }

    Some(KeybindingShortcut {
        key: key?,
        meta_key,
        ctrl_key,
        shift_key,
        alt_key,
        mod_key,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum WhenToken {
    Identifier(String),
    Not,
    And,
    Or,
    LParen,
    RParen,
}

fn tokenize_when_expression(expression: &str) -> Option<Vec<WhenToken>> {
    let mut tokens = Vec::new();
    let bytes = expression.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        let ch = bytes[index] as char;
        if ch.is_ascii_whitespace() {
            index += 1;
            continue;
        }
        if expression[index..].starts_with("&&") {
            tokens.push(WhenToken::And);
            index += 2;
            continue;
        }
        if expression[index..].starts_with("||") {
            tokens.push(WhenToken::Or);
            index += 2;
            continue;
        }
        if ch == '!' {
            tokens.push(WhenToken::Not);
            index += 1;
            continue;
        }
        if ch == '(' {
            tokens.push(WhenToken::LParen);
            index += 1;
            continue;
        }
        if ch == ')' {
            tokens.push(WhenToken::RParen);
            index += 1;
            continue;
        }
        if ch.is_ascii_alphabetic() || ch == '_' {
            let start = index;
            index += 1;
            while index < bytes.len() {
                let c = bytes[index] as char;
                if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
                    index += 1;
                } else {
                    break;
                }
            }
            tokens.push(WhenToken::Identifier(expression[start..index].to_owned()));
            continue;
        }
        return None;
    }

    Some(tokens)
}

pub fn parse_keybinding_when_expression(expression: &str) -> Option<WhenNode> {
    let trimmed = expression.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_KEYBINDING_WHEN_LENGTH {
        return None;
    }
    let tokens = tokenize_when_expression(trimmed)?;
    if tokens.is_empty() {
        return None;
    }
    let mut index = 0usize;
    let ast = parse_when_or(&tokens, &mut index, 0)?;
    if index != tokens.len() {
        return None;
    }
    Some(ast)
}

fn parse_when_primary(tokens: &[WhenToken], index: &mut usize, depth: usize) -> Option<WhenNode> {
    if depth > MAX_WHEN_EXPRESSION_DEPTH {
        return None;
    }
    match tokens.get(*index)? {
        WhenToken::Identifier(name) => {
            *index += 1;
            Some(WhenNode::Identifier { name: name.clone() })
        }
        WhenToken::LParen => {
            *index += 1;
            let node = parse_when_or(tokens, index, depth + 1)?;
            match tokens.get(*index) {
                Some(WhenToken::RParen) => {
                    *index += 1;
                    Some(node)
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn parse_when_unary(tokens: &[WhenToken], index: &mut usize, depth: usize) -> Option<WhenNode> {
    let mut not_count = 0usize;
    while matches!(tokens.get(*index), Some(WhenToken::Not)) {
        *index += 1;
        not_count += 1;
        if not_count > MAX_WHEN_EXPRESSION_DEPTH {
            return None;
        }
    }
    let mut node = parse_when_primary(tokens, index, depth)?;
    while not_count > 0 {
        node = WhenNode::Not {
            node: Box::new(node),
        };
        not_count -= 1;
    }
    Some(node)
}

fn parse_when_and(tokens: &[WhenToken], index: &mut usize, depth: usize) -> Option<WhenNode> {
    let mut left = parse_when_unary(tokens, index, depth)?;
    while matches!(tokens.get(*index), Some(WhenToken::And)) {
        *index += 1;
        let right = parse_when_unary(tokens, index, depth)?;
        left = WhenNode::And {
            left: Box::new(left),
            right: Box::new(right),
        };
    }
    Some(left)
}

fn parse_when_or(tokens: &[WhenToken], index: &mut usize, depth: usize) -> Option<WhenNode> {
    let mut left = parse_when_and(tokens, index, depth)?;
    while matches!(tokens.get(*index), Some(WhenToken::Or)) {
        *index += 1;
        let right = parse_when_and(tokens, index, depth)?;
        left = WhenNode::Or {
            left: Box::new(left),
            right: Box::new(right),
        };
    }
    Some(left)
}

pub fn evaluate_when_node(node: &WhenNode, context: &ShortcutContext) -> bool {
    match node {
        WhenNode::Identifier { name } => context.get(name),
        WhenNode::Not { node } => !evaluate_when_node(node, context),
        WhenNode::And { left, right } => {
            evaluate_when_node(left, context) && evaluate_when_node(right, context)
        }
        WhenNode::Or { left, right } => {
            evaluate_when_node(left, context) || evaluate_when_node(right, context)
        }
    }
}

pub fn matches_when_clause(when_ast: Option<&WhenNode>, context: &ShortcutContext) -> bool {
    match when_ast {
        Some(node) => evaluate_when_node(node, context),
        None => true,
    }
}

pub fn compile_resolved_keybinding_rule(rule: &KeybindingRule) -> Option<ResolvedKeybindingRule> {
    let shortcut = parse_keybinding_shortcut(&rule.key)?;
    let when_ast = match &rule.when {
        Some(when) => Some(parse_keybinding_when_expression(when)?),
        None => None,
    };
    Some(ResolvedKeybindingRule {
        command: rule.command.clone(),
        shortcut,
        when_ast,
    })
}

pub fn compile_resolved_keybindings(rules: &[KeybindingRule]) -> Vec<ResolvedKeybindingRule> {
    let mut compiled: Vec<ResolvedKeybindingRule> = rules
        .iter()
        .filter_map(compile_resolved_keybinding_rule)
        .collect();
    if compiled.len() > MAX_KEYBINDINGS_COUNT {
        let start = compiled.len() - MAX_KEYBINDINGS_COUNT;
        compiled.drain(..start);
    }
    compiled
}

pub fn merge_with_default_keybindings(custom: &[KeybindingRule]) -> Vec<ResolvedKeybindingRule> {
    let defaults = default_resolved_keybindings();
    let compiled_custom = compile_resolved_keybindings(custom);
    if compiled_custom.is_empty() {
        return defaults;
    }
    let overridden: HashSet<&str> = compiled_custom
        .iter()
        .map(|binding| binding.command.as_str())
        .collect();
    let mut merged: Vec<ResolvedKeybindingRule> = defaults
        .into_iter()
        .filter(|binding| !overridden.contains(binding.command.as_str()))
        .collect();
    merged.extend(compiled_custom);
    if merged.len() > MAX_KEYBINDINGS_COUNT {
        let start = merged.len() - MAX_KEYBINDINGS_COUNT;
        merged.drain(..start);
    }
    merged
}

pub fn shortcut_conflict_key(shortcut: &KeybindingShortcut, platform: Platform) -> String {
    let use_meta_for_mod = platform.is_mac();
    let meta_key = shortcut.meta_key || (shortcut.mod_key && use_meta_for_mod);
    let ctrl_key = shortcut.ctrl_key || (shortcut.mod_key && !use_meta_for_mod);
    format!(
        "{}|{}|{}|{}|{}",
        shortcut.key,
        if meta_key { "meta" } else { "" },
        if ctrl_key { "ctrl" } else { "" },
        if shortcut.shift_key { "shift" } else { "" },
        if shortcut.alt_key { "alt" } else { "" },
    )
}

pub fn shortcut_for_context(
    binding: &ResolvedKeybindingRule,
    context: &ShortcutContext,
) -> KeybindingShortcut {
    let mut shortcut = binding.shortcut.clone();
    if context.composer_requires_modifier
        && binding.command == "composer.sendAlternate"
        && shortcut.key == "enter"
        && shortcut.mod_key
        && !shortcut.meta_key
        && !shortcut.ctrl_key
        && !shortcut.alt_key
        && !shortcut.shift_key
    {
        shortcut.shift_key = true;
    }
    shortcut
}

fn event_modifiers(event: &rune::KeyEvent, platform: Platform) -> (bool, bool, bool, bool) {
    let bare = event.mods.cmd && !event.mods.ctrl && !event.mods.logo;
    let (meta_key, ctrl_key) = if bare {
        (platform.is_mac(), !platform.is_mac())
    } else if platform.is_mac() {
        (event.mods.logo || event.mods.cmd, event.mods.ctrl)
    } else {
        (event.mods.logo, event.mods.ctrl || event.mods.cmd)
    };
    (meta_key, ctrl_key, event.mods.shift, event.mods.alt)
}

pub fn matches_shortcut_modifiers(
    event: &rune::KeyEvent,
    shortcut: &KeybindingShortcut,
    platform: Platform,
) -> bool {
    let use_meta_for_mod = platform.is_mac();
    let expected_meta = shortcut.meta_key || (shortcut.mod_key && use_meta_for_mod);
    let expected_ctrl = shortcut.ctrl_key || (shortcut.mod_key && !use_meta_for_mod);
    let (meta_key, ctrl_key, shift_key, alt_key) = event_modifiers(event, platform);
    meta_key == expected_meta
        && ctrl_key == expected_ctrl
        && shift_key == shortcut.shift_key
        && alt_key == shortcut.alt_key
}

fn us_shifted_to_unshifted(c: char) -> Option<char> {
    match c {
        '!' => Some('1'),
        '@' => Some('2'),
        '#' => Some('3'),
        '$' => Some('4'),
        '%' => Some('5'),
        '^' => Some('6'),
        '&' => Some('7'),
        '*' => Some('8'),
        '(' => Some('9'),
        ')' => Some('0'),
        '~' => Some('`'),
        '_' => Some('-'),
        '+' => Some('='),
        '{' => Some('['),
        '}' => Some(']'),
        '|' => Some('\\'),
        ':' => Some(';'),
        '"' => Some('\''),
        '<' => Some(','),
        '>' => Some('.'),
        '?' => Some('/'),
        _ => None,
    }
}

fn us_physical_key_token(code: rune::PhysicalKeyCode) -> Option<&'static str> {
    use rune::PhysicalKeyCode;
    Some(match code {
        PhysicalKeyCode::Backquote => "`",
        PhysicalKeyCode::Backslash => "\\",
        PhysicalKeyCode::BracketLeft => "[",
        PhysicalKeyCode::BracketRight => "]",
        PhysicalKeyCode::Comma => ",",
        PhysicalKeyCode::Digit0 => "0",
        PhysicalKeyCode::Digit1 => "1",
        PhysicalKeyCode::Digit2 => "2",
        PhysicalKeyCode::Digit3 => "3",
        PhysicalKeyCode::Digit4 => "4",
        PhysicalKeyCode::Digit5 => "5",
        PhysicalKeyCode::Digit6 => "6",
        PhysicalKeyCode::Digit7 => "7",
        PhysicalKeyCode::Digit8 => "8",
        PhysicalKeyCode::Digit9 => "9",
        PhysicalKeyCode::Equal => "=",
        PhysicalKeyCode::Minus => "-",
        PhysicalKeyCode::Period => ".",
        PhysicalKeyCode::Quote => "'",
        PhysicalKeyCode::Semicolon => ";",
        PhysicalKeyCode::Slash => "/",
        _ => return None,
    })
}

fn physical_letter_token(code: rune::PhysicalKeyCode) -> Option<char> {
    use rune::PhysicalKeyCode;
    Some(match code {
        PhysicalKeyCode::KeyA => 'a',
        PhysicalKeyCode::KeyB => 'b',
        PhysicalKeyCode::KeyC => 'c',
        PhysicalKeyCode::KeyD => 'd',
        PhysicalKeyCode::KeyE => 'e',
        PhysicalKeyCode::KeyF => 'f',
        PhysicalKeyCode::KeyG => 'g',
        PhysicalKeyCode::KeyH => 'h',
        PhysicalKeyCode::KeyI => 'i',
        PhysicalKeyCode::KeyJ => 'j',
        PhysicalKeyCode::KeyK => 'k',
        PhysicalKeyCode::KeyL => 'l',
        PhysicalKeyCode::KeyM => 'm',
        PhysicalKeyCode::KeyN => 'n',
        PhysicalKeyCode::KeyO => 'o',
        PhysicalKeyCode::KeyP => 'p',
        PhysicalKeyCode::KeyQ => 'q',
        PhysicalKeyCode::KeyR => 'r',
        PhysicalKeyCode::KeyS => 's',
        PhysicalKeyCode::KeyT => 't',
        PhysicalKeyCode::KeyU => 'u',
        PhysicalKeyCode::KeyV => 'v',
        PhysicalKeyCode::KeyW => 'w',
        PhysicalKeyCode::KeyX => 'x',
        PhysicalKeyCode::KeyY => 'y',
        PhysicalKeyCode::KeyZ => 'z',
        _ => return None,
    })
}

pub fn resolve_event_keys(
    event: &rune::KeyEvent,
    physical: Option<rune::PhysicalKeyCode>,
    key_base: Option<char>,
) -> HashSet<String> {
    let mut keys = HashSet::new();
    let mut layout_is_ascii_letter = false;
    match &event.key {
        rune::Key::Text(t) => {
            let lower = t.to_lowercase();
            let mut chars = lower.chars();
            if let (Some(ch), None) = (chars.next(), chars.next()) {
                layout_is_ascii_letter = ch.is_ascii_lowercase();
                if event.mods.shift
                    && let Some(unshifted) = us_shifted_to_unshifted(ch)
                {
                    keys.insert(unshifted.to_string());
                }
            }
            keys.insert(lower);
        }
        rune::Key::Command(ch) => {
            let lower = ch.to_ascii_lowercase();
            layout_is_ascii_letter = lower.is_ascii_lowercase();
            keys.insert(lower.to_string());
            if event.mods.shift
                && let Some(unshifted) = us_shifted_to_unshifted(lower)
            {
                keys.insert(unshifted.to_string());
            }
        }
        rune::Key::Enter => {
            keys.insert("enter".into());
        }
        rune::Key::Backspace => {
            keys.insert("backspace".into());
        }
        rune::Key::Delete => {
            keys.insert("delete".into());
        }
        rune::Key::Left => {
            keys.insert("arrowleft".into());
        }
        rune::Key::Right => {
            keys.insert("arrowright".into());
        }
        rune::Key::Up => {
            keys.insert("arrowup".into());
        }
        rune::Key::Down => {
            keys.insert("arrowdown".into());
        }
        rune::Key::Home => {
            keys.insert("home".into());
        }
        rune::Key::End => {
            keys.insert("end".into());
        }
        rune::Key::PageUp => {
            keys.insert("pageup".into());
        }
        rune::Key::PageDown => {
            keys.insert("pagedown".into());
        }
        rune::Key::Escape => {
            keys.insert("escape".into());
        }
        rune::Key::Tab => {
            keys.insert("tab".into());
        }
        rune::Key::Function(n) => {
            keys.insert(format!("f{n}"));
        }
        _ => {}
    }
    if let Some(base) = key_base {
        keys.insert(base.to_ascii_lowercase().to_string());
    }
    if let Some(code) = physical {
        if !layout_is_ascii_letter && let Some(letter) = physical_letter_token(code) {
            keys.insert(letter.to_string());
        }
        if let Some(token) = us_physical_key_token(code) {
            keys.insert(token.to_owned());
        }
    }
    keys
}

/// Ported from KSG Code's `isRichTextBoldShortcut` (`apps/web/src/keybindings.ts`):
/// Mod+B without extra modifiers, with non-empty text selection.
pub fn is_rich_text_bold_shortcut(
    event: &rune::KeyEvent,
    physical: Option<rune::PhysicalKeyCode>,
    key_base: Option<char>,
    has_selection: bool,
) -> bool {
    has_selection
        && (event.mods.cmd || event.mods.ctrl)
        && !event.mods.alt
        && !event.mods.shift
        && resolve_event_keys(event, physical, key_base).contains("b")
}

pub fn matches_keybinding_shortcut(
    event: &rune::KeyEvent,
    shortcut: &KeybindingShortcut,
    platform: Platform,
    physical: Option<rune::PhysicalKeyCode>,
    key_base: Option<char>,
) -> bool {
    if !matches_shortcut_modifiers(event, shortcut, platform) {
        return false;
    }
    resolve_event_keys(event, physical, key_base).contains(&shortcut.key)
}

pub fn resolve_shortcut_command<'a>(
    event: &rune::KeyEvent,
    keybindings: &'a [ResolvedKeybindingRule],
    context: &ShortcutContext,
    platform: Platform,
    physical: Option<rune::PhysicalKeyCode>,
    key_base: Option<char>,
) -> Option<&'a str> {
    for binding in keybindings.iter().rev() {
        if !matches_when_clause(binding.when_ast.as_ref(), context) {
            continue;
        }
        let shortcut = shortcut_for_context(binding, context);
        if matches_keybinding_shortcut(event, &shortcut, platform, physical, key_base) {
            return Some(binding.command.as_str());
        }
    }
    None
}

#[cfg(test)]
pub fn effective_shortcuts_for_command(
    keybindings: &[ResolvedKeybindingRule],
    command: &str,
    context: &ShortcutContext,
    platform: Platform,
) -> Vec<KeybindingShortcut> {
    let mut claimed = HashSet::new();
    let mut effective = Vec::new();
    for binding in keybindings.iter().rev() {
        if !matches_when_clause(binding.when_ast.as_ref(), context) {
            continue;
        }
        let shortcut = shortcut_for_context(binding, context);
        let conflict = shortcut_conflict_key(&shortcut, platform);
        if !claimed.insert(conflict) {
            continue;
        }
        if binding.command == command {
            effective.push(shortcut);
        }
    }
    effective
}

#[cfg(test)]
pub fn shortcut_label_for_command(
    keybindings: &[ResolvedKeybindingRule],
    command: &str,
    context: &ShortcutContext,
    platform: Platform,
) -> Option<String> {
    effective_shortcuts_for_command(keybindings, command, context, platform)
        .into_iter()
        .next()
        .map(|shortcut| format_shortcut_label(&shortcut, platform))
}

pub fn format_shortcut_key_label(key: &str) -> String {
    if key == " " {
        return "Space".into();
    }
    if key.chars().count() == 1 {
        return key.to_uppercase();
    }
    match key {
        "escape" => "Esc".into(),
        "arrowup" => "Up".into(),
        "arrowdown" => "Down".into(),
        "arrowleft" => "Left".into(),
        "arrowright" => "Right".into(),
        other => {
            let mut chars = other.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                None => String::new(),
            }
        }
    }
}

pub fn format_shortcut_label(shortcut: &KeybindingShortcut, platform: Platform) -> String {
    let key_label = format_shortcut_key_label(&shortcut.key);
    let use_meta_for_mod = platform.is_mac();
    let show_meta = shortcut.meta_key || (shortcut.mod_key && use_meta_for_mod);
    let show_ctrl = shortcut.ctrl_key || (shortcut.mod_key && !use_meta_for_mod);
    let show_alt = shortcut.alt_key;
    let show_shift = shortcut.shift_key;

    if use_meta_for_mod {
        format!(
            "{}{}{}{}{}",
            if show_ctrl { "\u{2303}" } else { "" },
            if show_alt { "\u{2325}" } else { "" },
            if show_shift { "\u{21e7}" } else { "" },
            if show_meta { "\u{2318}" } else { "" },
            key_label
        )
    } else {
        let mut parts = Vec::new();
        if show_ctrl {
            parts.push("Ctrl");
        }
        if show_alt {
            parts.push("Alt");
        }
        if show_shift {
            parts.push("Shift");
        }
        if show_meta {
            parts.push("Meta");
        }
        parts.push(&key_label);
        parts.join("+")
    }
}

pub fn keycap_parts(shortcut: &KeybindingShortcut, platform: Platform) -> Vec<String> {
    let use_meta_for_mod = platform.is_mac();
    let show_meta = shortcut.meta_key || (shortcut.mod_key && use_meta_for_mod);
    let show_ctrl = shortcut.ctrl_key || (shortcut.mod_key && !use_meta_for_mod);
    let mut parts = Vec::new();
    if use_meta_for_mod {
        if show_ctrl {
            parts.push("\u{2303}".into());
        }
        if shortcut.alt_key {
            parts.push("\u{2325}".into());
        }
        if shortcut.shift_key {
            parts.push("\u{21e7}".into());
        }
        if show_meta {
            parts.push("\u{2318}".into());
        }
    } else {
        if show_ctrl {
            parts.push("Ctrl".into());
        }
        if shortcut.alt_key {
            parts.push("Alt".into());
        }
        if shortcut.shift_key {
            parts.push("Shift".into());
        }
        if show_meta {
            parts.push("Meta".into());
        }
    }
    parts.push(format_shortcut_key_label(&shortcut.key));
    parts
}

pub fn shortcut_to_keybinding_input(shortcut: &KeybindingShortcut) -> String {
    let mut parts = Vec::new();
    if shortcut.mod_key {
        parts.push("mod");
    }
    if shortcut.meta_key {
        parts.push("meta");
    }
    if shortcut.ctrl_key {
        parts.push("ctrl");
    }
    if shortcut.alt_key {
        parts.push("alt");
    }
    if shortcut.shift_key {
        parts.push("shift");
    }
    let key_token = match shortcut.key.as_str() {
        " " => "space",
        "escape" => "esc",
        other => other,
    };
    parts.push(key_token);
    parts.join("+")
}

pub fn when_ast_to_expression(node: Option<&WhenNode>) -> String {
    let Some(node) = node else {
        return String::new();
    };
    match node {
        WhenNode::Identifier { name } => name.clone(),
        WhenNode::Not { node } => format!("!{}", wrap_when_expression(node)),
        WhenNode::And { left, right } => {
            format!(
                "{} && {}",
                wrap_when_expression(left),
                wrap_when_expression(right)
            )
        }
        WhenNode::Or { left, right } => {
            format!(
                "{} || {}",
                wrap_when_expression(left),
                wrap_when_expression(right)
            )
        }
    }
}

fn wrap_when_expression(node: &WhenNode) -> String {
    match node {
        WhenNode::Identifier { .. } | WhenNode::Not { .. } => when_ast_to_expression(Some(node)),
        WhenNode::And { .. } | WhenNode::Or { .. } => {
            format!("({})", when_ast_to_expression(Some(node)))
        }
    }
}

pub fn keybinding_from_key_event(
    event: &rune::KeyEvent,
    platform: Platform,
    physical: Option<rune::PhysicalKeyCode>,
    key_base: Option<char>,
) -> Option<String> {
    if matches!(event.key, rune::Key::Modifier { .. }) {
        return None;
    }
    let key_token = match &event.key {
        rune::Key::Enter => Some("enter".to_owned()),
        rune::Key::Tab => Some("tab".to_owned()),
        rune::Key::Backspace => Some("backspace".to_owned()),
        rune::Key::Delete => Some("delete".to_owned()),
        rune::Key::Home => Some("home".to_owned()),
        rune::Key::End => Some("end".to_owned()),
        rune::Key::PageUp => Some("pageup".to_owned()),
        rune::Key::PageDown => Some("pagedown".to_owned()),
        rune::Key::Up => Some("arrowup".to_owned()),
        rune::Key::Down => Some("arrowdown".to_owned()),
        rune::Key::Left => Some("arrowleft".to_owned()),
        rune::Key::Right => Some("arrowright".to_owned()),
        rune::Key::Escape => Some("esc".to_owned()),
        rune::Key::Function(n) => Some(format!("f{n}")),
        rune::Key::Command(ch) => {
            let lower = ch.to_ascii_lowercase();
            let unshifted = if event.mods.shift {
                us_shifted_to_unshifted(lower).unwrap_or(lower)
            } else {
                lower
            };
            Some(if unshifted == ' ' {
                "space".into()
            } else {
                unshifted.to_string()
            })
        }
        rune::Key::Text(text) => {
            let mut chars = text.chars();
            let ch = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            if ch == ' ' {
                Some("space".into())
            } else if let Some(base) = key_base.filter(|c| c.is_ascii_alphanumeric()) {
                Some(base.to_ascii_lowercase().to_string())
            } else if let Some(code) = physical.and_then(us_physical_key_token) {
                let lower = ch.to_ascii_lowercase();
                if event.mods.shift && us_shifted_to_unshifted(lower) == code.chars().next() {
                    Some(code.to_owned())
                } else if event.mods.alt {
                    Some(code.to_owned())
                } else {
                    Some(lower.to_string())
                }
            } else {
                let lower = ch.to_ascii_lowercase();
                let unshifted = if event.mods.shift {
                    us_shifted_to_unshifted(lower).unwrap_or(lower)
                } else {
                    lower
                };
                Some(unshifted.to_string())
            }
        }
        _ => None,
    }?;

    let (meta_key, ctrl_key, shift_key, alt_key) = event_modifiers(event, platform);
    let mut parts = Vec::new();
    if platform.is_mac() {
        if meta_key {
            parts.push("mod");
        }
        if ctrl_key {
            parts.push("ctrl");
        }
    } else {
        if ctrl_key {
            parts.push("mod");
        }
        if meta_key {
            parts.push("meta");
        }
    }
    if alt_key {
        parts.push("alt");
    }
    if shift_key {
        parts.push("shift");
    }
    parts.push(&key_token);
    Some(parts.join("+"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeybindingSource {
    Default,
    Custom,
}

impl KeybindingSource {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Custom => "Custom",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeybindingRow {
    pub id: String,
    pub command: String,
    pub key: String,
    pub when: String,
    pub source: KeybindingSource,
    pub default_key: Option<String>,
    pub default_when: String,
    pub binding: ResolvedKeybindingRule,
    pub conflicts: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeybindingGroup {
    pub id: &'static str,
    pub title: &'static str,
    pub rows: Vec<KeybindingRow>,
}

const KEYBINDING_GROUPS: &[(&str, &str, &[&str])] = &[
    (
        "navigation",
        "Navigation",
        &[
            "sidebar",
            "rightPanel",
            "commandPalette",
            "filePicker",
            "projectSearch",
            "editor",
        ],
    ),
    ("threads", "Threads", &["thread", "chat", "pullRequest"]),
    ("composer", "Composer", &["composer", "modelPicker"]),
    ("terminal", "Terminal", &["terminal"]),
    ("preview", "Preview & diff", &["preview", "diff"]),
    (
        "appearance",
        "Appearance",
        &["theme", "appearance", "themeEditor"],
    ),
    ("scripts", "Project scripts", &["script"]),
];

fn usage_command_rank(command: &str) -> Option<usize> {
    const USAGE_ORDER: [&str; 8] = [
        "usage.open",
        "usage.cost",
        "usage.tokens",
        "usage.limits",
        "usage.period.day",
        "usage.period.week",
        "usage.period.month",
        "usage.period.quarter",
    ];
    USAGE_ORDER.iter().position(|&item| item == command)
}

fn compare_commands(left: &str, right: &str) -> std::cmp::Ordering {
    match (usage_command_rank(left), usage_command_rank(right)) {
        (Some(l), Some(r)) => l.cmp(&r),
        (Some(_), None) => "usage.cost".cmp(right),
        (None, Some(_)) => left.cmp("usage.cost"),
        (None, None) => left.cmp(right),
    }
}

fn keybinding_group_for(command: &str) -> (&'static str, &'static str) {
    let prefix = command.split('.').next().unwrap_or("");
    KEYBINDING_GROUPS
        .iter()
        .find(|(_, _, prefixes)| prefixes.contains(&prefix))
        .map(|(id, title, _)| (*id, *title))
        .unwrap_or(("other", "Other"))
}

pub fn command_label(command: &str) -> String {
    match command {
        "workspace.focusNext" => "Focus Next Workspace Area".into(),
        "workspace.focusPrevious" => "Focus Previous Workspace Area".into(),
        "composer.sendAlternate" => "Composer: Opposite Queue or Steer Action".into(),
        "composer.sendBackground" => "Composer: Start in Background".into(),
        "composer.sendAndNewThread" => "Composer: Send and Start New Thread".into(),
        "composer.dictation" => "Composer: Start or Finish Dictation".into(),
        "composer.focus" => "Composer: Focus Input".into(),
        "thread.steerQueuedMessage" => "Queue: Send First Queued Message".into(),
        "thread.editQueuedMessage" => "Queue: Edit Last Queued Message".into(),
        "thread.copyReference" => "Pull Request: Copy Link or Thread".into(),
        "view.reopenClosed" => "Reopen Closed Tab".into(),
        "view.zoomIn" => "View: Zoom In".into(),
        "view.zoomOut" => "View: Zoom Out".into(),
        "view.resetZoom" => "View: Reset Zoom".into(),
        "preview.focusUrl" => "Preview: Focus URL".into(),
        "preview.find" => "Preview: Find on Page".into(),
        "usage.cost" => "Usage: Cost".into(),
        "usage.tokens" => "Usage: Tokens".into(),
        "usage.limits" => "Usage: Limits".into(),
        "usage.period.day" => "Usage: Period: Past 24h".into(),
        "usage.period.week" => "Usage: Period: 7 days".into(),
        "usage.period.month" => "Usage: Period: 30 days".into(),
        "usage.period.quarter" => "Usage: Period: 90 days".into(),
        _ => {
            if let Some(rest) = command
                .strip_prefix("script.")
                .and_then(|s| s.strip_suffix(".run"))
            {
                return format!("Run Script: {}", title_case_segment(rest));
            }
            command
                .split('.')
                .map(title_case_segment)
                .collect::<Vec<_>>()
                .join(": ")
        }
    }
}

fn title_case_segment(segment: &str) -> String {
    let mut spaced = String::new();
    let mut prev_lower_or_digit = false;
    for ch in segment.chars() {
        if ch == '-' || ch == '_' || ch.is_whitespace() {
            spaced.push(' ');
            prev_lower_or_digit = false;
        } else if ch.is_ascii_uppercase() && prev_lower_or_digit {
            spaced.push(' ');
            spaced.push(ch);
            prev_lower_or_digit = false;
        } else {
            spaced.push(ch);
            prev_lower_or_digit = ch.is_ascii_lowercase() || ch.is_ascii_digit();
        }
    }
    spaced
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn conflicts_with_when(left_when: &str, right_when: &str) -> bool {
    left_when.is_empty() || right_when.is_empty() || left_when == right_when
}

pub fn keybinding_conflict_labels(
    rows: &[KeybindingRow],
    row_id: &str,
    key: &str,
    when: &str,
    platform: Platform,
) -> Vec<String> {
    if key.trim().is_empty() {
        return Vec::new();
    }
    let Some(shortcut) = parse_keybinding_shortcut(key) else {
        return Vec::new();
    };
    let conflict_key = shortcut_conflict_key(&shortcut, platform);
    let mut conflicts: Vec<String> = rows
        .iter()
        .filter(|candidate| {
            candidate.id != row_id
                && shortcut_conflict_key(&candidate.binding.shortcut, platform) == conflict_key
                && conflicts_with_when(&candidate.when, when)
        })
        .map(|candidate| command_label(&candidate.command))
        .collect();
    conflicts.sort();
    conflicts.dedup();
    conflicts
}

fn source_for_binding(
    binding: &ResolvedKeybindingRule,
    defaults: &[ResolvedKeybindingRule],
) -> KeybindingSource {
    let binding_key = shortcut_to_keybinding_input(&binding.shortcut);
    let binding_when = when_ast_to_expression(binding.when_ast.as_ref());
    let is_default = defaults.iter().any(|entry| {
        entry.command == binding.command
            && shortcut_to_keybinding_input(&entry.shortcut) == binding_key
            && when_ast_to_expression(entry.when_ast.as_ref()) == binding_when
    });
    if is_default {
        KeybindingSource::Default
    } else {
        KeybindingSource::Custom
    }
}

fn default_binding_for_binding<'a>(
    binding: &ResolvedKeybindingRule,
    defaults: &'a [ResolvedKeybindingRule],
) -> Option<&'a ResolvedKeybindingRule> {
    let binding_key = shortcut_to_keybinding_input(&binding.shortcut);
    let binding_when = when_ast_to_expression(binding.when_ast.as_ref());
    defaults
        .iter()
        .find(|entry| {
            entry.command == binding.command
                && shortcut_to_keybinding_input(&entry.shortcut) == binding_key
                && when_ast_to_expression(entry.when_ast.as_ref()) == binding_when
        })
        .or_else(|| {
            defaults.iter().find(|entry| {
                entry.command == binding.command
                    && when_ast_to_expression(entry.when_ast.as_ref()) == binding_when
            })
        })
        .or_else(|| {
            defaults
                .iter()
                .find(|entry| entry.command == binding.command)
        })
}

pub fn build_keybinding_rows(
    keybindings: &[ResolvedKeybindingRule],
    platform: Platform,
) -> Vec<KeybindingRow> {
    let defaults = default_resolved_keybindings();
    let mut rows_by_id: BTreeMap<String, KeybindingRow> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    for binding in keybindings {
        let default_binding = default_binding_for_binding(binding, &defaults);
        let key = shortcut_to_keybinding_input(&binding.shortcut);
        let when = when_ast_to_expression(binding.when_ast.as_ref());
        let id = format!("{}\0{}\0{}", binding.command, key, when);
        let row = KeybindingRow {
            id: id.clone(),
            command: binding.command.clone(),
            key,
            when,
            source: source_for_binding(binding, &defaults),
            default_key: default_binding.map(|b| shortcut_to_keybinding_input(&b.shortcut)),
            default_when: when_ast_to_expression(default_binding.and_then(|b| b.when_ast.as_ref())),
            binding: binding.clone(),
            conflicts: Vec::new(),
        };
        if !rows_by_id.contains_key(&id) {
            order.push(id.clone());
        }
        rows_by_id.insert(id, row);
    }
    let mut rows: Vec<KeybindingRow> = order
        .into_iter()
        .filter_map(|id| rows_by_id.remove(&id))
        .collect();

    let snapshot = rows.clone();
    for row in &mut rows {
        row.conflicts =
            keybinding_conflict_labels(&snapshot, &row.id, &row.key, &row.when, platform);
    }
    rows.sort_by(|left, right| {
        compare_commands(&left.command, &right.command).then_with(|| left.key.cmp(&right.key))
    });
    rows
}

pub fn filter_keybinding_rows(rows: Vec<KeybindingRow>, query: &str) -> Vec<KeybindingRow> {
    let normalized = query.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return rows;
    }
    rows.into_iter()
        .filter(|row| {
            row.command.to_ascii_lowercase().contains(&normalized)
                || command_label(&row.command)
                    .to_ascii_lowercase()
                    .contains(&normalized)
                || row.key.to_ascii_lowercase().contains(&normalized)
                || row.when.to_ascii_lowercase().contains(&normalized)
                || row
                    .source
                    .label()
                    .to_ascii_lowercase()
                    .contains(&normalized)
        })
        .collect()
}

pub fn group_keybinding_rows(rows: Vec<KeybindingRow>) -> Vec<KeybindingGroup> {
    let mut grouped: BTreeMap<&'static str, Vec<KeybindingRow>> = BTreeMap::new();
    for row in rows {
        let (group_id, _) = keybinding_group_for(&row.command);
        grouped.entry(group_id).or_default().push(row);
    }
    let mut out = Vec::new();
    for (id, title, _) in KEYBINDING_GROUPS {
        if let Some(group_rows) = grouped.remove(id) {
            out.push(KeybindingGroup {
                id,
                title,
                rows: group_rows,
            });
        }
    }
    if let Some(other_rows) = grouped.remove("other") {
        out.push(KeybindingGroup {
            id: "other",
            title: "Other",
            rows: other_rows,
        });
    }
    out
}

pub fn conflict_warning_description(labels: &[String]) -> Option<String> {
    match labels.len() {
        0 => None,
        1 => Some(format!(
            "Conflicts with {}. The most recent matching binding wins when both conditions can apply.",
            labels[0]
        )),
        n => {
            let head = labels[..n.min(3)].join(", ");
            let suffix = if n > 3 { ", and more" } else { "" };
            Some(format!(
                "Conflicts with {head}{suffix}. The most recent matching binding wins when both conditions can apply."
            ))
        }
    }
}

pub fn build_keybinding_command_options(keybindings: &[ResolvedKeybindingRule]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut commands = Vec::new();
    for &cmd in STATIC_COMMANDS {
        if seen.insert(cmd.to_owned()) {
            commands.push(cmd.to_owned());
        }
    }
    for binding in keybindings {
        if seen.insert(binding.command.clone()) {
            commands.push(binding.command.clone());
        }
    }
    commands.sort_by(|left, right| {
        let left_rank = usage_command_rank(left);
        let right_rank = usage_command_rank(right);
        match (left_rank, right_rank) {
            (Some(l), Some(r)) => l.cmp(&r),
            _ => {
                let lk = if left_rank.is_some() {
                    command_label("usage.cost")
                } else {
                    command_label(left)
                };
                let rk = if right_rank.is_some() {
                    command_label("usage.cost")
                } else {
                    command_label(right)
                };
                lk.cmp(&rk)
            }
        }
    });
    commands
}

pub fn validate_keybinding_draft(command: &str, key: &str, when: &str) -> Result<(), String> {
    if command.trim().is_empty() {
        return Err("Select a command.".into());
    }
    if !is_valid_command(command) {
        return Err("Unknown keybinding command.".into());
    }
    let trimmed_key = key.trim();
    if trimmed_key.is_empty() {
        return Err("Press shortcut".into());
    }
    if trimmed_key.len() > MAX_KEYBINDING_VALUE_LENGTH {
        return Err(format!(
            "Keybinding must be {MAX_KEYBINDING_VALUE_LENGTH} characters or fewer."
        ));
    }
    if parse_keybinding_shortcut(trimmed_key).is_none() {
        return Err("Invalid keybinding.".into());
    }
    let trimmed_when = when.trim();
    if trimmed_when.len() > MAX_KEYBINDING_WHEN_LENGTH {
        return Err(format!(
            "When expression must be {MAX_KEYBINDING_WHEN_LENGTH} characters or fewer."
        ));
    }
    if !trimmed_when.is_empty() && parse_keybinding_when_expression(trimmed_when).is_none() {
        return Err("Use variables with !, &&, ||, and parentheses.".into());
    }
    Ok(())
}

pub fn upsert_custom_rule(
    custom: &mut Vec<KeybindingRule>,
    command: &str,
    key: &str,
    when: Option<&str>,
    replace: Option<(&str, &str, Option<&str>)>,
) -> Result<(), String> {
    validate_keybinding_draft(command, key, when.unwrap_or(""))?;
    let parsed = parse_keybinding_shortcut(key).ok_or("Invalid keybinding.")?;
    let normalized_key = shortcut_to_keybinding_input(&parsed);
    let normalized_when = when
        .map(str::trim)
        .filter(|w| !w.is_empty())
        .map(|w| {
            let ast = parse_keybinding_when_expression(w)
                .ok_or("Use variables with !, &&, ||, and parentheses.")?;
            Ok::<_, String>(when_ast_to_expression(Some(&ast)))
        })
        .transpose()?;
    let new_rule = KeybindingRule {
        key: normalized_key.clone(),
        command: command.to_owned(),
        when: normalized_when.clone(),
    };
    let replace_norm = replace.map(|(rc, rk, rw)| {
        let rk_norm = parse_keybinding_shortcut(rk)
            .map(|s| shortcut_to_keybinding_input(&s))
            .unwrap_or_else(|| rk.trim().to_owned());
        let rw_norm = rw
            .map(str::trim)
            .filter(|w| !w.is_empty())
            .and_then(|w| parse_keybinding_when_expression(w))
            .map(|ast| when_ast_to_expression(Some(&ast)));
        (rc.to_owned(), rk_norm, rw_norm)
    });

    custom.retain(|entry| {
        let entry_key = parse_keybinding_shortcut(&entry.key)
            .map(|s| shortcut_to_keybinding_input(&s))
            .unwrap_or_else(|| entry.key.trim().to_owned());
        let entry_when = entry
            .when
            .as_deref()
            .map(str::trim)
            .filter(|w| !w.is_empty())
            .and_then(parse_keybinding_when_expression)
            .map(|ast| when_ast_to_expression(Some(&ast)));
        let same_as_new = entry.command == new_rule.command
            && entry_key == new_rule.key
            && entry_when == new_rule.when;
        let same_as_replace = replace_norm.as_ref().is_some_and(|(rc, rk, rw)| {
            &entry.command == rc && &entry_key == rk && &entry_when == rw
        });
        !same_as_new && !same_as_replace
    });
    custom.push(new_rule);
    if custom.len() > MAX_KEYBINDINGS_COUNT {
        let start = custom.len() - MAX_KEYBINDINGS_COUNT;
        custom.drain(..start);
    }
    Ok(())
}

pub fn remove_custom_rule(
    custom: &mut Vec<KeybindingRule>,
    command: &str,
    key: &str,
    when: Option<&str>,
) {
    let target_key = parse_keybinding_shortcut(key)
        .map(|s| shortcut_to_keybinding_input(&s))
        .unwrap_or_else(|| key.trim().to_owned());
    let target_when = when
        .map(str::trim)
        .filter(|w| !w.is_empty())
        .and_then(parse_keybinding_when_expression)
        .map(|ast| when_ast_to_expression(Some(&ast)));
    custom.retain(|entry| {
        let entry_key = parse_keybinding_shortcut(&entry.key)
            .map(|s| shortcut_to_keybinding_input(&s))
            .unwrap_or_else(|| entry.key.trim().to_owned());
        let entry_when = entry
            .when
            .as_deref()
            .map(str::trim)
            .filter(|w| !w.is_empty())
            .and_then(parse_keybinding_when_expression)
            .map(|ast| when_ast_to_expression(Some(&ast)));
        !(entry.command == command && entry_key == target_key && entry_when == target_when)
    });
}

pub fn reset_custom_rule(
    custom: &mut Vec<KeybindingRule>,
    command: &str,
    key: &str,
    when: Option<&str>,
    default_key: Option<&str>,
    default_when: &str,
) -> Result<(), String> {
    remove_custom_rule(custom, command, key, when);
    let has_other_custom = custom.iter().any(|entry| entry.command == command);
    if has_other_custom && let Some(default_key) = default_key {
        let default_when_opt = (!default_when.trim().is_empty()).then_some(default_when);
        upsert_custom_rule(custom, command, default_key, default_when_opt, None)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rune::{Key, KeyEvent, Mods, PhysicalKeyCode};

    fn ev(key: Key, mods: Mods) -> KeyEvent {
        KeyEvent { key, mods }
    }

    #[test]
    fn parses_chords_modifiers_and_trailing_plus() {
        let plus = parse_keybinding_shortcut("mod++").unwrap();
        assert_eq!(plus.key, "+");
        assert!(plus.mod_key);

        let space = parse_keybinding_shortcut("Mod + Space").unwrap();
        assert_eq!(space.key, " ");
        assert!(space.mod_key);

        let esc = parse_keybinding_shortcut("ctrl+shift+esc").unwrap();
        assert_eq!(esc.key, "escape");
        assert!(esc.ctrl_key && esc.shift_key);

        assert!(parse_keybinding_shortcut("").is_none());
        assert!(parse_keybinding_shortcut("mod++").is_some());
        assert!(parse_keybinding_shortcut("mod+a+b").is_none());
    }

    #[test]
    fn platform_defaults_and_labels_match_mac_and_non_mac() {
        let bindings = default_resolved_keybindings();
        let ctx = ShortcutContext::desktop();
        assert_eq!(
            shortcut_label_for_command(&bindings, "sidebar.toggle", &ctx, Platform::Mac).as_deref(),
            Some("⌘B")
        );
        assert_eq!(
            shortcut_label_for_command(&bindings, "sidebar.toggle", &ctx, Platform::Linux)
                .as_deref(),
            Some("Ctrl+B")
        );
        assert_eq!(
            shortcut_label_for_command(&bindings, "modelPicker.toggle", &ctx, Platform::Mac)
                .as_deref(),
            Some("⇧⌘M")
        );
        assert_eq!(
            shortcut_label_for_command(&bindings, "modelPicker.toggle", &ctx, Platform::Windows)
                .as_deref(),
            Some("Ctrl+Shift+M")
        );
    }

    #[test]
    fn when_expression_parses_and_evaluates_precedence() {
        let ast =
            parse_keybinding_when_expression("!terminalFocus && (composerFocus || previewOpen)")
                .unwrap();
        let mut ctx = ShortcutContext::desktop();
        assert!(!evaluate_when_node(&ast, &ctx));
        ctx.composer_focus = true;
        assert!(evaluate_when_node(&ast, &ctx));
        ctx.terminal_focus = true;
        assert!(!evaluate_when_node(&ast, &ctx));
        assert!(parse_keybinding_when_expression("terminalFocus &&").is_none());
    }

    #[test]
    fn override_persistence_and_most_recent_wins() {
        let mut custom = Vec::new();
        upsert_custom_rule(&mut custom, "sidebar.toggle", "mod+shift+b", None, None).unwrap();
        let mut prefs = crate::ui::settings_ui::Preferences::default();
        prefs.keybindings = custom.clone();
        let json = serde_json::to_string(&prefs).unwrap();
        let restored: crate::ui::settings_ui::Preferences = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.keybindings, custom);

        let merged = merge_with_default_keybindings(&restored.keybindings);
        let ctx = ShortcutContext::desktop();
        let event = ev(
            Key::Command('b'),
            Mods {
                cmd: true,
                shift: true,
                ..Default::default()
            },
        );
        assert_eq!(
            resolve_shortcut_command(&event, &merged, &ctx, Platform::Mac, None, None),
            Some("sidebar.toggle")
        );
        let old_event = ev(
            Key::Command('b'),
            Mods {
                cmd: true,
                ..Default::default()
            },
        );
        assert_ne!(
            resolve_shortcut_command(&old_event, &merged, &ctx, Platform::Mac, None, None),
            Some("sidebar.toggle")
        );

        let mut custom_removed = restored.keybindings;
        remove_custom_rule(&mut custom_removed, "sidebar.toggle", "mod+shift+b", None);
        assert!(custom_removed.is_empty());
    }

    #[test]
    fn conflict_detection_resolves_mod_per_platform_and_respects_when() {
        let rules = compile_resolved_keybindings(&[
            rule("mod+k", "chat.new", None),
            rule("ctrl+k", "chat.newLocal", None),
            rule("meta+k", "thread.copyReference", None),
            rule("mod+k", "terminal.new", Some("terminalFocus")),
        ]);
        let mac_rows = build_keybinding_rows(&rules, Platform::Mac);
        let linux_rows = build_keybinding_rows(&rules, Platform::Linux);

        let mac_ctrl = mac_rows.iter().find(|r| r.key == "ctrl+k").unwrap();
        assert_eq!(mac_ctrl.conflicts, Vec::<String>::new());
        let mac_meta = mac_rows.iter().find(|r| r.key == "meta+k").unwrap();
        assert_eq!(mac_meta.conflicts, vec!["Chat: New", "Terminal: New"]);

        let linux_ctrl = linux_rows.iter().find(|r| r.key == "ctrl+k").unwrap();
        assert_eq!(linux_ctrl.conflicts, vec!["Chat: New", "Terminal: New"]);
        let linux_meta = linux_rows.iter().find(|r| r.key == "meta+k").unwrap();
        assert_eq!(linux_meta.conflicts, Vec::<String>::new());
    }

    #[test]
    fn matches_shifted_and_physical_keys() {
        let bindings = default_resolved_keybindings();
        let ctx = ShortcutContext::desktop();
        let prev = ev(
            Key::Command('{'),
            Mods {
                cmd: true,
                shift: true,
                ..Default::default()
            },
        );
        assert_eq!(
            resolve_shortcut_command(
                &prev,
                &bindings,
                &ctx,
                Platform::Mac,
                Some(PhysicalKeyCode::BracketLeft),
                Some('['),
            ),
            Some("thread.previous")
        );
    }

    #[test]
    fn validates_static_and_script_commands() {
        assert!(is_valid_command("modelPicker.toggle"));
        assert!(is_valid_command("script.build-app.run"));
        assert!(!is_valid_command("script..run"));
        assert!(!is_valid_command("script.Build.run"));
        assert!(!is_valid_command("unknown.command"));
    }

    #[test]
    fn global_shortcuts_preserve_existing_behavior_and_honour_overrides() {
        let mut state = crate::model::State::new(std::path::PathBuf::new(), false);
        state.populate_demo();
        let initial_sidebar = state.sidebar;
        let mut tx = rune::Tx::default();

        // Mod+B toggles sidebar.
        assert!(crate::key(
            &mut state,
            &ev(
                Key::Command('b'),
                Mods {
                    cmd: true,
                    ..Default::default()
                }
            ),
            &mut tx
        ));
        assert_eq!(state.sidebar, !initial_sidebar);

        // Mod+L focuses composer, Mod+K focuses search, Mod+Shift+P pins thread.
        state.current_mut().draft.focused = false;
        assert!(crate::key(
            &mut state,
            &ev(
                Key::Command('l'),
                Mods {
                    cmd: true,
                    ..Default::default()
                }
            ),
            &mut tx
        ));
        assert!(state.current().draft.focused);

        assert!(crate::key(
            &mut state,
            &ev(
                Key::Command('k'),
                Mods {
                    cmd: true,
                    ..Default::default()
                }
            ),
            &mut tx
        ));
        assert!(state.search.focused);
        state.search.focused = false;

        let was_pinned = state.current().pinned();
        assert!(crate::key(
            &mut state,
            &ev(
                Key::Command('p'),
                Mods {
                    cmd: true,
                    shift: true,
                    ..Default::default()
                }
            ),
            &mut tx
        ));
        assert_eq!(state.current().pinned(), !was_pinned);

        // Mod+= and Mod++ zoom in, Mod+- zooms out, Mod+0 resets zoom.
        assert!(crate::key(
            &mut state,
            &ev(
                Key::Command('='),
                Mods {
                    cmd: true,
                    ..Default::default()
                }
            ),
            &mut tx
        ));
        assert!(state.zoom > 1.0);
        assert!(crate::key(
            &mut state,
            &ev(
                Key::Command('+'),
                Mods {
                    cmd: true,
                    shift: true,
                    ..Default::default()
                }
            ),
            &mut tx
        ));
        assert!(crate::key(
            &mut state,
            &ev(
                Key::Command('0'),
                Mods {
                    cmd: true,
                    ..Default::default()
                }
            ),
            &mut tx
        ));
        assert!((state.zoom - 1.0).abs() < 0.001);

        // Escape while running stops even when composer is focused.
        let (pi, _cmds, _) = crate::backend::Pi::test_connection();
        state.current_mut().pi = Some(pi);
        state.current_mut().status = crate::model::Status::Running;
        state.current_mut().draft.focused = true;
        assert!(crate::key(
            &mut state,
            &ev(Key::Escape, Mods::default()),
            &mut tx
        ));

        // Rebinding thread.undo works through the registry.
        let id = state.current().id;
        state.pin_thread(id);
        let pinned_after_action = state.current().pinned();
        upsert_custom_rule(
            &mut state.preferences.keybindings,
            "thread.undo",
            "mod+shift+u",
            Some("!terminalFocus && !editableFocus"),
            None,
        )
        .unwrap();
        state.current_mut().draft.focused = false;
        assert!(!crate::key(
            &mut state,
            &ev(
                Key::Command('z'),
                Mods {
                    cmd: true,
                    ..Default::default()
                }
            ),
            &mut tx
        ));
        assert!(crate::key(
            &mut state,
            &ev(
                Key::Command('u'),
                Mods {
                    cmd: true,
                    shift: true,
                    ..Default::default()
                }
            ),
            &mut tx
        ));
        assert_eq!(state.current().pinned(), !pinned_after_action);
    }

    #[test]
    fn groups_filters_keycaps_and_resets_match_ksg_rules() {
        let plus = parse_keybinding_shortcut("mod++").unwrap();
        assert_eq!(keycap_parts(&plus, Platform::Mac), vec!["⌘", "+"]);
        assert_eq!(keycap_parts(&plus, Platform::Windows), vec!["Ctrl", "+"]);

        let model_toggle = parse_keybinding_shortcut("mod+shift+m").unwrap();
        assert_eq!(
            keycap_parts(&model_toggle, Platform::Mac),
            vec!["⇧", "⌘", "M"]
        );
        assert_eq!(
            keycap_parts(&model_toggle, Platform::Linux),
            vec!["Ctrl", "Shift", "M"]
        );

        let rows = build_keybinding_rows(&default_resolved_keybindings(), Platform::Mac);
        let groups = group_keybinding_rows(rows.clone());
        let titles: Vec<&str> = groups.iter().map(|g| g.title).collect();
        assert_eq!(
            titles,
            vec![
                "Navigation",
                "Threads",
                "Composer",
                "Terminal",
                "Preview & diff",
                "Appearance",
                "Other",
            ]
        );

        let filtered = filter_keybinding_rows(rows, "modelPicker.toggle");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].command, "modelPicker.toggle");

        let mut custom = Vec::new();
        upsert_custom_rule(
            &mut custom,
            "sidebar.toggle",
            "mod+k",
            None,
            Some(("sidebar.toggle", "mod+b", None)),
        )
        .unwrap();
        assert_eq!(custom.len(), 1);
        reset_custom_rule(
            &mut custom,
            "sidebar.toggle",
            "mod+k",
            None,
            Some("mod+b"),
            "",
        )
        .unwrap();
        assert!(custom.is_empty());
    }

    #[test]
    fn rich_text_bold_shortcut_follows_layout_and_selection_rules() {
        // Non-Latin layout ('и' at physical KeyB) matches when selected.
        let cyrillic_b = ev(
            Key::Text("и".into()),
            Mods {
                ctrl: true,
                ..Default::default()
            },
        );
        assert!(is_rich_text_bold_shortcut(
            &cyrillic_b,
            Some(PhysicalKeyCode::KeyB),
            None,
            true
        ));
        assert!(!is_rich_text_bold_shortcut(
            &cyrillic_b,
            Some(PhysicalKeyCode::KeyB),
            None,
            false
        ));

        // Latin layout letter 'x' at physical KeyB does NOT match 'b'.
        let x_on_b = ev(
            Key::Command('x'),
            Mods {
                cmd: true,
                ..Default::default()
            },
        );
        assert!(!is_rich_text_bold_shortcut(
            &x_on_b,
            Some(PhysicalKeyCode::KeyB),
            None,
            true
        ));

        // Latin layout letter 'b' at physical KeyN DOES match 'b'.
        let b_on_n = ev(
            Key::Command('b'),
            Mods {
                cmd: true,
                ..Default::default()
            },
        );
        assert!(is_rich_text_bold_shortcut(
            &b_on_n,
            Some(PhysicalKeyCode::KeyN),
            None,
            true
        ));
        assert!(!is_rich_text_bold_shortcut(
            &b_on_n,
            Some(PhysicalKeyCode::KeyN),
            None,
            false
        ));

        // Extra shift or alt prevents bold shortcut.
        let mod_shift_b = ev(
            Key::Command('b'),
            Mods {
                cmd: true,
                shift: true,
                ..Default::default()
            },
        );
        assert!(!is_rich_text_bold_shortcut(
            &mod_shift_b,
            Some(PhysicalKeyCode::KeyB),
            None,
            true
        ));
    }
}
