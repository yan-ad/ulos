//! Session-only wall-clock schedules. No GPUI, I/O, worker ownership or persistence.
//!
//! A tick consumes elapsed occurrences, not transfer results. Consequently a manual
//! stop or an empty/completed queue cannot restart the same occurrence. Clock rollback
//! never lowers the consumption watermark. Missed windows are not replayed.
use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveTime, TimeZone};

pub type QueueId = u64;
pub const DEFAULT_QUEUE: QueueId = 0;
pub const MAX_QUEUES: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScheduleAction {
    Start,
    Stop,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScheduleSpec {
    Once {
        date: NaiveDate,
        start: NaiveTime,
        stop: Option<NaiveTime>,
    },
    Weekly {
        /// Monday first. A stop earlier than start belongs to the next day.
        days: [bool; 7],
        start: NaiveTime,
        stop: Option<NaiveTime>,
    },
}

#[derive(Clone, Debug)]
pub struct Schedule {
    pub spec: ScheduleSpec,
    pub enabled: bool,
    consumed_through: Option<DateTime<Local>>,
    active_end: Option<DateTime<Local>>,
}

impl Schedule {
    pub fn new(spec: ScheduleSpec, now: DateTime<Local>) -> Result<Self, &'static str> {
        let (start, stop) = match &spec {
            ScheduleSpec::Once { date, start, stop } => {
                let instant =
                    local_at(*date, *start).ok_or("Start does not exist in local time")?;
                if instant <= now {
                    return Err("One-time start must be in the future");
                }
                (*start, *stop)
            }
            ScheduleSpec::Weekly { days, start, stop } => {
                if !days.iter().any(|day| *day) {
                    return Err("Select at least one weekday");
                }
                (*start, *stop)
            }
        };
        if stop == Some(start) {
            return Err("Stop must differ from start");
        }
        Ok(Self {
            spec,
            enabled: true,
            consumed_through: None,
            active_end: None,
        })
    }

    pub fn summary(&self) -> String {
        let (prefix, start, stop) = match &self.spec {
            ScheduleSpec::Once { date, start, stop } => (date.to_string(), start, stop),
            ScheduleSpec::Weekly { days, start, stop } => {
                let names = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
                let selected: Vec<_> = names
                    .into_iter()
                    .zip(days)
                    .filter_map(|(name, on)| on.then_some(name))
                    .collect();
                (selected.join(", "), start, stop)
            }
        };
        let stop = stop
            .map(|time| format!("–{}", time.format("%H:%M")))
            .unwrap_or_default();
        format!(
            "{prefix} {}{stop}{}",
            start.format("%H:%M"),
            if self.enabled { "" } else { " (disabled)" }
        )
    }

    fn tick(&mut self, now: DateTime<Local>) -> Option<ScheduleAction> {
        // End ownership survives disabling the schedule. Stop wins over catch-up
        // starts if a suspended app wakes across both boundaries.
        let stopped = self.active_end.is_some_and(|end| now >= end);
        if stopped {
            self.active_end = None;
        }
        let previous = self.consumed_through;
        self.consumed_through = Some(previous.map_or(now, |old| old.max(now)));
        if stopped {
            if matches!(self.spec, ScheduleSpec::Once { .. }) {
                self.enabled = false;
            }
            return Some(ScheduleAction::Stop);
        }
        if !self.enabled || previous.is_some_and(|old| now <= old) {
            return None;
        }
        // At most two dates: today and yesterday for an overnight interval.
        let today = now.date_naive();
        let dates = [today.pred_opt(), Some(today)];
        for date in dates.into_iter().flatten() {
            let (start, stop) = match &self.spec {
                ScheduleSpec::Once {
                    date: scheduled,
                    start,
                    stop,
                } if *scheduled == date => (*start, *stop),
                ScheduleSpec::Weekly { days, start, stop }
                    if days[date.weekday().num_days_from_monday() as usize] =>
                {
                    (*start, *stop)
                }
                _ => continue,
            };
            let Some(begin) = local_at(date, start) else {
                continue;
            };
            if begin > now || previous.is_some_and(|old| begin <= old) {
                continue;
            }
            let end = match stop {
                Some(time) => {
                    let end_date = if time < start {
                        date.succ_opt()
                    } else {
                        Some(date)
                    };
                    let Some(end) = end_date.and_then(|date| local_at(date, time)) else {
                        continue;
                    };
                    Some(end)
                }
                None => None,
            };
            // No-stop catch-up is limited to the start's local calendar day;
            // that cutoff is not an automatic stop of a started queue.
            if end.is_some_and(|end| now >= end) || (end.is_none() && date != today) {
                continue;
            }
            self.active_end = end;
            if end.is_none() && matches!(self.spec, ScheduleSpec::Once { .. }) {
                self.enabled = false;
            }
            return Some(ScheduleAction::Start);
        }
        if let ScheduleSpec::Once { date, start, .. } = &self.spec
            && local_at(*date, *start).is_some_and(|start| start <= now)
        {
            self.enabled = false;
        }
        None
    }
}

