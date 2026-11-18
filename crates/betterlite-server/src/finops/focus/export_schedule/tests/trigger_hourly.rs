#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::finops::focus::export_schedule::{ScheduleTrigger, build_scheduler_trigger};
use crate::finops::focus::support::{self, Recorder};

#[rstest]
#[case::mid_range(5, 5)]
#[case::zero(0, 0)]
#[case::negative(-10, 0)]
#[case::last_minute(59, 59)]
#[case::above_the_last_minute(90, 59)]
fn the_hourly_trigger_uses_the_offset_as_its_minute(
    #[case] offset: i64,
    #[case] expected_minute: i64,
) {
    let trigger = build_scheduler_trigger(&support::configured("hourly", Some(offset), None))
        .expect("hourly is supported");
    assert_eq!(
        trigger,
        ScheduleTrigger::Cron {
            hour: None,
            minute: expected_minute,
            second: 0,
        }
    );
}

#[test]
fn an_upper_case_frequency_is_scheduled_lowercased() {
    let recorder = Recorder::new(false);
    let mut params = support::params();
    params.frequency = Some("HOURLY".to_owned());
    params.cron_offset_minute = Some(7);
    let logger = support::build(&BTreeMap::new(), params, recorder.factory(), None);
    let trigger = build_scheduler_trigger(&logger).expect("hourly is supported");
    assert_eq!(
        trigger,
        ScheduleTrigger::Cron {
            hour: None,
            minute: 7,
            second: 0,
        }
    );
}
