//! Local-time schedules and a durable, at-most-once dispatch ledger.
use chrono::{
    DateTime, Datelike, Duration, Local, LocalResult, NaiveTime, TimeZone, Timelike, Utc,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, path::PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Schedule {
    Every { minutes: u32 },
    Daily { at: String },
    Weekdays { at: String },
}

impl Schedule {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Every { minutes } if *minutes < 15 => {
                Err("The interval must be at least 15 minutes.".into())
            }
            Self::Daily { at } | Self::Weekdays { at } => parse_time(at).map(|_| ()),
            _ => Ok(()),
        }
    }

    /// Ambiguous fall-back times use the first occurrence. Spring-forward gaps use the
    /// first valid minute after the gap. A calendar schedule has one slot per local date.
    pub fn next<T: TimeZone>(&self, after: DateTime<T>) -> DateTime<T> {
        if let Self::Every { minutes } = self {
            return after + Duration::minutes(i64::from(*minutes));
        }
        let (Self::Daily { at } | Self::Weekdays { at }) = self else {
            unreachable!()
        };
        let time = parse_time(at).expect("validated schedule");
        let timezone = after.timezone();
        let mut date = after.date_naive();
        loop {
            if !matches!(self, Self::Weekdays { .. }) || date.weekday().number_from_monday() <= 5 {
                let mut naive = date.and_time(time);
                // Some time zones skip a whole date. Stop at its end and try the next date.
                while naive.date() == date {
                    let candidate = match timezone.from_local_datetime(&naive) {
                        LocalResult::Single(t) => Some(t),
                        LocalResult::Ambiguous(a, b) => Some(a.min(b)),
                        LocalResult::None => None,
                    };
                    if let Some(candidate) = candidate {
                        if candidate > after {
                            return candidate;
                        }
                        break;
                    }
                    naive += Duration::minutes(1);
                }
            }
            date = date.succ_opt().expect("calendar date overflow");
        }
    }
}

fn parse_time(value: &str) -> Result<NaiveTime, String> {
    NaiveTime::parse_from_str(value, "%H:%M")
        .ok()
        .filter(|t| value == format!("{:02}:{:02}", t.hour(), t.minute()))
        .ok_or_else(|| "Use local time in HH:MM format.".into())
}

