#![cfg(test)]

use std::time::Duration;

use rstest::rstest;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::straiker::delivery::fixtures::{
    Fakes, caps, delivery, inputs, network_error, raw, request_data, run, sent, settings,
};
use crate::straiker::delivery::retry::{RandomDelay, RetryDelay, backoff_cap};

#[rstest]
#[case::first(0.1, 2.0, 0, Duration::from_millis(100))]
#[case::second(0.1, 2.0, 1, Duration::from_millis(200))]
#[case::third(0.1, 2.0, 2, Duration::from_millis(400))]
#[case::capped(0.1, 2.0, 5, Duration::from_secs(2))]
#[case::above_the_ceiling(1.0, 1.5, 1, Duration::from_millis(1500))]
#[case::zero_initial(0.0, 2.0, 3, Duration::ZERO)]
#[case::negative_attempt(0.1, 2.0, -1, Duration::from_millis(50))]
#[case::huge_attempt(0.1, 2.0, i64::MAX, Duration::from_secs(2))]
fn the_backoff_cap_doubles_until_the_ceiling(
    #[case] initial: f64,
    #[case] max: f64,
    #[case] attempt: i64,
    #[case] expected: Duration,
) {
    assert_eq!(backoff_cap(initial, max, attempt), expected);
}

#[rstest]
#[case::no_retry(0, 1, 0)]
#[case::one_retry(1, 2, 1)]
#[case::two_retries(2, 3, 2)]
#[case::five_retries(5, 6, 5)]
#[tokio::test]
async fn the_attempt_count_is_the_retry_count_plus_one(
    #[case] retries: i64,
    #[case] expected_sent: usize,
    #[case] expected_caps: usize,
) {
    let fakes = Fakes::default();
    let mut args = raw();
    args.max_retries = Some(retries);
    let handler = delivery(settings(&args), vec![network_error(); expected_sent]);
    let mut data = request_data(r#"{"model":"gpt-4o"}"#);
    let call = fakes.call(Some("completion"), None);
    run(
        &handler,
        &mut data,
        inputs(&["hi"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("every attempt fails");
    assert_eq!(sent(&handler).len(), expected_sent);
    assert_eq!(caps(&handler).len(), expected_caps);
}

#[tokio::test]
async fn the_configured_backoffs_are_used() {
    let fakes = Fakes::default();
    let mut args = raw();
    args.initial_backoff = Some(0.5);
    args.max_backoff = Some(1.0);
    let handler = delivery(
        settings(&args),
        vec![network_error(), network_error(), network_error()],
    );
    let mut data = request_data(r#"{"model":"gpt-4o"}"#);
    let call = fakes.call(Some("completion"), None);
    run(
        &handler,
        &mut data,
        inputs(&["hi"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("every attempt fails");
    assert_eq!(
        caps(&handler),
        vec![Duration::from_millis(500), Duration::from_secs(1)]
    );
}

#[rstest]
#[case::default(None, Duration::from_secs(5))]
#[case::configured(Some(7.5), Duration::from_millis(7500))]
#[tokio::test]
async fn every_attempt_carries_the_configured_timeout(
    #[case] timeout: Option<f64>,
    #[case] expected: Duration,
) {
    let fakes = Fakes::default();
    let mut args = raw();
    args.timeout = timeout;
    let handler = delivery(settings(&args), vec![network_error(), network_error()]);
    let mut data = request_data(r#"{"model":"gpt-4o"}"#);
    let call = fakes.call(Some("completion"), None);
    run(
        &handler,
        &mut data,
        inputs(&["hi"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("both attempts fail");
    assert!(sent(&handler).iter().all(|post| post.timeout == expected));
}

#[test]
fn the_random_delay_stays_within_the_cap() {
    let delay = RandomDelay::try_new().unwrap();
    let cap = Duration::from_millis(10);
    assert!((0..1000).all(|_| delay.delay(cap) <= cap));
    assert!((0..1000).any(|_| delay.delay(cap) > Duration::ZERO));
}

#[test]
fn a_zero_cap_draws_no_wait() {
    let delay = RandomDelay::try_new().unwrap();
    assert_eq!(delay.delay(Duration::ZERO), Duration::ZERO);
}
