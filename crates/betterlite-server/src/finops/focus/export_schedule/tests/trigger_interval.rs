#![cfg(test)]

use rstest::rstest;

use crate::finops::focus::export_schedule::{ScheduleTrigger, build_scheduler_trigger};
use crate::finops::focus::support;

#[rstest]
#[case::set(Some(120), 120)]
#[case::unset(None, 60)]
#[case::explicit_zero(Some(0), 0)]
#[case::single_second(Some(1), 1)]
fn the_interval_trigger_uses_the_configured_seconds(
    #[case] interval_seconds: Option<i64>,
    #[case] expected_seconds: i64,
) {
    let trigger = build_scheduler_trigger(&support::configured("interval", None, interval_seconds))
        .expect("interval is supported");
    assert_eq!(
        trigger,
        ScheduleTrigger::Interval {
            seconds: expected_seconds
        }
    );
}