/// Gaps skip an occurrence; folds choose only the earlier instant (including stops).
fn local_at(date: NaiveDate, time: NaiveTime) -> Option<DateTime<Local>> {
    match Local.from_local_datetime(&date.and_time(time)) {
        chrono::LocalResult::None => None,
        chrono::LocalResult::Single(instant) => Some(instant),
        // Some native timezone backends return fold candidates out of order.
        chrono::LocalResult::Ambiguous(a, b) => Some(a.min(b)),
    }
}

#[derive(Clone, Debug)]
pub struct NamedQueue {
    pub id: QueueId,
    pub name: String,
    pub running: bool,
    pub schedule: Option<Schedule>,
}

pub struct Scheduler {
    pub queues: Vec<NamedQueue>,
    pub selected: QueueId,
    next_id: QueueId,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self {
            queues: vec![NamedQueue {
                id: DEFAULT_QUEUE,
                name: "Main queue".into(),
                running: false,
                schedule: None,
            }],
            selected: DEFAULT_QUEUE,
            next_id: 0,
        }
    }
}

impl Scheduler {
    pub fn queue(&self, id: QueueId) -> Option<&NamedQueue> {
        self.queues.iter().find(|queue| queue.id == id)
    }
    pub fn queue_mut(&mut self, id: QueueId) -> Option<&mut NamedQueue> {
        self.queues.iter_mut().find(|queue| queue.id == id)
    }

