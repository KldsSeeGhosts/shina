//! Local equivalents of KSG Code's threadSort, threadSettled, threadInbox and threadUndo.
//! Ported from T3 Code, copyright 2026 T3 Tools Inc. MIT license in docs/t3-code-license.txt.
//! All persisted lifecycle times are Unix milliseconds. Legacy `updated` remains seconds.
use chrono::{DateTime, Datelike, Days, Local, NaiveDateTime, TimeZone, Timelike};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Settled {
    Settled,
    Active,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Lifecycle {
    pub created_at: u64,
    pub updated_at: u64,
    pub pinned_at: Option<u64>,
    pub pin_order_key: Option<String>,
    pub archived_at: Option<u64>,
    pub settled_override: Option<Settled>,
    pub settled_at: Option<u64>,
    pub unsettled_at: Option<u64>,
    pub snoozed_until: Option<u64>,
    pub snoozed_at: Option<u64>,
    pub last_snooze_wake_at: Option<u64>,
    pub last_visited_at: Option<u64>,
    pub last_completed_at: Option<u64>,
    pub last_requested_at: Option<u64>,
    pub last_started_at: Option<u64>,
    pub latest_user_message_at: Option<u64>,
    pub failed_at: Option<u64>,
    pub auto_settle_disabled_at: Option<u64>,
    /// A future PR integration can populate this and call the run-end sweep hook.
    pub pull_request_finished_at: Option<u64>,
}

impl Lifecycle {
    pub fn new(at: u64) -> Self {
        Self {
            created_at: at,
            updated_at: at,
            ..Self::default()
        }
    }
    pub fn from_record(record: &serde_json::Value) -> Result<Self, String> {
        if let Some(value) = record.get("lifecycle") {
            return serde_json::from_value(value.clone())
                .map_err(|e| format!("Invalid thread lifecycle: {e}"));
        }
        for field in ["pinned", "archived", "unread"] {
            if record
                .get(field)
                .is_some_and(|v| !v.is_null() && !v.is_boolean())
            {
                return Err(format!("Invalid legacy {field} value."));
            }
        }
        let at = record["updated"]
            .as_u64()
            .unwrap_or(0)
            .saturating_mul(1000)
            .max(1);
        Ok(Self {
            pinned_at: (record["pinned"] == true).then_some(at),
            archived_at: (record["archived"] == true).then_some(at),
            // A synthetic completion preserves the legacy unread bit without inventing a run.
            last_completed_at: (record["unread"] == true).then_some(at),
            last_visited_at: Some(if record["unread"] == true { at - 1 } else { at }),
            ..Self::new(at)
        })
    }
    pub fn unread(&self) -> bool {
        self.last_completed_at
            .is_some_and(|at| self.last_visited_at.is_some_and(|seen| at > seen))
    }
    pub fn mark_unread(&mut self) -> bool {
        let Some(at) = self.last_completed_at else {
            return false;
        };
        self.last_visited_at = Some(at.saturating_sub(1));
        true
    }
    pub fn settled(&self) -> bool {
        self.settled_override == Some(Settled::Settled)
    }
    pub fn snoozed(&self, now: u64) -> bool {
        self.snoozed_until.is_some_and(|at| at > now)
            && self
                .last_snooze_wake_at
                .is_none_or(|at| self.snoozed_at.is_some_and(|set| at < set))
    }
    pub fn woke_at(&self, now: u64) -> Option<u64> {
        self.last_snooze_wake_at
            .or(self.snoozed_until.filter(|at| *at <= now))
    }
    pub fn woke(&self, now: u64) -> bool {
        !self.settled()
            && self
                .woke_at(now)
                .is_some_and(|at| at > self.last_visited_at.unwrap_or(0))
    }
    pub fn settle(&mut self, at: u64) {
        self.settled_override = Some(Settled::Settled);
        self.settled_at.get_or_insert(at);
        self.unsettled_at = None;
        self.pinned_at = None;
        self.pin_order_key = None;
        self.snoozed_until = None;
        self.snoozed_at = None;
        self.last_visited_at = Some(at.max(self.last_visited_at.unwrap_or(0)));
    }
    pub fn unsettle(&mut self, at: u64, user: bool) {
        if self.settled() {
            self.unsettled_at = Some(at);
        }
        self.settled_at = None;
        self.settled_override = user.then_some(Settled::Active);
    }
    pub fn pin(&mut self, at: u64, key: Option<String>) {
        self.unsettle(at, false);
        self.pinned_at = Some(at);
        self.pin_order_key = key;
        self.snoozed_until = None;
        self.snoozed_at = None;
    }
    pub fn snooze(&mut self, at: u64, until: u64) {
        self.snoozed_at = Some(at);
        self.snoozed_until = Some(until);
        self.last_snooze_wake_at = None;
        self.last_visited_at = Some(at);
    }
    pub fn unsnooze(&mut self, at: u64) {
        self.snoozed_until = None;
        self.snoozed_at = None;
        self.last_snooze_wake_at = Some(at);
    }
    pub fn user_sent(&mut self, at: u64) {
        self.unsettle(at, false);
        self.snoozed_until = None;
        self.snoozed_at = None;
        self.latest_user_message_at = Some(at);
        self.last_requested_at = Some(at);
        if self.last_visited_at.is_none() {
            self.last_visited_at = Some(at.saturating_sub(1));
        }
    }
    pub fn activity(&self) -> u64 {
        [
            Some(self.created_at),
            self.latest_user_message_at,
            self.last_requested_at,
            self.last_started_at,
            self.last_completed_at,
            self.last_snooze_wake_at,
        ]
        .into_iter()
        .flatten()
        .max()
        .unwrap_or(0)
    }
    pub fn auto_settle_at(&self, at: u64, days: u32, busy: bool, captain: bool) -> Option<u64> {
        if captain
            || busy
            || self.archived_at.is_some()
            || self.pinned_at.is_some()
            || self.settled_override.is_some()
            || self.auto_settle_disabled_at.is_some()
            || self.snoozed(at)
        {
            return None;
        }
        let activity = self
            .activity()
            .max(self.snoozed_until.filter(|t| *t <= at).unwrap_or(0));
        if self.pr_settlement_ready(at) {
            return Some(activity);
        }
        (days > 0 && at.saturating_sub(activity) >= u64::from(days) * 86_400_000)
            .then_some(activity)
    }
    /// Future PR tracking must set the field only after all relevant links have
    /// snapshots and none remains open. This hook does not fetch or watch PRs.
    pub fn pr_settlement_ready(&self, at: u64) -> bool {
        self.pull_request_finished_at.is_some_and(|finished| {
            finished <= at
                && finished
                    >= self
                        .created_at
                        .max(self.latest_user_message_at.unwrap_or(0))
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Section {
    Pinned,
    Active,
    Snoozed,
    Settled,
}
pub fn section(life: &Lifecycle, now: u64) -> Section {
    if life.snoozed(now) {
        Section::Snoozed
    } else if life.settled() {
        Section::Settled
    } else if life.pinned_at.is_some() {
        Section::Pinned
    } else {
        Section::Active
    }
}

pub fn compare(section: Section, a: &Lifecycle, b: &Lifecycle) -> Ordering {
    let newest = |a: u64, b: u64| b.cmp(&a);
    match section {
        Section::Pinned => match (&a.pin_order_key, &b.pin_order_key) {
            (Some(a), Some(b)) => a.cmp(b),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            _ => newest(a.created_at, b.created_at),
        },
        Section::Active => {
            let stamp = |l: &Lifecycle| {
                [
                    l.created_at,
                    l.unsettled_at.unwrap_or(0),
                    l.last_requested_at.unwrap_or(0),
                    l.last_completed_at.unwrap_or(0),
                ]
                .into_iter()
                .max()
                .unwrap()
            };
            newest(stamp(a), stamp(b))
        }
        Section::Snoozed => a.snoozed_until.cmp(&b.snoozed_until),
        Section::Settled => {
            let stamp = |l: &Lifecycle| {
                l.settled_at.unwrap_or_else(|| {
                    [
                        l.latest_user_message_at,
                        l.last_requested_at,
                        l.last_started_at,
                        l.last_completed_at,
                    ]
                    .into_iter()
                    .flatten()
                    .max()
                    .unwrap_or(l.updated_at)
                })
            };
            newest(stamp(a), stamp(b))
        }
    }
}

/// Rune supplies pointer drags on Windows. The timed Snoozed shelf rejects drops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropPlan {
    Pin,
    ReorderPin,
    Unpin,
    Unsettle,
    Wake,
    Settle,
    None,
}
pub fn plan_drop(from: Section, to: Section) -> DropPlan {
    use Section::*;
    match (from, to) {
        (_, Snoozed) => DropPlan::None,
        (Pinned, Pinned) => DropPlan::ReorderPin,
        (_, Pinned) => DropPlan::Pin,
        (Pinned, Active) => DropPlan::Unpin,
        (Settled, Active) => DropPlan::Unsettle,
        (Snoozed, Active) => DropPlan::Wake,
        (f, Settled) if f != Settled => DropPlan::Settle,
        _ => DropPlan::None,
    }
}

pub fn pin_key_between(before: Option<&str>, after: Option<&str>) -> Option<String> {
    fn valid(k: &str) -> bool {
        !k.is_empty() && !k.ends_with('a') && k.bytes().all(|c| c.is_ascii_lowercase())
    }
    fn midpoint(a: &[u8], b: &[u8]) -> Vec<u8> {
        if !b.is_empty() {
            let n = b
                .iter()
                .enumerate()
                .take_while(|(i, c)| a.get(*i).copied().unwrap_or(b'a') == **c)
                .count();
            if n > 0 {
                let mut key = b[..n].to_vec();
                key.extend(midpoint(a.get(n..).unwrap_or_default(), &b[n..]));
                return key;
            }
        }
        let da = a.first().map(|c| c - b'a').unwrap_or(0);
        let db = b.first().map(|c| c - b'a').unwrap_or(26);
        if db - da > 1 {
            return vec![b'a' + (da + db).div_ceil(2)];
        }
        if b.len() > 1 {
            return vec![b[0]];
        }
        let mut key = vec![b'a' + da];
        key.extend(midpoint(a.get(1..).unwrap_or_default(), &[]));
        key
    }
    if before.is_some_and(|s| !valid(s))
        || after.is_some_and(|s| !valid(s))
        || after.is_some_and(|b| before.is_some_and(|a| a >= b))
    {
        return None;
    }
    String::from_utf8(midpoint(
        before.unwrap_or("").as_bytes(),
        after.unwrap_or("").as_bytes(),
    ))
    .ok()
}
pub fn spread_pin_keys(count: usize) -> Vec<String> {
    let mut width = 2;
    let mut space = 26usize.pow(width);
    while space <= (count + 1) * 2 {
        width += 1;
        space *= 26;
    }
    (0..count)
        .map(|i| {
            let mut value = (space as f64 * (i + 1) as f64 / (count + 1) as f64).round() as usize;
            if value.is_multiple_of(26) {
                value += 1;
            }
            let mut key = vec![b'a'; width as usize];
            for c in key.iter_mut().rev() {
                *c += (value % 26) as u8;
                value /= 26;
            }
            String::from_utf8(key).unwrap()
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    Settle,
    Snooze,
    Unpin,
    Archive,
    Discard,
}
impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::Settle => "Settled",
            Self::Snooze => "Snoozed",
            Self::Unpin => "Unpinned",
            Self::Archive => "Archived",
            Self::Discard => "Draft discarded",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UndoStep {
    Unsettle,
    Unsnooze,
    Pin(Option<String>),
    Snooze(u64),
    Unarchive,
    RestoreDraft(String),
    RestoreImages(Vec<crate::attachments::Attachment>),
}
pub fn plan_undo(action: Action, old: &Lifecycle, draft: &str) -> Vec<UndoStep> {
    let mut steps = match action {
        Action::Snooze => return vec![UndoStep::Unsnooze],
        Action::Archive => return vec![UndoStep::Unarchive],
        Action::Discard => return vec![UndoStep::RestoreDraft(draft.into())],
        Action::Settle => vec![UndoStep::Unsettle],
        Action::Unpin => vec![],
    };
    if action == Action::Unpin || old.pinned_at.is_some() {
        steps.push(UndoStep::Pin(old.pin_order_key.clone()));
    }
    if let Some(at) = old.snoozed_until {
        steps.push(UndoStep::Snooze(at));
    }
    steps
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UndoEntry {
    pub id: usize,
    pub steps: Vec<UndoStep>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UndoNotice {
    pub action: Action,
    pub entries: Vec<UndoEntry>,
    pub expires: u64,
    pub owns_text_key: bool,
}
impl UndoNotice {
    pub fn push(slot: &mut Option<Self>, action: Action, id: usize, steps: Vec<UndoStep>, at: u64) {
        let same = slot
            .as_ref()
            .is_some_and(|s| s.action == action && s.expires > at);
        if !same {
            *slot = Some(Self {
                action,
                entries: vec![],
                expires: at + 5000,
                owns_text_key: true,
            });
        }
        let notice = slot.as_mut().unwrap();
        notice.entries.retain(|e| e.id != id);
        notice.entries.push(UndoEntry { id, steps });
        notice.expires = at + 5000;
        notice.owns_text_key = true;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Preset {
    After(u32),
    At { day: Day, hour: u32, minute: u32 },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Day {
    Today,
    Tomorrow,
    Weekday(u32),
} // Monday = 0.
#[derive(Clone, Debug)]
pub struct SnoozeChoice {
    pub preset: Preset,
    pub label: String,
    pub time: String,
    pub until: u64,
}
pub fn resolve_preset<T: TimeZone>(preset: Preset, now: DateTime<T>) -> Option<SnoozeChoice> {
    let zone = now.timezone();
    let (wake, label, names_day) = match preset {
        Preset::After(minutes) => {
            if !(1..=43200).contains(&minutes) {
                return None;
            }
            let (n, unit) = if minutes % 1440 == 0 {
                (minutes / 1440, "day")
            } else if minutes % 60 == 0 {
                (minutes / 60, "hour")
            } else {
                (minutes, "minute")
            };
            (
                now.clone() + chrono::Duration::minutes(minutes.into()),
                format!("In {n} {unit}{}", if n == 1 { "" } else { "s" }),
                false,
            )
        }
        Preset::At { day, hour, minute } => {
            let days = match day {
                Day::Today => 0,
                Day::Tomorrow => 1,
                Day::Weekday(d) if d < 7 => {
                    let n = (d + 7 - now.weekday().num_days_from_monday()) % 7;
                    if n == 0 { 7 } else { n }
                }
                _ => return None,
            };
            let date = now.date_naive().checked_add_days(Days::new(days.into()))?;
            let naive = date.and_hms_opt(hour, minute, 0)?;
            // Pick the first occurrence at a fall-back fold. Reject spring gaps.
            let wake = zone.from_local_datetime(&naive).earliest()?;
            let label = match days {
                0 => {
                    if hour < 12 {
                        "This morning".into()
                    } else if hour < 17 {
                        "This afternoon".into()
                    } else {
                        "This evening".into()
                    }
                }
                1 => "Tomorrow".into(),
                _ if day == Day::Weekday(0) => "Next week".into(),
                _ => format!(
                    "{}{}",
                    if days == 7 { "Next " } else { "" },
                    [
                        "Monday",
                        "Tuesday",
                        "Wednesday",
                        "Thursday",
                        "Friday",
                        "Saturday",
                        "Sunday"
                    ][wake.weekday().num_days_from_monday() as usize]
                ),
            };
            let names = label != "Next week";
            (wake, label, names)
        }
    };
    if wake <= now {
        return None;
    }
    let time = if names_day || wake.date_naive() == now.date_naive() {
        format!("{:02}:{:02}", wake.hour(), wake.minute())
    } else {
        format!("{} {:02}:{:02}", wake.weekday(), wake.hour(), wake.minute())
    };
    Some(SnoozeChoice {
        preset,
        label,
        time,
        until: wake.timestamp_millis().try_into().ok()?,
    })
}
pub fn snooze_choices() -> Vec<SnoozeChoice> {
    let at = |day, hour| Preset::At {
        day,
        hour,
        minute: 0,
    };
    let mut choices: Vec<_> = [
        Preset::After(60),
        Preset::After(180),
        at(Day::Today, 18),
        at(Day::Tomorrow, 9),
        at(Day::Weekday(0), 9),
    ]
    .into_iter()
    .filter_map(|p| resolve_preset(p, Local::now()))
    .collect();
    choices.sort_by_key(|p| p.until / 60000);
    choices.dedup_by_key(|p| p.until / 60000);
    choices
}
pub fn pick_preset(choice: &SnoozeChoice, at: u64) -> Option<u64> {
    match choice.preset {
        Preset::After(minutes) => Some(at + u64::from(minutes) * 60000),
        _ => (choice.until > at).then_some(choice.until),
    }
}
pub fn custom_snooze<T: TimeZone>(input: &str, now: DateTime<T>) -> Option<u64> {
    let input = input.trim();
    let wake = if let Some(unit) = input.chars().last().filter(|c| "mhd".contains(*c)) {
        let amount = input[..input.len() - 1].trim().parse::<f64>().ok()?;
        let factor = match unit {
            'h' => 3600000.0,
            'd' => 86400000.0,
            _ => 60000.0,
        };
        if !amount.is_finite() || amount <= 0.0 || amount * factor > i64::MAX as f64 {
            return None;
        }
        now.clone()
            .checked_add_signed(chrono::Duration::milliseconds((amount * factor) as i64))?
    } else {
        if input.len() != 16
            || !input.bytes().enumerate().all(|(i, c)| match i {
                4 | 7 => c == b'-',
                10 => c == b' ',
                13 => c == b':',
                _ => c.is_ascii_digit(),
            })
        {
            return None;
        }
        let naive = NaiveDateTime::parse_from_str(input, "%Y-%m-%d %H:%M").ok()?;
        now.timezone().from_local_datetime(&naive).earliest()?
    };
    (wake > now)
        .then(|| wake.timestamp_millis().try_into().ok())
        .flatten()
}
pub fn wake_label(until: u64, at: u64) -> String {
    let left = until.saturating_sub(at);
    if left == 0 {
        "now".into()
    } else if left < 3600000 {
        format!("{}m", left.div_ceil(60000))
    } else if left < 86400000 {
        format!("{}h", left.div_ceil(3600000))
    } else {
        format!("{}d", left.div_ceil(86400000))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono_tz::America::New_York;
    use serde_json::json;

    #[test]
    fn migration_preserves_legacy_bits_and_rejects_malformed_lifecycle() {
        let old = json!({"updated":100,"pinned":true,"archived":true,"unread":true});
        let l = Lifecycle::from_record(&old).unwrap();
        assert_eq!(l.pinned_at, Some(100000));
        assert_eq!(l.archived_at, Some(100000));
        assert!(l.unread());
        let record = json!({"lifecycle":l});
        assert_eq!(Lifecycle::from_record(&record).unwrap(), l);
        assert!(Lifecycle::from_record(&json!({"lifecycle":{"pinnedAt":"oops"}})).is_err());
        let mut empty = Lifecycle::new(1);
        assert!(!empty.mark_unread());
        empty.last_completed_at = Some(20);
        assert!(empty.mark_unread());
        assert_eq!(empty.last_visited_at, Some(19));
    }

    #[test]
    fn exact_section_priority_and_manual_override() {
        let mut l = Lifecycle::new(1);
        assert_eq!(section(&l, 10), Section::Active);
        l.pin(2, Some("n".into()));
        assert_eq!(section(&l, 10), Section::Pinned);
        l.settled_override = Some(Settled::Settled);
        assert_eq!(section(&l, 10), Section::Settled);
        l.snooze(3, 100);
        assert_eq!(section(&l, 10), Section::Snoozed);
        l.settle(20);
        assert_eq!(l.pinned_at, None);
        assert_eq!(l.snoozed_until, None);
        l.unsettle(21, true);
        assert_eq!(l.settled_override, Some(Settled::Active));
        assert_eq!(l.unsettled_at, Some(21));
        l.user_sent(22);
        assert_eq!(l.settled_override, None);
    }

    #[test]
    fn comparators_match_section_sort_keys() {
        let mut a = Lifecycle::new(10);
        let mut b = Lifecycle::new(20);
        a.pin_order_key = Some("n".into());
        assert_eq!(compare(Section::Pinned, &a, &b), Ordering::Less);
        b.pin_order_key = Some("b".into());
        assert_eq!(compare(Section::Pinned, &a, &b), Ordering::Greater);
        a.pin_order_key = None;
        b.pin_order_key = None;
        assert_eq!(compare(Section::Pinned, &a, &b), Ordering::Greater);
        a.last_completed_at = Some(30);
        assert_eq!(compare(Section::Active, &a, &b), Ordering::Less);
        a.snoozed_until = Some(50);
        b.snoozed_until = Some(60);
        assert_eq!(compare(Section::Snoozed, &a, &b), Ordering::Less);
        a.settled_at = Some(2);
        b.settled_at = Some(1);
        assert_eq!(compare(Section::Settled, &a, &b), Ordering::Less);
        a.settled_at = None;
        b.settled_at = None;
        assert_eq!(compare(Section::Settled, &a, &b), Ordering::Less);
    }

    #[test]
    fn fractional_pin_keys_match_t3_and_allow_repeated_insertion() {
        assert_eq!(pin_key_between(None, None).as_deref(), Some("n"));
        assert_eq!(pin_key_between(None, Some("n")).as_deref(), Some("h"));
        assert_eq!(pin_key_between(Some("n"), Some("o")).as_deref(), Some("nn"));
        for bad in ["", "a", "na", "A", "1"] {
            assert!(pin_key_between(Some(bad), None).is_none());
        }
        assert!(pin_key_between(Some("z"), Some("b")).is_none());
        let mut previous = "n".to_string();
        for _ in 0..1000 {
            let key = pin_key_between(Some(&previous), Some("o")).unwrap();
            assert!(previous < key && key.as_str() < "o");
            previous = key;
        }
        for count in [0, 1, 10, 1000] {
            let keys = spread_pin_keys(count);
            assert_eq!(keys.len(), count);
            assert!(keys.windows(2).all(|w| w[0] < w[1]));
            assert!(keys.iter().all(|k| !k.ends_with('a')));
        }
    }

    #[test]
    fn auto_settle_eligibility_uses_inactivity_and_never_captains() {
        let l = Lifecycle::new(1);
        let at = 3 * 86400000 + 1;
        assert_eq!(l.auto_settle_at(at, 3, false, false), Some(1));
        assert_eq!(l.auto_settle_at(at - 1, 3, false, false), None);
        for (days, busy, captain) in [(0, false, false), (3, true, false), (3, false, true)] {
            assert_eq!(l.auto_settle_at(at, days, busy, captain), None);
        }
        for field in ["pin", "archive", "optout", "active", "settled", "snooze"] {
            let mut l = l.clone();
            match field {
                "pin" => l.pinned_at = Some(1),
                "archive" => l.archived_at = Some(1),
                "optout" => l.auto_settle_disabled_at = Some(1),
                "active" => l.settled_override = Some(Settled::Active),
                "settled" => l.settled_override = Some(Settled::Settled),
                _ => l.snooze(1, at + 1),
            }
            assert_eq!(l.auto_settle_at(at, 3, false, false), None, "{field}");
        }
        let mut linked = l;
        linked.pull_request_finished_at = Some(2);
        assert!(linked.pr_settlement_ready(3));
        assert_eq!(linked.auto_settle_at(3, 0, false, false), Some(1));
        linked.latest_user_message_at = Some(3);
        assert!(!linked.pr_settlement_ready(3));
    }

    #[test]
    fn completion_does_not_wake_and_early_wake_remains_acknowledged() {
        let mut l = Lifecycle::new(1);
        l.snooze(10, 100);
        l.last_completed_at = Some(20);
        assert!(l.snoozed(30));
        l.last_snooze_wake_at = Some(40);
        assert!(!l.snoozed(50));
        assert_eq!(l.woke_at(120), Some(40));
        assert!(l.woke(50));
        l.last_visited_at = Some(50);
        assert!(!l.woke(120));
        l.unsnooze(130);
        assert_eq!(l.snoozed_until, None);
    }

    #[test]
    fn undo_plans_restore_pin_order_and_snooze_after_pin() {
        let mut l = Lifecycle::new(1);
        l.pin(2, Some("nn".into()));
        l.snooze(3, 100);
        assert_eq!(
            plan_undo(Action::Settle, &l, ""),
            vec![
                UndoStep::Unsettle,
                UndoStep::Pin(Some("nn".into())),
                UndoStep::Snooze(100)
            ]
        );
        assert_eq!(
            plan_undo(Action::Unpin, &l, ""),
            vec![UndoStep::Pin(Some("nn".into())), UndoStep::Snooze(100)]
        );
        assert_eq!(plan_undo(Action::Snooze, &l, ""), vec![UndoStep::Unsnooze]);
        assert_eq!(
            plan_undo(Action::Archive, &l, ""),
            vec![UndoStep::Unarchive]
        );
        assert_eq!(
            plan_undo(Action::Discard, &l, "draft"),
            vec![UndoStep::RestoreDraft("draft".into())]
        );
        let mut slot = None;
        UndoNotice::push(&mut slot, Action::Settle, 1, vec![], 10);
        UndoNotice::push(&mut slot, Action::Settle, 2, vec![], 20);
        assert_eq!(slot.as_ref().unwrap().entries.len(), 2);
        assert_eq!(slot.as_ref().unwrap().expires, 5020);
        UndoNotice::push(&mut slot, Action::Snooze, 3, vec![], 30);
        assert_eq!(slot.as_ref().unwrap().entries.len(), 1);
    }

    #[test]
    fn drop_plan_table_rejects_timed_targets() {
        use Section::*;
        for from in [Pinned, Active, Snoozed, Settled] {
            assert_eq!(plan_drop(from, Snoozed), DropPlan::None);
        }
        assert_eq!(plan_drop(Pinned, Pinned), DropPlan::ReorderPin);
        assert_eq!(plan_drop(Active, Pinned), DropPlan::Pin);
        assert_eq!(plan_drop(Pinned, Active), DropPlan::Unpin);
        assert_eq!(plan_drop(Settled, Active), DropPlan::Unsettle);
        assert_eq!(plan_drop(Snoozed, Active), DropPlan::Wake);
        assert_eq!(plan_drop(Active, Settled), DropPlan::Settle);
        assert_eq!(plan_drop(Active, Active), DropPlan::None);
    }

    #[test]
    fn calendar_presets_cross_dst_without_adding_24_hour_days() {
        for (date, expected_hours) in [((2026, 3, 7), 23), ((2026, 10, 31), 25)] {
            let now = New_York
                .with_ymd_and_hms(date.0, date.1, date.2, 9, 0, 0)
                .unwrap();
            let p = resolve_preset(
                Preset::At {
                    day: Day::Tomorrow,
                    hour: 9,
                    minute: 0,
                },
                now,
            )
            .unwrap();
            assert_eq!(
                p.until as i64 - now.timestamp_millis(),
                expected_hours * 3600000
            );
            assert_eq!(p.label, "Tomorrow");
            assert_eq!(p.time, "09:00");
        }
        let now = New_York.with_ymd_and_hms(2026, 3, 7, 23, 30, 0).unwrap();
        assert!(
            resolve_preset(
                Preset::At {
                    day: Day::Today,
                    hour: 18,
                    minute: 0
                },
                now
            )
            .is_none()
        );
        assert!(custom_snooze("2026-03-08 02:30", now).is_none());
        assert!(custom_snooze("2026-02-30 09:00", now).is_none());
        assert!(custom_snooze("-3h", now).is_none());
        assert!(custom_snooze("NaNm", now).is_none());
        assert_eq!(
            custom_snooze("90m", now),
            Some(now.timestamp_millis() as u64 + 5400000)
        );
        let sunday = New_York.with_ymd_and_hms(2026, 3, 8, 12, 0, 0).unwrap();
        assert_eq!(
            resolve_preset(
                Preset::At {
                    day: Day::Weekday(0),
                    hour: 9,
                    minute: 0
                },
                sunday
            )
            .unwrap()
            .label,
            "Tomorrow"
        );
        let monday = New_York.with_ymd_and_hms(2026, 3, 9, 12, 0, 0).unwrap();
        let next = resolve_preset(
            Preset::At {
                day: Day::Weekday(0),
                hour: 9,
                minute: 0,
            },
            monday,
        )
        .unwrap();
        assert_eq!(next.label, "Next week");
        assert_eq!(pick_preset(&next, next.until), None);
        let duration = resolve_preset(Preset::After(60), monday).unwrap();
        assert_eq!(duration.label, "In 1 hour");
        assert_eq!(
            pick_preset(&duration, duration.until),
            Some(duration.until + 3600000)
        );
        assert_eq!(wake_label(1, 2), "now");
        assert_eq!(wake_label(60001, 0), "2m");
        assert_eq!(wake_label(3600001, 0), "2h");
        assert_eq!(wake_label(86400001, 0), "2d");
        let choices = snooze_choices();
        assert!(
            choices
                .windows(2)
                .all(|p| p[0].until / 60000 < p[1].until / 60000)
        );
    }
}
