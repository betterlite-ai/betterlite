use betterlite_config::sections::scheduled_job_stagger::Settings as StaggerSettings;
use betterlite_config::sections::settings::JsonValue;
use betterlite_core::duration::seconds::duration_in_seconds;
use chrono::{DateTime, TimeDelta, Utc};
use jiff::tz::TimeZoneDatabase;

use crate::job_scheduler::triggers::{CronTrigger, CronZone, IntervalTrigger};
use crate::scheduled_job_stagger::Trigger;
use crate::scheduled_job_stagger::apply::stagger_trigger;

use super::SPEND_LOG_CLEANUP_JOB_ID;
use super::ports::GeneralSettingsView;
use super::schedule::{CleanupSchedule, cleanup_schedule_of, value_text};

// APScheduler builds its start date as datetime.now() + interval, so a first fire past year 9999 raises OverflowError
const PYTHON_DATETIME_MAX_SECONDS: i64 = 253_402_300_799;

#[derive(Clone, Copy)]
pub struct ScheduleContext<'context> {
    pub stagger: &'context StaggerSettings,
    pub identity: &'context str,
    pub now: DateTime<Utc>,
    pub timezone: Option<&'context str>,
    pub database: &'context TimeZoneDatabase,
}

pub(super) struct ParsedInterval<'schedule> {
    pub(super) text: &'schedule str,
    pub(super) seconds: u64,
    pub(super) period_seconds: Option<u32>,
}

// An absent or null cron leaves the interval branch; any other value is used as given, since docs/compatibility.md rules out Python truthiness
#[must_use]
pub fn cleanup_trigger(
    settings: &dyn GeneralSettingsView,
    context: &ScheduleContext<'_>,
) -> Option<Box<dyn Trigger>> {
    let schedule = cleanup_schedule_of(settings);
    let Some(cron) = schedule.cleanup_cron.as_ref() else {
        return interval_trigger(&schedule, context);
    };
    let Some(text) = cron.as_str() else {
        tracing::error!(
            "Invalid maximum_spend_logs_cleanup_cron value: {}",
            value_text(cron)
        );
        return None;
    };
    match CronTrigger::from_crontab(text, CronZone::Local) {
        Ok(trigger) => Some(Box::new(trigger)),
        Err(_error) => {
            tracing::error!("Invalid maximum_spend_logs_cleanup_cron value: {text}");
            None
        }
    }
}

fn interval_trigger(
    schedule: &CleanupSchedule,
    context: &ScheduleContext<'_>,
) -> Option<Box<dyn Trigger>> {
    let ParsedInterval {
        seconds,
        period_seconds,
        ..
    } = parsed_interval(schedule, context)?;
    Some(stagger_trigger(
        SPEND_LOG_CLEANUP_JOB_ID,
        Box::new(IntervalTrigger { seconds }),
        period_seconds,
        context.stagger,
        context.identity,
    ))
}

pub(super) fn trigger_text(schedule: &CleanupSchedule) -> String {
    match schedule.cleanup_cron.as_ref() {
        Some(JsonValue::String(cron)) => format!("cron {cron}"),
        Some(value) => value_text(value),
        None => match schedule.cleanup_interval.as_ref() {
            Some(JsonValue::String(text)) => format!("interval of {text}"),
            Some(value) => value_text(value),
            None => "interval of 1d".to_owned(),
        },
    }
}

// APScheduler's IntervalTrigger raises OverflowError at construction for a first fire past datetime.max
#[must_use]
pub(super) fn interval_past_datetime_max(now: DateTime<Utc>, seconds: u64) -> bool {
    let Ok(signed_seconds) = i64::try_from(seconds) else {
        return true;
    };
    let Some(delta) = TimeDelta::try_seconds(signed_seconds) else {
        return true;
    };
    now.checked_add_signed(delta)
        .is_none_or(|fire| fire.timestamp() > PYTHON_DATETIME_MAX_SECONDS)
}

pub(super) fn parsed_interval<'schedule>(
    schedule: &'schedule CleanupSchedule,
    context: &ScheduleContext<'_>,
) -> Option<ParsedInterval<'schedule>> {
    let text = match schedule.cleanup_interval.as_ref() {
        None => "1d",
        Some(JsonValue::String(text)) => text.as_str(),
        Some(value) => {
            tracing::error!(
                "Invalid maximum_spend_logs_retention_interval value: {}",
                value_text(value)
            );
            return None;
        }
    };
    let Ok(parsed) = duration_in_seconds(text, context.now, context.timezone, context.database)
    else {
        tracing::error!("Invalid maximum_spend_logs_retention_interval value: {text:?}");
        return None;
    };
    let Ok(seconds) = u64::try_from(parsed.seconds) else {
        tracing::error!("Invalid maximum_spend_logs_retention_interval value: {text:?}");
        return None;
    };
    if interval_past_datetime_max(context.now, seconds) {
        tracing::error!("Invalid maximum_spend_logs_retention_interval value: {text:?}");
        return None;
    }
    Some(ParsedInterval {
        text,
        seconds,
        period_seconds: u32::try_from(parsed.seconds).ok(),
    })
}
