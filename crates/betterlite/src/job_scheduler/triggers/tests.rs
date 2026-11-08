#![cfg(test)]

use std::env::current_exe;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::str;

use chrono::{DateTime, Utc};
use rstest::rstest;

use crate::scheduled_job_stagger::Trigger as _;

use super::{CronTrigger, CronZone, IntervalTrigger, interval_seconds, local_offset_seconds};
use crate::job_scheduler::SchedulerError;

const LOCAL_ZONE_CASE: &str =
    "job_scheduler::triggers::tests::cron_local_zone_fires_on_apscheduler_instants";
const LOCAL_ZONE_MARKER: &str = "b8370-local-zone-ok";

fn instant(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap()
        .with_timezone(&Utc)
}

fn first_fires(trigger: &CronTrigger, start: DateTime<Utc>, count: usize) -> Vec<DateTime<Utc>> {
    let mut times = Vec::new();
    let mut previous = None;
    while times.len() < count {
        let now = previous.unwrap_or(start);
        let Some(next) = trigger.next_fire_time(previous, now) else {
            break;
        };
        times.push(next);
        previous = Some(next);
    }
    times
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

#[rstest]
#[case::six_fields("0 0 3 * * *", "Wrong number of fields; got 6, expected 5")]
#[case::four_fields("0 3 * *", "Wrong number of fields; got 4, expected 5")]
#[case::empty("", "Wrong number of fields; got 0, expected 5")]
fn cron_requires_five_fields(#[case] text: &str, #[case] message: &str) {
    let error = CronTrigger::from_crontab(text, CronZone::Utc).unwrap_err();
    assert_eq!(error.to_string(), message);
    assert!(matches!(error, SchedulerError::Crontab { .. }));
}

#[rstest]
#[case::minute_step_over_range(
    "*/60 * * * *",
    "Error validating expression '*/60': the step value (60) is higher than the total range of the expression (59)"
)]
#[case::hour_step_over_range(
    "0 */24 * * *",
    "Error validating expression '*/24': the step value (24) is higher than the total range of the expression (23)"
)]
#[case::value_step_over_range(
    "5/60 * * * *",
    "Error validating expression '5/60': the step value (60) is higher than the total range of the expression (59)"
)]
#[case::range_step_over_range(
    "10-20/30 * * * *",
    "Error validating expression '10-20/30': the step value (30) is higher than the total range of the expression (10)"
)]
#[case::empty_range_step(
    "5-5/1 * * * *",
    "Error validating expression '5-5/1': the step value (1) is higher than the total range of the expression (0)"
)]
#[case::day_step_over_range(
    "0 0 */31 * *",
    "Error validating expression '*/31': the step value (31) is higher than the total range of the expression (30)"
)]
#[case::month_step_over_range(
    "0 0 1 */12 *",
    "Error validating expression '*/12': the step value (12) is higher than the total range of the expression (11)"
)]
#[case::zero_step("*/0 * * * *", "Increment must be higher than 0")]
#[case::descending_range(
    "0 0 5-1 * *",
    "The minimum value in a range must not be higher than the maximum"
)]
#[case::descending_month_names(
    "0 0 1 mar-jan *",
    "The minimum value in a range must not be higher than the maximum"
)]
#[case::minute_too_high(
    "60 * * * *",
    "Error validating expression '60': the last value (60) is higher than the maximum value (59)"
)]
#[case::day_zero(
    "0 0 0 * *",
    "Error validating expression '0': the first value (0) is lower than the minimum value (1)"
)]
#[case::weekday_seven(
    "0 3 * * 7",
    "Error validating expression '7': the last value (7) is higher than the maximum value (6)"
)]
#[case::weekday_range_over_max(
    "0 3 * * 6-7",
    "Error validating expression '6-7': the last value (7) is higher than the maximum value (6)"
)]
#[case::question_mark("0 0 ? * *", "Unrecognized expression \"?\" for field \"day\"")]
#[case::croner_last_day("0 0 L * *", "Unrecognized expression \"L\" for field \"day\"")]
#[case::croner_nearest_weekday("0 0 15W * *", "Unrecognized expression \"15W\" for field \"day\"")]
#[case::question_mark_minute("? * * * *", "Unrecognized expression \"?\" for field \"minute\"")]
#[case::unknown_month("0 0 1 janx *", "Invalid month name \"janx\"")]
#[case::unknown_weekday("0 3 * * monx", "Invalid weekday name \"monx\"")]
fn cron_rejects_what_apscheduler_rejects(#[case] cron: &str, #[case] message: &str) {
    let error = CronTrigger::from_crontab(cron, CronZone::Utc).unwrap_err();
    assert_eq!(error.to_string(), message);
}

