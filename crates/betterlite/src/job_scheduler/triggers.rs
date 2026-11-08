use chrono::{DateTime, Local, TimeDelta, Utc};
use croner::Cron;

use crate::scheduled_job_stagger::Trigger;

use super::SchedulerError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntervalTrigger {
    pub seconds: u64,
}

impl Trigger for IntervalTrigger {
    fn next_fire_time(
        &self,
        previous_fire_time: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Option<DateTime<Utc>> {
        // APScheduler turns a zero-length interval into one second.
        let seconds = if self.seconds == 0 { 1 } else { self.seconds };
        let interval = TimeDelta::try_seconds(i64::try_from(seconds).ok()?)?;
        previous_fire_time.map_or_else(
            || now.checked_add_signed(interval),
            |previous| previous.checked_add_signed(interval),
        )
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
        let [minute, hour, day, month, day_of_week_text] = <[&str; 5]>::try_from(fields.as_slice())
            .map_err(|_source| SchedulerError::Crontab { got: count })?;
        let day_of_week = translate_day_of_week(day_of_week_text)?;
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
        let start = ceil_to_second(search_start(previous_fire_time, now)?)?;
        match self.zone {
            CronZone::Utc => match self.cron.find_next_occurrence(&start, true) {
                Ok(next) => Some(next),
                Err(error) => {
                    tracing::debug!(%error, "cron trigger has no next fire time");
                    None
                }
            },
            CronZone::Local => {
                let local = start.with_timezone(&Local);
                match self.cron.find_next_occurrence(&local, true) {
                    Ok(next) => Some(next.with_timezone(&Utc)),
                    Err(error) => {
                        tracing::debug!(%error, "cron trigger has no next fire time");
                        None
                    }
                }
            }
        }
    }
}

// APScheduler searches from one microsecond past the previous fire, or from now when that is later.
fn search_start(
    previous_fire_time: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let Some(previous) = previous_fire_time else {
        return Some(now);
    };
    let bumped = previous.checked_add_signed(TimeDelta::microseconds(1))?;
    if now < bumped {
        if now == previous {
            Some(bumped)
        } else {
            Some(now)
        }
    } else {
        Some(bumped)
    }
}

// APScheduler rounds the search start up to a whole second, which croner's search then includes.
fn ceil_to_second(time: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let nanos = time.timestamp_subsec_nanos();
    if nanos == 0 {
        return Some(time);
    }
    let rest = 1_000_000_000_u32.checked_sub(nanos)?;
    time.checked_add_signed(TimeDelta::nanoseconds(i64::from(rest)))
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

// APScheduler numbers day_of_week 0=Monday; croner reads 0 as Sunday, so numbers become weekday names.
fn translate_day_of_week(field: &str) -> Result<String, SchedulerError> {
    let mut translated = Vec::new();
    for element in field.split(',') {
        translated.push(translate_day_element(element)?);
    }
    Ok(translated.join(","))
}

fn translate_day_element(element: &str) -> Result<String, SchedulerError> {
    let (base, step_text) = match element.split_once('/') {
        Some((base, step_part)) => (base, Some(step_part)),
        None => (element, None),
    };
    let step = match step_text {
        Some(text) => match text.parse::<u32>() {
            Ok(width) => Some(width),
            Err(_source) => return Ok(element.to_owned()),
        },
        None => None,
    };
    // croner rejects a zero step itself, with its own message.
    if step == Some(0) {
        return Ok(element.to_owned());
    }
    if base == "*" {
        return step.map_or_else(|| Ok("*".to_owned()), |width| weekday_list(0, 6, width));
    }
    if let Some((first_text, last_text)) = base.split_once('-') {
        return match (weekday_number(first_text)?, weekday_number(last_text)?) {
            (Some(first), Some(last)) => translate_day_range(element, first, last, step),
            _ => Ok(element.to_owned()),
        };
    }
    match (weekday_number(base)?, step) {
        (Some(value), None) => Ok(weekday_name(value)?.to_owned()),
        (Some(value), Some(width)) => weekday_list(value, 6, width),
        (None, _) => Ok(element.to_owned()),
    }
}

fn translate_day_range(
    element: &str,
    first: u32,
    last: u32,
    step: Option<u32>,
) -> Result<String, SchedulerError> {
    match step {
        Some(width) if first <= last => weekday_list(first, last, width),
        Some(_width) => Ok(element.to_owned()),
        None if first == last => Ok(weekday_name(first)?.to_owned()),
        None => Ok(format!("{}-{}", weekday_name(first)?, weekday_name(last)?)),
    }
}

fn weekday_number(text: &str) -> Result<Option<u32>, SchedulerError> {
    match text.parse::<u32>() {
        Ok(value) if value <= 6 => Ok(Some(value)),
        Ok(value) => Err(SchedulerError::DayOfWeek {
            value: u64::from(value),
        }),
        Err(_source) => Ok(None),
    }
}

fn weekday_name(value: u32) -> Result<&'static str, SchedulerError> {
    match value {
        0 => Ok("mon"),
        1 => Ok("tue"),
        2 => Ok("wed"),
        3 => Ok("thu"),
        4 => Ok("fri"),
        5 => Ok("sat"),
        6 => Ok("sun"),
        other => Err(SchedulerError::DayOfWeek {
            value: u64::from(other),
        }),
    }
}

fn weekday_list(start: u32, end: u32, step: u32) -> Result<String, SchedulerError> {
    let range = end.saturating_sub(start);
    if step > range {
        return Err(SchedulerError::CronStep {
            step: u64::from(step),
            range: u64::from(range),
        });
    }
    let mut names = Vec::new();
    let mut value = start;
    while value <= end {
        names.push(weekday_name(value)?);
        value = value.saturating_add(step);
    }
    Ok(names.join(","))
}

mod tests;
