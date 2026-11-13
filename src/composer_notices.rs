//! General notice stack above the composer, ported from KSG Code's `ComposerBannerStack`,
//! `ComposerBanner`, and the connection-recovery parts of `useSystemComposerBanners`.

use crate::model::State;
use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    Default,
    Error,
    Info,
    Success,
    Warning,
}

/// Where a notice sits in the stack. `Environment` notices always come before `Attention`
/// notices, and within each group notices remain in insertion order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Group {
    Environment,
    #[default]
    Attention,
}

#[derive(Clone)]
pub struct ActionHandler(Arc<dyn Fn(&mut State) + Send + Sync>);

#[allow(dead_code)]
impl ActionHandler {
    pub fn new(handler: impl Fn(&mut State) + Send + Sync + 'static) -> Self {
        Self(Arc::new(handler))
    }

    pub fn call(&self, state: &mut State) {
        (self.0)(state);
    }
}

impl fmt::Debug for ActionHandler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ActionHandler(..)")
    }
}

#[derive(Clone, Debug)]
pub struct NoticeAction {
    pub label: String,
    pub tooltip: Option<String>,
    pub disabled: bool,
    pub handler: ActionHandler,
}

#[allow(dead_code)]
impl NoticeAction {
    pub fn new(
        label: impl Into<String>,
        handler: impl Fn(&mut State) + Send + Sync + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            tooltip: None,
            disabled: false,
            handler: ActionHandler::new(handler),
        }
    }

    pub fn with_tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

#[derive(Clone, Debug)]
pub struct Notice {
    pub id: String,
    pub variant: Variant,
    pub group: Group,
    pub icon: &'static str,
    pub title: String,
    pub description: Option<String>,
    pub actions: Vec<NoticeAction>,
    pub dismiss_label: Option<String>,
}

#[allow(dead_code)]
impl Notice {
    pub fn new(
        id: impl Into<String>,
        variant: Variant,
        icon: &'static str,
        title: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            variant,
            group: Group::default(),
            icon,
            title: title.into(),
            description: None,
            actions: Vec::new(),
            dismiss_label: None,
        }
    }

    pub fn group(mut self, group: Group) -> Self {
        self.group = group;
        self
    }

    pub fn description(mut self, description: Option<String>) -> Self {
        self.description = description;
        self
    }

    pub fn actions(mut self, actions: Vec<NoticeAction>) -> Self {
        self.actions = actions;
        self
    }

    pub fn dismiss_label(mut self, label: impl Into<String>) -> Self {
        self.dismiss_label = Some(label.into());
        self
    }
}

#[derive(Default)]
pub struct Store {
    items: Vec<Notice>,
    dismissed: HashSet<String>,
    details_open: Option<String>,
}

#[allow(dead_code)]
impl Store {
    /// Insert a new notice, or update an existing notice with the same `id` in place so its
    /// position in its group never shifts.
    pub fn push(&mut self, notice: Notice) {
        if let Some(existing) = self.items.iter_mut().find(|item| item.id == notice.id) {
            *existing = notice;
        } else {
            self.items.push(notice);
        }
    }

    /// Remove a notice by `id` and clear its dismissed/details state so a later occurrence of
    /// the condition shows the notice again.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.items.len();
        self.items.retain(|item| item.id != id);
        self.dismissed.remove(id);
        if self.details_open.as_deref() == Some(id) {
            self.details_open = None;
        }
        self.items.len() != before
    }

    /// Synchronize one notice slot by `id`. `None` removes the notice and clears dismissal.
    pub fn sync(&mut self, id: &str, notice: Option<Notice>) -> bool {
        match notice {
            Some(notice) => {
                if let Some(existing) = self.items.iter_mut().find(|item| item.id == id) {
                    let changed = existing.variant != notice.variant
                        || existing.group != notice.group
                        || existing.icon != notice.icon
                        || existing.title != notice.title
                        || existing.description != notice.description
                        || existing.dismiss_label != notice.dismiss_label
                        || existing.actions.len() != notice.actions.len()
                        || existing.actions.iter().zip(&notice.actions).any(|(a, b)| {
                            a.label != b.label || a.tooltip != b.tooltip || a.disabled != b.disabled
                        });
                    *existing = notice;
                    changed
                } else {
                    self.items.push(notice);
                    true
                }
            }
            None => self.remove(id),
        }
    }

    pub fn dismiss(&mut self, id: &str) {
        if self.items.iter().any(|item| item.id == id) {
            self.dismissed.insert(id.to_owned());
        }
        if self.details_open.as_deref() == Some(id) {
            self.details_open = None;
        }
    }

    pub fn is_dismissed(&self, id: &str) -> bool {
        self.dismissed.contains(id)
    }

    pub fn toggle_details(&mut self, id: &str) {
        if self.details_open.as_deref() == Some(id) {
            self.details_open = None;
        } else if self.items.iter().any(|item| item.id == id) {
            self.details_open = Some(id.to_owned());
        }
    }

    pub fn close_details(&mut self) {
        self.details_open = None;
    }

    pub fn open_details_notice(&self) -> Option<&Notice> {
        let id = self.details_open.as_deref()?;
        if self.dismissed.contains(id) {
            return None;
        }
        self.items.iter().find(|item| item.id == id)
    }

    /// All visible notices in KSG order: `Environment` first, then `Attention`, preserving
    /// insertion order within each group.
    pub fn ordered(&self) -> Vec<&Notice> {
        const GROUPS: [Group; 2] = [Group::Environment, Group::Attention];
        GROUPS
            .into_iter()
            .flat_map(|group| {
                self.items
                    .iter()
                    .filter(move |item| item.group == group && !self.dismissed.contains(&item.id))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_insertion_order_renders_environment_before_attention_and_keeps_positions() {
        let mut store = Store::default();
        store.push(Notice::new(
            "att-1",
            Variant::Info,
            "alert",
            "First attention",
        ));
        store.push(
            Notice::new("env-1", Variant::Warning, "alert", "First env").group(Group::Environment),
        );
        store.push(Notice::new(
            "att-2",
            Variant::Default,
            "alert",
            "Second attention",
        ));
        store.push(
            Notice::new("env-2", Variant::Error, "alert", "Second env").group(Group::Environment),
        );

        let ids = |store: &Store| -> Vec<String> {
            store.ordered().into_iter().map(|n| n.id.clone()).collect()
        };
        assert_eq!(ids(&store), vec!["env-1", "env-2", "att-1", "att-2"]);

        // Updating in place preserves position.
        store.push(
            Notice::new("env-1", Variant::Error, "alert", "Updated env").group(Group::Environment),
        );
        assert_eq!(ids(&store), vec!["env-1", "env-2", "att-1", "att-2"]);

        // Removing one keeps the remaining positions intact.
        assert!(store.remove("env-1"));
        assert_eq!(ids(&store), vec!["env-2", "att-1", "att-2"]);

        // Dismissing hides until the condition ends, then re-raising shows it again.
        store.dismiss("att-1");
        assert!(store.is_dismissed("att-1"));
        assert_eq!(ids(&store), vec!["env-2", "att-2"]);
        store.remove("att-1");
        assert!(!store.is_dismissed("att-1"));
        store.push(Notice::new("att-1", Variant::Info, "alert", "Raised again"));
        assert_eq!(ids(&store), vec!["env-2", "att-2", "att-1"]);
    }
}
