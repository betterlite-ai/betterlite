#![cfg(test)]

use std::time::Duration;

use betterlite_config::sections::mock_response::failure::MockError;
use rstest::rstest;
use tokio::time::Instant;

use super::{TimeoutValue, delay_duration, raise_on_timeout, sleep_delay, timeout_duration};

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

#[tokio::test(start_paused = true)]
async fn a_mock_timeout_sleeps_the_deployment_timeout_then_fails() {
    let timeout = TimeoutValue::Seconds(2.0);
    let started = Instant::now();
    let result = raise_on_timeout(Some(true), Some(&timeout)).await;
    assert_eq!(result, Err(MockError::Timeout));
    assert_eq!(started.elapsed(), Duration::from_secs(2));
}

#[rstest]
#[case::no_timeout(Some(true), None)]
#[case::disabled(Some(false), Some(TimeoutValue::Seconds(2.0)))]
#[case::unset(None, Some(TimeoutValue::Seconds(2.0)))]
#[tokio::test(start_paused = true)]
async fn a_mock_without_both_inputs_answers_normally(
    #[case] mock_timeout: Option<bool>,
    #[case] timeout: Option<TimeoutValue>,
) {
    let started = Instant::now();
    let result = raise_on_timeout(mock_timeout, timeout.as_ref()).await;
    assert_eq!(result, Ok(()));
    assert_eq!(started.elapsed(), Duration::ZERO);
}

#[tokio::test(start_paused = true)]
async fn a_mock_delay_sleeps_before_the_answer() {
    let started = Instant::now();
    assert_eq!(sleep_delay(Some(0.5_f64)).await, Ok(()));
    assert_eq!(started.elapsed(), Duration::from_millis(500));
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
