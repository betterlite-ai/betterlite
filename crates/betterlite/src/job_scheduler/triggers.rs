use chrono::{DateTime, Local, LocalResult, NaiveDateTime, TimeDelta, TimeZone, Utc};
use croner::Cron;

use crate::scheduled_job_stagger::Trigger;

use super::SchedulerError;
use super::fields::{CronField, normalize_field};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntervalTrigger {
    pub seconds: u64,
}

impl IntervalTrigger {
    // APScheduler turns a zero-length interval into one second.
    fn interval(self) -> Option<TimeDelta> {
        match i64::try_from(self.seconds.max(1)) {
            Ok(seconds) => TimeDelta::try_seconds(seconds),
            Err(_source) => None,
        }
    }
}

impl Trigger for IntervalTrigger {
    fn next_fire_time(
        &self,
        previous_fire_time: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Option<DateTime<Utc>> {
        previous_fire_time
            .unwrap_or(now)
            .checked_add_signed(self.interval()?)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CronZone {
    Utc,
    Local,
}

#[derive(Debug, Clone)]
pub struct CronTrigger {
    cron: Cron,
    zone: CronZone,
}

impl CronTrigger {
    pub fn from_crontab(text: &str, zone: CronZone) -> Result<Self, SchedulerError> {
        let fields: Vec<&str> = text.split_whitespace().collect();
        let count = fields.len();
        if count != 5 {
            return Err(SchedulerError::Crontab { got: count });
        }
        let [
            minute_text,
            hour_text,
            day_text,
            month_text,
            day_of_week_text,
        ] = <[&str; 5]>::try_from(fields.as_slice())
            .map_err(|_source| SchedulerError::Crontab { got: count })?;
        let minute = normalize_field(CronField::Minute, minute_text)?;
        let hour = normalize_field(CronField::Hour, hour_text)?;
        let day = normalize_field(CronField::DayOfMonth, day_text)?;
        let month = normalize_field(CronField::Month, month_text)?;
        let day_of_week = normalize_field(CronField::DayOfWeek, day_of_week_text)?;
        let pattern = format!("{minute} {hour} {day} {month} {day_of_week}");
        let mut cron = Cron::new(&pattern);
        cron.with_dom_and_dow();
        let parsed = cron.parse()?;
        Ok(Self { cron: parsed, zone })
    }
}

impl Trigger for CronTrigger {
    fn next_fire_time(
        &self,
        previous_fire_time: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Option<DateTime<Utc>> {
        let start = match previous_fire_time {
            Some(previous) => {
                let bumped = previous.checked_add_signed(TimeDelta::microseconds(1))?;
                if now < bumped && now != previous {
                    now
                } else {
                    bumped
                }
            }
            None => now,
        };
        // APScheduler rounds the search start up to a whole second, which croner's search then includes.
        let nanos = start.timestamp_subsec_nanos();
        let rounded = if nanos == 0 {
            start
        } else {
            let rest = 1_000_000_000_u32.checked_sub(nanos)?;
            start.checked_add_signed(TimeDelta::nanoseconds(i64::from(rest)))?
        };
        match self.zone {
            CronZone::Utc => search(&self.cron, &rounded),
            CronZone::Local => local_search(&self.cron, rounded),
        }
    }
}

fn search<Tz>(cron: &Cron, start: &DateTime<Tz>) -> Option<DateTime<Tz>>
where
    Tz: TimeZone,
{
    match cron.find_next_occurrence(start, true) {
        Ok(found) => Some(found),
        Err(error) => {
            tracing::debug!(%error, "cron trigger has no next fire time");
            None
        }
    }
}

// A search from the first pass of a repeated hour is also run from the second, and the earlier answer wins.
fn local_search(cron: &Cron, start: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let local_start = start.with_timezone(&Local);
    let first_found = search(cron, &local_start)?;
    let first = resolve_local(first_found.naive_local(), start)
        .unwrap_or_else(|| first_found.with_timezone(&Utc));
    let LocalResult::Ambiguous(one, two) = Local.from_local_datetime(&local_start.naive_local())
    else {
        return Some(first);
    };
    let early_instant = one.with_timezone(&Utc).min(two.with_timezone(&Utc));
    if start != early_instant {
        return Some(first);
    }
    let transition = fold_transition(start, one.with_timezone(&Utc).max(two.with_timezone(&Utc)))?;
    let second_found = search(cron, &transition.with_timezone(&Local))?;
    let second = resolve_local(second_found.naive_local(), transition)
        .unwrap_or_else(|| second_found.with_timezone(&Utc));
    Some(first.min(second))
}

fn resolve_local(naive: NaiveDateTime, floor: DateTime<Utc>) -> Option<DateTime<Utc>> {
    match Local.from_local_datetime(&naive) {
        LocalResult::Single(value) => Some(value.with_timezone(&Utc)),
        // Drop the spurious side chrono reports for the wall time that ends a repeated hour.
        LocalResult::Ambiguous(one, two) => {
            let first = one.with_timezone(&Utc);
            let second = two.with_timezone(&Utc);
            [first, second]
                .into_iter()
                .filter(|instant| instant.with_timezone(&Local).naive_local() == naive)
                .filter(|instant| *instant >= floor)
                .min()
        }
        LocalResult::None => None,
    }
}

// The first instant in (start, end] whose UTC offset differs from the start's: where a repeated hour begins.
fn fold_transition(start: DateTime<Utc>, end: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let initial = local_offset_seconds(start);
    if local_offset_seconds(end) == initial {
        return None;
    }
    let mut low = start.timestamp();
    let mut high = end.timestamp();
    while high.saturating_sub(low) > 1 {
        let span = high.saturating_sub(low);
        let middle = DateTime::from_timestamp(low.saturating_add(span.div_euclid(2)), 0)?;
        if local_offset_seconds(middle) == initial {
            low = middle.timestamp();
        } else {
            high = middle.timestamp();
        }
    }
    DateTime::from_timestamp(high, 0)
}

fn local_offset_seconds(time: DateTime<Utc>) -> i32 {
    Local
        .offset_from_utc_datetime(&time.naive_utc())
        .local_minus_utc()
}

// A boxed Trigger hides its type, so an interval is recognized by both its answers landing one fixed step out.
pub(crate) fn interval_seconds(trigger: &dyn Trigger, now: DateTime<Utc>) -> Option<u32> {
    let first = trigger.next_fire_time(None, now)?;
    let period = first.signed_duration_since(now);
    if period <= TimeDelta::zero() {
        return None;
    }
    let later = now.checked_add_signed(TimeDelta::seconds(1))?;
    let shifted = trigger.next_fire_time(None, later)?;
    if shifted.signed_duration_since(later) != period {
        return None;
    }
    u32::try_from(period.num_seconds()).ok()
}

mod tests;
