//! General notice stack above the composer, ported from KSG Code's `ComposerBannerStack`,
//! `ComposerBanner`, and the connection-recovery parts of `useSystemComposerBanners`.

use crate::client::{ConnectionPhase, ConnectionState, FailureKind};
use crate::model::State;
use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Ported from KSG Code's `ENVIRONMENT_RECONNECT_WARNING_GRACE_MS` in `ChatView.logic.ts`.
pub const ENVIRONMENT_RECONNECT_WARNING_GRACE_MS: u64 = 2_000;
/// Ported from KSG Code's `DISCONNECT_DELAY_MS` in `useEnvironmentDisconnectDelay.ts`.
pub const DISCONNECT_DELAY_MS: u64 = 20_000;

/// Ported from KSG Code's `presentConnectionDiagnosis` (`locate` + `headline`) in
/// `packages/client-runtime/src/connection/diagnosis.ts` for direct connections.
pub fn connection_diagnosis(host_label: &str, state: &ConnectionState) -> Option<String> {
    if state.phase == ConnectionPhase::Available {
        return None;
    }
    if state.phase == ConnectionPhase::Offline {
        return Some("This device is offline".into());
    }
    match state.last_failure {
        Some(FailureKind::Outdated) => Some(format!("{host_label} runs an older Shina")),
        Some(FailureKind::AuthDenied) => {
            Some(format!("Shina on {host_label} did not accept this device"))
        }
        Some(FailureKind::Timeout) => Some(format!("{host_label} did not answer in time")),
        Some(FailureKind::Dns) => Some(format!("{host_label} address did not resolve")),
        Some(FailureKind::InvalidResponse) => {
            Some(format!("Shina on {host_label} sent an unexpected answer"))
        }
        Some(FailureKind::Refused | FailureKind::ServerNotAnswering) => {
            Some(format!("Shina on {host_label} did not answer"))
        }
        None if state.phase == ConnectionPhase::Error => {
            Some(format!("Shina on {host_label} did not answer"))
        }
        None => None,
    }
}

