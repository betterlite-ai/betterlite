#![cfg(test)]

use crate::finops::focus::error::ScheduleError;
use crate::finops::focus::support;

#[test]
fn a_frequency_outside_the_three_is_refused() {
    let error = support::configured("Weekly", Some(5), None)
        .build_scheduler_trigger()
        .expect_err("weekly has no trigger");
    assert!(
        matches!(&error, ScheduleError::UnsupportedFrequency { frequency } if frequency == "weekly")
    );
    assert_eq!(error.to_string(), "Unsupported frequency: weekly");
}

#[test]
fn an_empty_frequency_is_used_as_given_and_refused() {
    let error = support::configured("", Some(5), None)
        .build_scheduler_trigger()
        .expect_err("an empty frequency has no trigger");
    assert_eq!(error.to_string(), "Unsupported frequency: ");
}
