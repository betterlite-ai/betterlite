#![cfg(test)]

use std::time::Duration;

use betterlite_config::sections::mock_response::failure::MockError;
use rstest::rstest;

use super::{TimeoutValue, delay_duration, timeout_duration};

#[rstest]
#[case::number(TimeoutValue::Seconds(1.5), Duration::from_millis(1500))]
#[case::numeric_string(TimeoutValue::Text(String::from("0.25")), Duration::from_millis(250))]
fn mock_calls_behavior_tests_behavior(#[case] timeout: TimeoutValue, #[case] expected: Duration) {
    assert_eq!(timeout_duration(&timeout), Ok(expected));
}

#[rstest]
#[case::not_a_number(TimeoutValue::Text(String::from("soon")))]
#[case::negative(TimeoutValue::Seconds(-1.0))]
fn a_timeout_that_cannot_be_waited_is_a_typed_failure(#[case] timeout: TimeoutValue) {
    assert_eq!(
        timeout_duration(&timeout),
        Err(MockError::WrongType { field: "timeout" })
    );
}

#[test]
fn no_mock_delay_waits_nothing() {
    assert_eq!(delay_duration(None), Ok(Duration::ZERO));
    assert_eq!(
        delay_duration(Some(-0.5_f64)),
        Err(MockError::WrongType {
            field: "mock_delay"
        })
    );
}
