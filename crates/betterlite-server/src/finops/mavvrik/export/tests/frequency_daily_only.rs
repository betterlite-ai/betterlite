#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::finops::mavvrik::support::{Recorder, build, environment, try_build};

#[rstest]
#[case::hourly("hourly")]
#[case::mixed_case("Hourly")]
#[case::interval("interval")]
#[case::blank("   ")]
fn a_frequency_other_than_daily_is_refused(#[case] value: &str) {
    let error = try_build(
        &environment(&[("MAVVRIK_FOCUS_FREQUENCY", value)]),
        &Recorder::default(),
        None,
    )
    .err()
    .expect("a frequency other than daily is refused");
    assert!(error.to_string().contains("is not supported"));
    assert!(error.to_string().contains("one file per calendar date"));
}

#[test]
fn the_refusal_keeps_the_lowercased_value() {
    let error = try_build(
        &environment(&[("MAVVRIK_FOCUS_FREQUENCY", "HOURLY")]),
        &Recorder::default(),
        None,
    )
    .err()
    .expect("a frequency other than daily is refused");
    assert!(
        error
            .to_string()
            .contains("MAVVRIK_FOCUS_FREQUENCY='hourly'")
    );
}

#[rstest]
#[case::unset(None)]
#[case::empty(Some(""))]
#[case::set(Some("DAILY"))]
#[case::mixed(Some("Daily"))]
fn daily_is_accepted_in_any_case_and_the_logger_keeps_it_lowercased(#[case] value: Option<&str>) {
    let environment = value.map_or_else(BTreeMap::new, |value| {
        environment(&[("MAVVRIK_FOCUS_FREQUENCY", value)])
    });
    let logger = build(&environment, &Recorder::default(), None);
    assert_eq!(logger.frequency, "daily");
}

#[test]
fn a_refused_frequency_is_named_before_the_inherited_settings_are_read() {
    let error = try_build(
        &environment(&[
            ("MAVVRIK_FOCUS_FREQUENCY", "hourly"),
            ("FOCUS_CRON_OFFSET", "not-a-number"),
        ]),
        &Recorder::default(),
        None,
    )
    .err()
    .expect("the frequency check fails before the cron offset is parsed");
    assert!(error.to_string().contains("is not supported"));
}
