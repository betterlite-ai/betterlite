#![cfg(test)]

use crate::finops::focus::error::ScheduleError;
use crate::finops::focus::export_schedule::compute_time_window;
use crate::finops::focus::support::{self, FIXED_NOW, at};

#[test]
fn a_frequency_outside_the_three_has_no_window() {
    let error = compute_time_window(&support::configured("weekly", None, None), at(FIXED_NOW))
        .expect_err("weekly has no time window");
    assert!(
        matches!(&error, ScheduleError::UnsupportedFrequency { frequency } if frequency == "weekly")
    );
    assert_eq!(error.to_string(), "Unsupported frequency: weekly");
}
