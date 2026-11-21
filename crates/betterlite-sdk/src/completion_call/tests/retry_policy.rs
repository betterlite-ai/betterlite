use std::time::Duration;

use betterlite_models::decisions::Json;
use serde_json::Number;

use super::support::{FakeSleeper, RecordingTarget, request};
use super::{body_text, zero_retries};
use crate::completion_call::retry::{RetryStrategy, with_retries};

#[tokio::test]
async fn retry_defaults_to_three_attempts_and_zeroes_inner_retries() {
    let target = RecordingTarget::always_failing();
    let sleeper = FakeSleeper::default();
    let call = request("{}");
    let error = with_retries(&target, call, None, &sleeper)
        .await
        .unwrap_err();
    let attempts = target.call_map();
    assert_eq!(attempts.len(), 3);
    assert!(zero_retries(attempts[0].get("num_retries")));
    assert!(zero_retries(attempts[0].get("max_retries")));
    assert!(zero_retries(attempts[1].get("num_retries")));
    assert!(zero_retries(attempts[1].get("max_retries")));
    assert!(zero_retries(attempts[2].get("num_retries")));
    assert!(zero_retries(attempts[2].get("max_retries")));
    assert_eq!(error.to_string(), "first down");
    assert_eq!(
        sleeper.slept(),
        vec![Duration::ZERO, Duration::ZERO],
        "constant retry waits nothing"
    );
}

#[tokio::test]
async fn retry_exponential_waits_double_and_cap_at_ten_seconds() {
    let target = RecordingTarget::always_failing();
    let sleeper = FakeSleeper::default();
    let call = request(r#"{"num_retries":7,"retry_strategy":"exponential_backoff_retry"}"#);
    with_retries(&target, call, None, &sleeper)
        .await
        .unwrap_err();
    assert_eq!(target.call_map().len(), 7);
    assert_eq!(
        sleeper.slept(),
        vec![
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4),
            Duration::from_secs(8),
            Duration::from_secs(10),
            Duration::from_secs(10),
        ]
    );
}

#[tokio::test]
async fn retry_stops_on_the_first_answer() {
    let target = RecordingTarget::failing_then_ok(1);
    let sleeper = FakeSleeper::default();
    let call = request(r#"{"num_retries":5,"retry_strategy":"exponential_backoff_retry"}"#);
    let outcome = with_retries(&target, call, None, &sleeper).await.unwrap();
    assert_eq!(target.call_map().len(), 2);
    assert!(body_text(&outcome).is_some());
    assert_eq!(sleeper.slept(), vec![Duration::from_secs(1)]);
}

#[tokio::test]
async fn retry_one_means_a_single_call() {
    let target = RecordingTarget::always_failing();
    let sleeper = FakeSleeper::default();
    let call = request(r#"{"num_retries":1}"#);
    let error = with_retries(&target, call, None, &sleeper)
        .await
        .unwrap_err();
    assert_eq!(target.call_map().len(), 1);
    assert_eq!(error.to_string(), "first down");
}

#[test]
fn retry_strategy_rejects_a_non_text_value() {
    let error = RetryStrategy::parse(Some(&Json::Number(Number::from(5_i64)))).unwrap_err();
    assert!(error.to_string().contains("retry_strategy"));
    assert_eq!(
        RetryStrategy::parse(Some(&Json::Text(String::from("constant_retry")))).unwrap(),
        RetryStrategy::Constant
    );
}