#[rstest]
#[case::weekday_name_with_step(
    "0 3 * * mon/2",
    ["2026-01-05T03:00:00Z", "2026-01-12T03:00:00Z", "2026-01-19T03:00:00Z"]
)]
#[case::weekday_name_range_with_step(
    "0 3 * * mon-fri/2",
    ["2026-01-01T03:00:00Z", "2026-01-02T03:00:00Z", "2026-01-05T03:00:00Z"]
)]
#[case::weekday_name_with_number(
    "0 0 * * mon-5",
    ["2026-01-05T00:00:00Z", "2026-01-12T00:00:00Z", "2026-01-19T00:00:00Z"]
)]
#[case::month_name_with_step(
    "0 0 1 jan/2 *",
    ["2026-01-01T00:00:00Z", "2027-01-01T00:00:00Z", "2028-01-01T00:00:00Z"]
)]
#[case::month_name_range_with_step(
    "0 0 1 jan-mar/2 *",
    ["2026-01-01T00:00:00Z", "2026-02-01T00:00:00Z", "2026-03-01T00:00:00Z"]
)]
fn cron_name_terms_keep_apschedulers_prefix(#[case] cron: &str, #[case] expected: [&str; 3]) {
    let trigger = CronTrigger::from_crontab(cron, CronZone::Utc).unwrap();
    let fires = first_fires(&trigger, instant("2026-01-01T00:00:00Z"), 3);
    assert_eq!(fires, expected.map(instant));
}

fn is_us_eastern() -> bool {
    local_offset_seconds(instant("2026-01-15T12:00:00Z")) == -18_000
        && local_offset_seconds(instant("2026-07-15T12:00:00Z")) == -14_400
}

#[test]
fn cron_local_zone_fires_on_apscheduler_instants() {
    if !is_us_eastern() {
        return;
    }
    let repeated = CronTrigger::from_crontab("30 1 * * *", CronZone::Local).unwrap();
    let repeated_fires = first_fires(&repeated, instant("2026-11-01T00:00:00Z"), 3);
    assert_eq!(
        repeated_fires,
        [
            instant("2026-11-01T05:30:00Z"),
            instant("2026-11-01T06:30:00Z"),
            instant("2026-11-02T06:30:00Z"),
        ]
    );
    let gap = CronTrigger::from_crontab("30 2 * * *", CronZone::Local).unwrap();
    let gap_fires = first_fires(&gap, instant("2026-03-08T00:00:00Z"), 1);
    assert_eq!(gap_fires, [instant("2026-03-08T07:00:00Z")]);
    let daily = CronTrigger::from_crontab("15 0 * * *", CronZone::Local).unwrap();
    let daily_fires = first_fires(&daily, instant("2026-03-06T00:00:00Z"), 4);
    assert_eq!(
        daily_fires,
        [
            instant("2026-03-06T05:15:00Z"),
            instant("2026-03-07T05:15:00Z"),
            instant("2026-03-08T05:15:00Z"),
            instant("2026-03-09T04:15:00Z"),
        ]
    );
    println!("{LOCAL_ZONE_MARKER}");
}

#[test]
fn cron_local_zone_is_run_under_a_fixed_zone() {
    let exe = current_exe().unwrap();
    let output = Command::new(exe)
        .arg("--exact")
        .arg(LOCAL_ZONE_CASE)
        .arg("--nocapture")
        .env("TZ", "EST5EDT,M3.2.0,M11.1.0")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "the fixed-zone child run failed: {}",
        String::from_utf8(output.stderr).unwrap()
    );
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains(LOCAL_ZONE_MARKER)
    );
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