#[derive(Clone)]
pub struct EnvironmentStatus {
    pub server_id: String,
    pub label: String,
    pub connection: ConnectionState,
    pub can_disconnect: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ReconnectGrace {
    key: (String, u64),
    started_at: Instant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DisconnectHold {
    server_id: String,
    started_at: Instant,
}

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

#[derive(Clone, Debug, PartialEq)]
pub enum NoticeBody {
    UsageLimits(crate::usage_limits::UsageLimitsReport),
}

#[derive(Clone, Debug)]
pub struct Notice {
    pub id: String,
    pub variant: Variant,
    pub group: Group,
    pub icon: &'static str,
    pub title: String,
    pub description: Option<String>,
    pub body: Option<NoticeBody>,
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
            body: None,
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

    pub fn body(mut self, body: NoticeBody) -> Self {
        self.body = Some(body);
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
    grace: Option<ReconnectGrace>,
    disconnect_hold: Option<DisconnectHold>,
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
                        || existing.body != notice.body
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

    /// Synchronize the `Environment` connection-recovery notice for the open thread's machine,
    /// returning `(changed, next_wake_delay)`.
    pub fn sync_environment(
        &mut self,
        active: Option<EnvironmentStatus>,
        now: Instant,
    ) -> (bool, Option<Duration>) {
        const PREFIX: &str = "environment-unavailable:";
        let expected_id = active
            .as_ref()
            .map(|env| format!("{PREFIX}{}", env.server_id));
        let stale: Vec<String> = self
            .items
            .iter()
            .filter(|item| {
                item.id.starts_with(PREFIX) && expected_id.as_deref() != Some(item.id.as_str())
            })
            .map(|item| item.id.clone())
            .collect();
        let mut changed = false;
        for id in stale {
            changed |= self.remove(&id);
        }

        let Some(env) = active else {
            self.grace = None;
            self.disconnect_hold = None;
            return (changed, None);
        };
        let notice_id = format!("{PREFIX}{}", env.server_id);
        let (disconnect_elapsed, disconnect_wake) = if env.can_disconnect {
            let started_at = match &self.disconnect_hold {
                Some(existing) if existing.server_id == env.server_id => existing.started_at,
                _ => {
                    self.disconnect_hold = Some(DisconnectHold {
                        server_id: env.server_id.clone(),
                        started_at: now,
                    });
                    now
                }
            };
            let delay = Duration::from_millis(DISCONNECT_DELAY_MS);
            let held = now.saturating_duration_since(started_at);
            if held >= delay {
                (true, None)
            } else {
                (false, Some(delay - held))
            }
        } else {
            self.disconnect_hold = None;
            (false, None)
        };
        let reconnecting = matches!(
            env.connection.phase,
            ConnectionPhase::Connecting | ConnectionPhase::Reconnecting
        );
        if reconnecting {
            let key = (env.server_id.clone(), env.connection.attempt_seq);
            let started_at = match &self.grace {
                Some(existing) if existing.key == key => existing.started_at,
                _ => {
                    self.grace = Some(ReconnectGrace {
                        key,
                        started_at: now,
                    });
                    now
                }
            };
            let grace = Duration::from_millis(ENVIRONMENT_RECONNECT_WARNING_GRACE_MS);
            let elapsed = now.saturating_duration_since(started_at);
            if elapsed < grace {
                changed |= self.remove(&notice_id);
                let wake = min_wake(Some(grace - elapsed), disconnect_wake);
                return (changed, wake);
            }
        } else {
            self.grace = None;
        }

        let variant = if env.connection.phase == ConnectionPhase::Error {
            Variant::Error
        } else {
            Variant::Warning
        };
        let title = format!(
            "{} is {}",
            env.label,
            if reconnecting {
                "reconnecting"
            } else {
                "offline"
            }
        );
        let description = connection_diagnosis(&env.label, &env.connection);
        let mut actions = Vec::new();
        if env.connection.phase != ConnectionPhase::Connecting {
            let server = env.server_id.clone();
            actions.push(NoticeAction::new("Retry", move |s| {
                s.retry_machine(&server);
            }));
        }
        if disconnect_elapsed && env.can_disconnect {
            let server = env.server_id;
            actions.push(
                NoticeAction::new("Disconnect server", move |s| {
                    s.disconnect_server(&server);
                })
                .with_tooltip("Hide this server's threads. Switch it on again in Environments."),
            );
        }
        let notice = Notice::new(notice_id.clone(), variant, "alert", title)
            .group(Group::Environment)
            .description(description)
            .actions(actions);
        changed |= self.sync(&notice_id, Some(notice));
        (changed, disconnect_wake)
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

fn min_wake(a: Option<Duration>, b: Option<Duration>) -> Option<Duration> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) | (None, Some(a)) => Some(a),
        (None, None) => None,
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

    #[test]
    fn grace_period_delays_reconnecting_notice_and_restarts_on_new_attempt() {
        let mut store = Store::default();
        let t0 = Instant::now();
        let env = |attempt_seq: u64| EnvironmentStatus {
            server_id: "studio".into(),
            label: "Studio".into(),
            connection: ConnectionState {
                phase: ConnectionPhase::Reconnecting,
                attempt: attempt_seq as u32,
                attempt_seq,
                last_failure: Some(FailureKind::ServerNotAnswering),
                last_error: Some("Server listener is unavailable.".into()),
            },
            can_disconnect: true,
        };

        let (_, wake) = store.sync_environment(Some(env(1)), t0);
        assert!(store.ordered().is_empty());
        assert_eq!(
            wake,
            Some(Duration::from_millis(
                ENVIRONMENT_RECONNECT_WARNING_GRACE_MS
            ))
        );

        let (_, wake) = store.sync_environment(Some(env(1)), t0 + Duration::from_millis(1_999));
        assert!(store.ordered().is_empty());
        assert_eq!(wake, Some(Duration::from_millis(1)));

        let (_, wake) = store.sync_environment(
            Some(env(1)),
            t0 + Duration::from_millis(ENVIRONMENT_RECONNECT_WARNING_GRACE_MS),
        );
        assert_eq!(
            wake,
            Some(Duration::from_millis(
                DISCONNECT_DELAY_MS - ENVIRONMENT_RECONNECT_WARNING_GRACE_MS
            ))
        );
        let notices = store.ordered();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].title, "Studio is reconnecting");
        assert_eq!(
            notices[0].description.as_deref(),
            Some("Shina on Studio did not answer")
        );

        // A second reconnect attempt restarts the grace period.
        let t_retry = t0 + Duration::from_millis(3_000);
        let (_, wake) = store.sync_environment(Some(env(2)), t_retry);
        assert!(store.ordered().is_empty());
        assert_eq!(
            wake,
            Some(Duration::from_millis(
                ENVIRONMENT_RECONNECT_WARNING_GRACE_MS
            ))
        );

        store.sync_environment(
            Some(env(2)),
            t_retry + Duration::from_millis(ENVIRONMENT_RECONNECT_WARNING_GRACE_MS),
        );
        assert_eq!(store.ordered().len(), 1);
    }

    #[test]
    fn disconnect_delay_gates_action_and_never_shows_for_local() {
        let mut store = Store::default();
        let t0 = Instant::now();
        let offline_remote = |id: &str| EnvironmentStatus {
            server_id: id.into(),
            label: "Studio".into(),
            connection: ConnectionState {
                phase: ConnectionPhase::Offline,
                ..Default::default()
            },
            can_disconnect: true,
        };

        let (_, wake) = store.sync_environment(Some(offline_remote("studio")), t0);
        assert_eq!(wake, Some(Duration::from_millis(DISCONNECT_DELAY_MS)));
        assert_eq!(
            store.ordered()[0]
                .actions
                .iter()
                .map(|a| a.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Retry"]
        );

        store.sync_environment(
            Some(offline_remote("studio")),
            t0 + Duration::from_millis(DISCONNECT_DELAY_MS - 1),
        );
        assert_eq!(store.ordered()[0].actions.len(), 1);

        let (_, wake) = store.sync_environment(
            Some(offline_remote("studio")),
            t0 + Duration::from_millis(DISCONNECT_DELAY_MS),
        );
        assert_eq!(wake, None);
        assert_eq!(
            store.ordered()[0]
                .actions
                .iter()
                .map(|a| a.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Retry", "Disconnect server"]
        );

        // Local machine never offers "Disconnect server", even after the delay.
        let local = EnvironmentStatus {
            server_id: "local".into(),
            label: "Local".into(),
            connection: ConnectionState {
                phase: ConnectionPhase::Offline,
                ..Default::default()
            },
            can_disconnect: false,
        };
        store.sync_environment(Some(local.clone()), t0);
        store.sync_environment(
            Some(local),
            t0 + Duration::from_millis(DISCONNECT_DELAY_MS * 2),
        );
        assert_eq!(
            store.ordered()[0]
                .actions
                .iter()
                .map(|a| a.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Retry"]
        );
    }
}
