#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::str;

use chrono::{DateTime, Utc};
use rstest::rstest;

use crate::scheduled_job_stagger::Trigger as _;

use super::{CronTrigger, CronZone, IntervalTrigger, interval_seconds};
use crate::job_scheduler::SchedulerError;

fn instant(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap()
        .with_timezone(&Utc)
}

#[test]
fn interval_first_fire_is_the_answer_time_plus_the_interval() {
    let now = instant("2026-01-01T00:00:00Z");
    let trigger = IntervalTrigger { seconds: 60 };
    assert_eq!(
        trigger.next_fire_time(None, now),
        Some(instant("2026-01-01T00:01:00Z"))
    );
    let previous = instant("2026-01-01T00:01:00Z");
    assert_eq!(
        trigger.next_fire_time(Some(previous), now),
        Some(instant("2026-01-01T00:02:00Z"))
    );
}

#[test]
fn interval_zero_seconds_acts_as_one_second() {
    let now = instant("2026-01-01T00:00:00Z");
    let trigger = IntervalTrigger { seconds: 0 };
    assert_eq!(
        trigger.next_fire_time(None, now),
        Some(instant("2026-01-01T00:00:01Z"))
    );
}

#[test]
fn cron_daily_utc_fires_at_the_requested_wall_clock_time() {
    let trigger = CronTrigger::from_crontab("15 0 * * *", CronZone::Utc).unwrap();
    let start = instant("2026-01-01T00:00:00Z");
    let first = trigger.next_fire_time(None, start).unwrap();
    assert_eq!(first, instant("2026-01-01T00:15:00Z"));
    let second = trigger.next_fire_time(Some(first), first).unwrap();
    assert_eq!(second, instant("2026-01-02T00:15:00Z"));
}

#[test]
fn cron_weekday_zero_is_monday() {
    let trigger = CronTrigger::from_crontab("0 3 * * 0", CronZone::Utc).unwrap();
    let start = instant("2026-01-01T00:00:00Z");
    let first = trigger.next_fire_time(None, start).unwrap();
    assert_eq!(first, instant("2026-01-05T03:00:00Z"));
    let second = trigger.next_fire_time(Some(first), first).unwrap();
    assert_eq!(second, instant("2026-01-12T03:00:00Z"));
}

#[test]
fn cron_step_and_range_match_apscheduler() {
    let trigger = CronTrigger::from_crontab("0-30/10 9-11 * * 1-5", CronZone::Utc).unwrap();
    let start = instant("2026-01-01T00:00:00Z");
    let first = trigger.next_fire_time(None, start).unwrap();
    assert_eq!(first, instant("2026-01-01T09:00:00Z"));
    let second = trigger.next_fire_time(Some(first), first).unwrap();
    assert_eq!(second, instant("2026-01-01T09:10:00Z"));
    let third = trigger.next_fire_time(Some(second), second).unwrap();
    assert_eq!(third, instant("2026-01-01T09:20:00Z"));
}

#[test]
fn cron_minute_steps_fire_on_the_step_grid() {
    let trigger = CronTrigger::from_crontab("*/15 * * * *", CronZone::Utc).unwrap();
    let start = instant("2026-01-01T00:07:30Z");
    assert_eq!(
        trigger.next_fire_time(None, start),
        Some(instant("2026-01-01T00:15:00Z"))
    );
}

#[test]
fn cron_named_weekdays_and_months_match() {
    let trigger = CronTrigger::from_crontab("0 9 * * mon-fri", CronZone::Utc).unwrap();
    let start = instant("2026-01-03T00:00:00Z");
    assert_eq!(
        trigger.next_fire_time(None, start),
        Some(instant("2026-01-05T09:00:00Z"))
    );
    let monthly = CronTrigger::from_crontab("0 0 1 jan-mar *", CronZone::Utc).unwrap();
    assert_eq!(
        monthly.next_fire_time(None, start),
        Some(instant("2026-02-01T00:00:00Z"))
    );
}

#[test]
fn cron_local_zone_produces_a_fire_time() {
    let trigger = CronTrigger::from_crontab("15 0 * * *", CronZone::Local).unwrap();
    let start = instant("2026-01-01T00:00:00Z");
    assert!(trigger.next_fire_time(None, start).is_some());
}

#[rstest]
#[case::six_fields("0 0 3 * * *", "Wrong number of fields; got 6, expected 5")]
#[case::four_fields("0 3 * *", "Wrong number of fields; got 4, expected 5")]
#[case::empty("", "Wrong number of fields; got 0, expected 5")]
fn cron_requires_five_fields(#[case] text: &str, #[case] message: &str) {
    let error = CronTrigger::from_crontab(text, CronZone::Utc).unwrap_err();
    assert_eq!(error.to_string(), message);
    assert!(matches!(error, SchedulerError::Crontab { .. }));
}

#[test]
fn cron_weekday_seven_is_rejected() {
    let error = CronTrigger::from_crontab("0 3 * * 7", CronZone::Utc).unwrap_err();
    assert!(matches!(error, SchedulerError::DayOfWeek { value: 7 }));
}

#[test]
fn interval_seconds_recognizes_an_interval_trigger() {
    let now = instant("2026-01-01T00:00:00Z");
    let trigger = IntervalTrigger { seconds: 90 };
    assert_eq!(interval_seconds(&trigger, now), Some(90));
}

#[test]
fn interval_seconds_does_not_take_a_cron_trigger_for_an_interval() {
    let now = instant("2026-01-01T00:00:00Z");
    let trigger = CronTrigger::from_crontab("15 0 * * *", CronZone::Utc).unwrap();
    assert_eq!(interval_seconds(&trigger, now), None);
}

#[rstest]
fn replays_crontab_trigger_corpus(#[files("fuzz/corpus/crontab_trigger/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let Ok(text) = str::from_utf8(&bytes) else {
        return;
    };
    if let Ok(trigger) = CronTrigger::from_crontab(text, CronZone::Utc) {
        let _next = trigger.next_fire_time(None, DateTime::UNIX_EPOCH);
    }
}