    fn validate_name(&self, name: &str, except: Option<QueueId>) -> Result<String, &'static str> {
        if name.chars().any(char::is_control) {
            return Err("Queue names cannot contain controls");
        }
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 80 {
            return Err("Queue names must contain 1–80 characters");
        }
        if self.queues.iter().any(|queue| {
            Some(queue.id) != except && queue.name.to_lowercase() == name.to_lowercase()
        }) {
            return Err("Queue names must be unique");
        }
        Ok(name.into())
    }

    pub fn create(&mut self, name: &str) -> Result<QueueId, &'static str> {
        if self.queues.len() >= MAX_QUEUES {
            return Err("At most 32 queues are allowed");
        }
        let name = self.validate_name(name, None)?;
        let id = self
            .next_id
            .checked_add(1)
            .ok_or("Queue identifiers exhausted")?;
        self.next_id = id;
        self.queues.push(NamedQueue {
            id,
            name,
            running: false,
            schedule: None,
        });
        Ok(id)
    }

    pub fn rename(&mut self, id: QueueId, name: &str) -> Result<(), &'static str> {
        let name = self.validate_name(name, Some(id))?;
        self.queue_mut(id).ok_or("Queue not found")?.name = name;
        Ok(())
    }

    /// The manager must separately reassign item membership before removal.
    pub fn remove(&mut self, id: QueueId) -> Result<(), &'static str> {
        if id == DEFAULT_QUEUE {
            return Err("The main queue cannot be removed");
        }
        if self.queue(id).is_none() {
            return Err("Queue not found");
        }
        self.queues.retain(|queue| queue.id != id);
        if self.selected == id {
            self.selected = DEFAULT_QUEUE;
        }
        Ok(())
    }

    pub fn set_schedule(
        &mut self,
        id: QueueId,
        schedule: Option<Schedule>,
        now: DateTime<Local>,
    ) -> Result<(), &'static str> {
        // Revalidate user-visible public spec, and start a fresh consumption epoch.
        let schedule = schedule
            .map(|schedule| {
                let enabled = schedule.enabled;
                Schedule::new(schedule.spec, now).map(|mut schedule| {
                    schedule.enabled = enabled;
                    schedule
                })
            })
            .transpose()?;
        self.queue_mut(id).ok_or("Queue not found")?.schedule = schedule;
        Ok(())
    }

    pub fn tick(&mut self, now: DateTime<Local>) -> Vec<(QueueId, ScheduleAction)> {
        let mut actions = Vec::new();
        for queue in &mut self.queues {
            if let Some(action) = queue
                .schedule
                .as_mut()
                .and_then(|schedule| schedule.tick(now))
            {
                queue.running = action == ScheduleAction::Start;
                actions.push((queue.id, action));
            }
        }
        actions.sort_by_key(|(_, action)| *action == ScheduleAction::Start);
        actions
    }
    pub fn has_enabled_schedule(&self) -> bool {
        self.queues.iter().any(|queue| {
            queue
                .schedule
                .as_ref()
                .is_some_and(|schedule| schedule.enabled)
        })
    }
    pub fn any_running(&self) -> bool {
        self.queues.iter().any(|queue| queue.running)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(day: u32, hour: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(2026, 10, day, hour, 0, 0)
            .earliest()
            .unwrap()
    }
    fn time(hour: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(hour, 0, 0).unwrap()
    }
    fn once(stop: Option<u32>) -> Schedule {
        Schedule::new(
            ScheduleSpec::Once {
                date: at(19, 0).date_naive(),
                start: time(10),
                stop: stop.map(time),
            },
            at(18, 0),
        )
        .unwrap()
    }
    fn install(schedule: Schedule) -> Scheduler {
        let mut scheduler = Scheduler::default();
        scheduler
            .set_schedule(DEFAULT_QUEUE, Some(schedule), at(18, 0))
            .unwrap();
        scheduler
    }
    #[test]
    fn bounded_names_and_monotonic_ids() {
        let mut scheduler = Scheduler::default();
        assert!(scheduler.create("MAIN QUEUE").is_err());
        for name in ["", " \n ", &"a".repeat(81)] {
            assert!(scheduler.create(name).is_err());
        }
        let first = scheduler.create(" Work ").unwrap();
        assert_eq!(scheduler.queue(first).unwrap().name, "Work");
        assert!(scheduler.rename(first, "main queue").is_err());
        scheduler.selected = first;
        scheduler.remove(first).unwrap();
        assert_eq!(scheduler.selected, DEFAULT_QUEUE);
        assert!(scheduler.remove(DEFAULT_QUEUE).is_err());
        assert!(scheduler.create("Work").unwrap() > first);
        while scheduler.queues.len() < MAX_QUEUES {
            scheduler
                .create(&format!("Queue {}", scheduler.queues.len()))
                .unwrap();
        }
        assert!(scheduler.create("overflow").is_err());
    }
    #[test]
    fn validation_is_atomic() {
        assert!(
            Schedule::new(
                ScheduleSpec::Once {
                    date: at(18, 0).date_naive(),
                    start: time(0),
                    stop: None
                },
                at(18, 0)
            )
            .is_err()
        );
        assert!(
            Schedule::new(
                ScheduleSpec::Weekly {
                    days: [false; 7],
                    start: time(10),
                    stop: None
                },
                at(18, 0)
            )
            .is_err()
        );
        assert!(
            Schedule::new(
                ScheduleSpec::Weekly {
                    days: [true; 7],
                    start: time(10),
                    stop: Some(time(10))
                },
                at(18, 0)
            )
            .is_err()
        );
        let mut scheduler = install(once(Some(12)));
        let mut invalid = once(Some(12));
        invalid.spec = ScheduleSpec::Weekly {
            days: [false; 7],
            start: time(10),
            stop: None,
        };
        assert!(scheduler.set_schedule(0, Some(invalid), at(18, 0)).is_err());
        assert!(matches!(
            scheduler.queue(0).unwrap().schedule.as_ref().unwrap().spec,
            ScheduleSpec::Once { .. }
        ));
    }
    #[test]
    fn catchup_manual_stop_and_rollback_are_consumed_once() {
        let mut scheduler = install(once(Some(12)));
        assert!(scheduler.tick(at(19, 9)).is_empty());
        assert_eq!(scheduler.tick(at(19, 11)), vec![(0, ScheduleAction::Start)]);
        scheduler.queue_mut(0).unwrap().running = false; // manual stop / exhausted queue
        for now in [at(19, 11), at(19, 9), at(19, 10), at(19, 11)] {
            assert!(scheduler.tick(now).is_empty());
        }
        assert_eq!(scheduler.tick(at(19, 13)), vec![(0, ScheduleAction::Stop)]);
        assert!(!scheduler.has_enabled_schedule());
        assert!(scheduler.tick(at(19, 14)).is_empty());
    }
    #[test]
    fn ended_and_no_stop_windows_are_never_replayed() {
        let mut scheduler = install(once(Some(12)));
        assert!(scheduler.tick(at(19, 13)).is_empty());
        assert!(scheduler.tick(at(19, 11)).is_empty());
        let mut scheduler = install(once(None));
        assert!(scheduler.tick(at(20, 0)).is_empty());
        let mut scheduler = install(once(None));
        assert_eq!(scheduler.tick(at(19, 23)), vec![(0, ScheduleAction::Start)]);
        assert!(scheduler.tick(at(20, 0)).is_empty());
        assert!(scheduler.any_running());
    }
    #[test]
    fn weekly_overnight_and_long_sleep_stop_wins() {
        // October 19, 2026 is Monday.
        let weekly = Schedule::new(
            ScheduleSpec::Weekly {
                days: [true, false, false, false, false, false, false],
                start: time(22),
                stop: Some(time(2)),
            },
            at(18, 0),
        )
        .unwrap();
        let mut scheduler = install(weekly.clone());
        assert_eq!(scheduler.tick(at(20, 1)), vec![(0, ScheduleAction::Start)]);
        assert_eq!(scheduler.tick(at(20, 2)), vec![(0, ScheduleAction::Stop)]);
        assert!(scheduler.tick(at(21, 23)).is_empty());
        assert_eq!(scheduler.tick(at(26, 23)), vec![(0, ScheduleAction::Start)]);
        let mut scheduler = install(weekly);
        assert_eq!(scheduler.tick(at(19, 23)), vec![(0, ScheduleAction::Start)]);
        // Sleep through end and into the next occurrence: stop, not stop+start.
        assert_eq!(scheduler.tick(at(26, 23)), vec![(0, ScheduleAction::Stop)]);
        assert!(scheduler.tick(at(26, 23)).is_empty());
    }
    #[test]
    fn disabled_and_deleted_queues_do_not_start() {
        let mut scheduler = install(once(Some(12)));
        scheduler
            .queue_mut(0)
            .unwrap()
            .schedule
            .as_mut()
            .unwrap()
            .enabled = false;
        assert!(scheduler.tick(at(19, 11)).is_empty());
        scheduler
            .queue_mut(0)
            .unwrap()
            .schedule
            .as_mut()
            .unwrap()
            .enabled = true;
        assert!(scheduler.tick(at(19, 11)).is_empty());
        let id = scheduler.create("Deleted").unwrap();
        scheduler
            .set_schedule(id, Some(once(None)), at(18, 0))
            .unwrap();
        scheduler.remove(id).unwrap();
        assert!(scheduler.tick(at(19, 11)).is_empty());
        // Disabling a started schedule still honors its owned stop boundary.
        let mut scheduler = install(once(Some(12)));
        scheduler.tick(at(19, 10));
        scheduler
            .queue_mut(0)
            .unwrap()
            .schedule
            .as_mut()
            .unwrap()
            .enabled = false;
        assert_eq!(scheduler.tick(at(19, 12)), vec![(0, ScheduleAction::Stop)]);
    }
    /// Run with TZ=America/New_York to exercise the actual local DST database.
    #[test]
    fn dst_gap_skips_and_fold_uses_only_earliest() {
        let gap_date = NaiveDate::from_ymd_opt(2026, 3, 8).unwrap();
        let fold_date = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
        let fold_time = NaiveTime::from_hms_opt(1, 30, 0).unwrap();
        let chrono::LocalResult::Ambiguous(early, late) =
            Local.from_local_datetime(&fold_date.and_time(fold_time))
        else {
            // Other local zones need not have this particular fold.
            return;
        };
        assert_eq!(local_at(fold_date, fold_time), Some(early.min(late)));
        assert!(local_at(gap_date, NaiveTime::from_hms_opt(2, 30, 0).unwrap()).is_none());
        let before = Local
            .with_ymd_and_hms(2026, 10, 31, 0, 0, 0)
            .earliest()
            .unwrap();
        let mut schedule = Schedule::new(
            ScheduleSpec::Weekly {
                days: [true; 7],
                start: fold_time,
                stop: Some(time(3)),
            },
            before,
        )
        .unwrap();
        assert_eq!(schedule.tick(early.min(late)), Some(ScheduleAction::Start));
        assert_eq!(schedule.tick(early.max(late)), None);
        let mut gap = Schedule::new(
            ScheduleSpec::Weekly {
                days: [true; 7],
                start: NaiveTime::from_hms_opt(2, 30, 0).unwrap(),
                stop: Some(time(4)),
            },
            before,
        )
        .unwrap();
        let after_gap = Local
            .with_ymd_and_hms(2026, 3, 8, 3, 0, 0)
            .earliest()
            .unwrap();
        assert_eq!(gap.tick(after_gap), None);
    }

    #[test]
    fn once_overnight_stop() {
        let mut scheduler = install(once(Some(2)));
        assert_eq!(scheduler.tick(at(20, 1)), vec![(0, ScheduleAction::Start)]);
        assert_eq!(scheduler.tick(at(20, 2)), vec![(0, ScheduleAction::Stop)]);
    }
}