pub fn tools() -> Vec<serde_json::Value> {
    use serde_json::json;
    vec![
        json!({
            "name":"create_routine",
            "description":"Schedule work in the calling captain's home. Every run is a Shina-labelled message. No approval is needed to create a routine. Routines wait for pending approvals/questions and while the user is away. Only a captain home can manage its routines.",
            "inputSchema":{"type":"object","properties":{
                "name":{"type":"string","minLength":1,"maxLength":200},
                "prompt":{"type":"string","minLength":1,"maxLength":64000},
                "schedule":{"oneOf":[
                    {"type":"object","properties":{"kind":{"const":"every"},"minutes":{"type":"integer","minimum":15}},"required":["kind","minutes"],"additionalProperties":false},
                    {"type":"object","properties":{"kind":{"enum":["daily","weekdays"]},"at":{"type":"string","pattern":"^([01][0-9]|2[0-3]):[0-5][0-9]$"}},"required":["kind","at"],"additionalProperties":false}
                ]},
                "missedPolicy":{"type":"string","enum":["skip","run_once"],"default":"skip"}
            },"required":["name","prompt","schedule"],"additionalProperties":false}
        }),
        json!({"name":"list_routines","description":"List the calling captain's visible routines, schedules and latest run state.",
            "inputSchema":{"type":"object","properties":{}},"annotations":{"readOnlyHint":true}}),
        json!({"name":"pause_routine","description":"Pause or unpause one of the calling captain's routines.",
            "inputSchema":{"type":"object","properties":{"id":{"type":"integer"},"paused":{"type":"boolean","default":true}},"required":["id"]}}),
        json!({"name":"delete_routine","description":"Remove one of the calling captain's routines. This does not cancel an already accepted turn.",
            "inputSchema":{"type":"object","properties":{"id":{"type":"integer"}},"required":["id"]}}),
    ]
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissedPolicy {
    #[default]
    Skip,
    RunOnce,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Routine {
    pub id: u64,
    pub captain: usize,
    /// Stable home identity prevents roster renumbering from changing the owner.
    pub home: usize,
    pub name: String,
    pub prompt: String,
    pub schedule: Schedule,
    #[serde(default)]
    pub missed_policy: MissedPolicy,
    pub paused: bool,
    pub next_at: i64,
    pub last_slot: Option<i64>,
    pub last_status: String,
    pub active_delivery: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoutineView {
    pub id: u64,
    pub name: String,
    pub prompt: String,
    pub schedule: Schedule,
    pub missed_policy: MissedPolicy,
    pub paused: bool,
    pub next_at: i64,
    pub last_status: String,
    pub running: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ledger {
    version: u32,
    next_id: u64,
    routines: Vec<Routine>,
}

pub struct Routines {
    path: PathBuf,
    ledger: Ledger,
    /// A damaged file is never replaced by an empty ledger.
    pub error: Option<String>,
}

impl Default for Routines {
    fn default() -> Self {
        Self {
            path: PathBuf::new(),
            ledger: Ledger {
                version: 1,
                next_id: 1,
                routines: vec![],
            },
            error: None,
        }
    }
}

impl Routines {
    pub fn load(path: PathBuf) -> Self {
        let mut result = Self {
            path,
            ..Self::default()
        };
        let loaded = (|| {
            let Some(value) = crate::store::load(&result.path)? else {
                return Ok(None);
            };
            let ledger: Ledger =
                serde_json::from_value(value).map_err(|e| format!("Invalid routines: {e}"))?;
            let mut ids = HashSet::new();
            if ledger.version != 1 || ledger.next_id == 0 {
                return Err("Unsupported routines file.".into());
            }
            for routine in &ledger.routines {
                routine.schedule.validate()?;
                if routine.id == 0
                    || routine.home == 0
                    || routine.id >= ledger.next_id
                    || !ids.insert(routine.id)
                    || routine.name.trim().is_empty()
                    || routine.name.len() > 200
                    || routine.prompt.trim().is_empty()
                    || routine.prompt.len() > 64000
                    || DateTime::from_timestamp(routine.next_at, 0).is_none()
                    || routine
                        .last_slot
                        .is_some_and(|slot| slot >= routine.next_at)
                    || routine.active_delivery.as_ref().is_some_and(|id| {
                        routine
                            .last_slot
                            .is_none_or(|slot| *id != format!("routine-{}-{slot}", routine.id))
                    })
                {
                    return Err("Invalid routine record.".into());
                }
            }
            Ok(Some(ledger))
        })();
        match loaded {
            Ok(Some(mut ledger)) => {
                for routine in &mut ledger.routines {
                    if routine.active_delivery.take().is_some() {
                        routine.last_status = "Interrupted by restart. Review the home conversation; this slot was not replayed.".into();
                    }
                }
                result.ledger = ledger;
            }
            Ok(None) => {}
            Err(error) => result.error = Some(error),
        }
        result
    }

    fn commit(&mut self, ledger: Ledger) -> Result<(), String> {
        if let Some(error) = &self.error {
            return Err(format!(
                "{error} The routines file has not been overwritten."
            ));
        }
        if self.path.as_os_str().is_empty() {
            return Err("Routine storage is not available.".into());
        }
        crate::store::write_atomic(
            &self.path,
            &serde_json::to_vec(&ledger).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("Could not save routines: {e}"))?;
        self.ledger = ledger;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create(
        &mut self,
        captain: usize,
        home: usize,
        name: String,
        prompt: String,
        schedule: Schedule,
        missed_policy: MissedPolicy,
        now: DateTime<Local>,
    ) -> Result<u64, String> {
        schedule.validate()?;
        if name.trim().is_empty()
            || name.len() > 200
            || prompt.trim().is_empty()
            || prompt.len() > 64000
        {
            return Err(
                "A routine needs a name of at most 200 bytes and a prompt of at most 64000 bytes."
                    .into(),
            );
        }
        let mut ledger = self.ledger.clone();
        let id = ledger.next_id;
        ledger.next_id = id.checked_add(1).ok_or("Routine id overflow.")?;
        ledger.routines.push(Routine {
            id,
            captain,
            home,
            name,
            prompt,
            next_at: schedule.next(now).timestamp(),
            schedule,
            missed_policy,
            paused: false,
            last_slot: None,
            last_status: "Scheduled".into(),
            active_delivery: None,
        });
        self.commit(ledger)?;
        Ok(id)
    }

    pub fn entries(&self) -> &[Routine] {
        &self.ledger.routines
    }

    pub fn views(&self, home: usize) -> Vec<RoutineView> {
        self.entries()
            .iter()
            .filter(|r| r.home == home)
            .map(|r| RoutineView {
                id: r.id,
                name: r.name.clone(),
                prompt: r.prompt.clone(),
                schedule: r.schedule.clone(),
                missed_policy: r.missed_policy,
                paused: r.paused,
                next_at: r.next_at,
                last_status: r.last_status.clone(),
                running: r.active_delivery.is_some(),
            })
            .collect()
    }

    pub fn set_paused(&mut self, id: u64, paused: bool) -> Result<(), String> {
        self.edit(id, |r| r.paused = paused)
    }

    pub fn delete(&mut self, id: u64) -> Result<(), String> {
        let mut ledger = self.ledger.clone();
        if !ledger.routines.iter().any(|r| r.id == id) {
            return Err("No such routine.".into());
        }
        ledger.routines.retain(|r| r.id != id);
        self.commit(ledger)
    }

    fn edit(&mut self, id: u64, f: impl FnOnce(&mut Routine)) -> Result<(), String> {
        let mut ledger = self.ledger.clone();
        f(ledger
            .routines
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or("No such routine.")?);
        self.commit(ledger)
    }

    /// Reserve the slot on disk before returning its message. A restart never replays a
    /// reserved slot, even if a crash prevented Pi from acknowledging it.
    pub fn due(
        &mut self,
        now: DateTime<Local>,
        blocked: impl Fn(usize) -> bool,
    ) -> Result<Vec<(usize, String)>, String> {
        let mut ledger = self.ledger.clone();
        let mut messages = vec![];
        let mut changed = false;
        for r in &mut ledger.routines {
            if r.paused
                || r.active_delivery.is_some()
                || blocked(r.home)
                || r.next_at > now.timestamp()
            {
                continue;
            }
            let slot = r.next_at;
            let missed = now.timestamp().saturating_sub(slot) >= 60
                && r.last_status != "Waiting for your response";
            r.next_at = match r.schedule {
                Schedule::Every { minutes } => {
                    let step = i64::from(minutes) * 60;
                    slot + ((now.timestamp() - slot) / step + 1) * step
                }
                _ => r.schedule.next(now).timestamp(),
            };
            changed = true;
            if r.last_slot.is_some_and(|last| last >= slot) {
                continue;
            }
            r.last_slot = Some(slot);
            if missed && r.missed_policy == MissedPolicy::Skip {
                r.last_status = "Skipped missed run".into();
                continue;
            }
            let delivery = format!("routine-{}-{slot}", r.id);
            r.active_delivery = Some(delivery.clone());
            r.last_status = "Queued".into();
            messages.push((
                r.home,
                format!(
                    "[Shina orchestration]\n[delivery {delivery}]\nRoutine: {}\n\n{}",
                    r.name, r.prompt
                ),
            ));
        }
        if changed {
            self.commit(ledger)?
        }
        Ok(messages)
    }

    pub fn finish(&mut self, id: u64, status: &str) -> Result<(), String> {
        self.edit(id, |r| {
            r.active_delivery = None;
            r.last_status = status.into();
        })
    }

    pub fn note_away(&mut self) -> Result<(), String> {
        let mut ledger = self.ledger.clone();
        let mut changed = false;
        for r in &mut ledger.routines {
            if !r.paused && r.last_status != "Paused while you were away" {
                r.last_status = "Paused while you were away".into();
                changed = true;
            }
        }
        if changed {
            self.commit(ledger)?
        }
        Ok(())
    }

    pub fn hold_for_response(
        &mut self,
        now: DateTime<Local>,
        waiting: impl Fn(usize) -> bool,
    ) -> Result<(), String> {
        let mut ledger = self.ledger.clone();
        let mut changed = false;
        for r in &mut ledger.routines {
            if !r.paused
                && r.active_delivery.is_none()
                && r.next_at <= now.timestamp()
                && waiting(r.home)
                && r.last_status != "Waiting for your response"
            {
                r.last_status = "Waiting for your response".into();
                changed = true;
            }
        }
        if changed {
            self.commit(ledger)?
        }
        Ok(())
    }

    /// One scheduler deadline, excluding blocked captains. Input/resume and approval events
    /// re-evaluate excluded schedules; they do not install a second schedule timer.
    pub fn deadline(&self, blocked: impl Fn(usize) -> bool) -> Option<DateTime<Utc>> {
        self.entries()
            .iter()
            .filter(|r| {
                !r.paused
                    && r.active_delivery.is_none()
                    && (!blocked(r.home) || r.next_at > Local::now().timestamp())
            })
            .filter_map(|r| DateTime::from_timestamp(r.next_at, 0))
            .min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono_tz::America::New_York;

    #[test]
    fn calendar_slots_handle_dst_and_weekends() {
        let daily = Schedule::Daily { at: "02:30".into() };
        let before = New_York.with_ymd_and_hms(2026, 3, 7, 4, 0, 0).unwrap();
        let next = daily.next(before);
        assert_eq!((next.day(), next.hour(), next.minute()), (8, 3, 0));
        let fall = Schedule::Daily { at: "01:30".into() };
        let first = fall.next(New_York.with_ymd_and_hms(2026, 10, 31, 4, 0, 0).unwrap());
        assert_eq!(
            fall.next(first).day(),
            2,
            "no second slot during the repeated hour"
        );
        let weekdays = Schedule::Weekdays { at: "09:00".into() };
        let friday = New_York.with_ymd_and_hms(2026, 11, 6, 10, 0, 0).unwrap();
        assert_eq!(weekdays.next(friday).weekday(), chrono::Weekday::Mon);
        assert_eq!(weekdays.next(friday).hour(), 9);
    }

    fn fixture() -> Routines {
        let root = crate::store::directory().unwrap().join("routines");
        std::fs::create_dir_all(&root).unwrap();
        Routines::load(root.join("routines.json"))
    }

    #[test]
    fn dedupe_survives_restart_and_missed_runs_run_only_once() {
        let mut store = fixture();
        let now = Local::now();
        store
            .create(
                0,
                7,
                "Check".into(),
                "Read status".into(),
                Schedule::Every { minutes: 15 },
                MissedPolicy::RunOnce,
                now,
            )
            .unwrap();
        let later = now + Duration::hours(4);
        assert_eq!(store.due(later, |_| false).unwrap().len(), 1);
        let mut reopened = Routines::load(store.path.clone());
        assert!(reopened.due(later, |_| false).unwrap().is_empty());
        assert!(reopened.entries()[0].last_status.contains("not replayed"));
    }

    #[test]
    fn skip_and_approval_wait_and_away_note_are_durable() {
        let mut store = fixture();
        let now = Local::now();
        store
            .create(
                0,
                7,
                "Check".into(),
                "Read status".into(),
                Schedule::Every { minutes: 15 },
                MissedPolicy::Skip,
                now,
            )
            .unwrap();
        let later = now + Duration::minutes(15);
        assert!(store.due(later, |_| true).unwrap().is_empty());
        store.note_away().unwrap();
        assert_eq!(store.views(7)[0].last_status, "Paused while you were away");
        assert!(
            store
                .due(later + Duration::hours(3), |_| false)
                .unwrap()
                .is_empty()
        );
        assert_eq!(store.views(7)[0].last_status, "Skipped missed run");
    }

    #[test]
    fn approvals_defer_instead_of_skipping_and_pausing_is_reversible() {
        let mut store = fixture();
        let now = Local::now();
        let id = store
            .create(
                0,
                7,
                "Check".into(),
                "Read status".into(),
                Schedule::Every { minutes: 15 },
                MissedPolicy::Skip,
                now,
            )
            .unwrap();
        store.set_paused(id, true).unwrap();
        assert!(
            store
                .due(now + Duration::minutes(15), |_| false)
                .unwrap()
                .is_empty()
        );
        store.set_paused(id, false).unwrap();
        store
            .hold_for_response(now + Duration::minutes(15), |_| true)
            .unwrap();
        assert_eq!(store.views(7)[0].last_status, "Waiting for your response");
        assert_eq!(
            store
                .due(now + Duration::hours(3), |_| false)
                .unwrap()
                .len(),
            1
        );
        assert!(
            store
                .due(now + Duration::hours(3), |_| false)
                .unwrap()
                .is_empty()
        );
        store.delete(id).unwrap();
        assert!(Routines::load(store.path).entries().is_empty());
    }

    #[test]
    fn damaged_file_blocks_mutation_and_bad_schedules_are_rejected() {
        let mut store = fixture();
        std::fs::write(&store.path, "{broken").unwrap();
        store = Routines::load(store.path.clone());
        assert!(
            store
                .create(
                    0,
                    7,
                    "Check".into(),
                    "Prompt".into(),
                    Schedule::Every { minutes: 15 },
                    MissedPolicy::Skip,
                    Local::now()
                )
                .is_err()
        );
        assert_eq!(std::fs::read_to_string(&store.path).unwrap(), "{broken");
        assert!(Schedule::Every { minutes: 14 }.validate().is_err());
        assert!(Schedule::Daily { at: "9:00".into() }.validate().is_err());
    }
}
