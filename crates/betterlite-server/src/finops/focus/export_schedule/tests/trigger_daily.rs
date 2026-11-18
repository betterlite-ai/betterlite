#![cfg(test)]

use rstest::rstest;

use crate::finops::focus::export_schedule::{ScheduleTrigger, build_scheduler_trigger};
use crate::finops::focus::support;

#[rstest]
#[case::under_an_hour(5, Some(0), 5)]
#[case::zero(0, Some(0), 0)]
#[case::exactly_one_hour(60, Some(1), 0)]
#[case::hour_and_a_half(90, Some(1), 30)]
#[case::negative(-5, Some(0), 0)]
#[case::last_minute_of_the_day(1439, Some(23), 59)]
#[case::one_full_day(1440, Some(23), 0)]
#[case::far_beyond_a_day(9999, Some(23), 39)]
fn the_daily_trigger_splits_the_offset_into_hour_and_minute(
    #[case] offset: i64,
    #[case] expected_hour: Option<i64>,
    #[case] expected_minute: i64,
) {
    let trigger = build_scheduler_trigger(&support::configured("daily", Some(offset), None))
        .expect("daily is supported");
    assert_eq!(
        trigger,
        ScheduleTrigger::Cron {
            hour: expected_hour,
            minute: expected_minute,
            second: 0,
        }
    );
}
