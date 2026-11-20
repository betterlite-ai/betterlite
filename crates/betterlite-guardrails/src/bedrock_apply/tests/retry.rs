use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::json;
use tokio::time::Instant;

use super::{Reply, Scripted, guardrail, input_scan, message, record_field, records_of, run};
use crate::guardrail_base::failure::GuardrailFailure;

fn throttle() -> Reply {
    Reply::failed(429, json!({ "message": "Rate exceeded" }))
}

#[tokio::test(start_paused = true)]
async fn a_throttle_retries_after_the_base_delay() {
    let messages = vec![message("user", json!("hello"))];
    let transport = Scripted::new(vec![throttle(), Reply::ok(json!({ "action": "NONE" }))]);
    let mut data = BTreeMap::new();
    let started = Instant::now();
    let outcome = run(&guardrail(), &mut data, input_scan(&messages), &transport).await;
    assert_eq!(started.elapsed(), Duration::from_millis(500));
    let response = outcome.expect("the retry passes");
    assert_eq!(response.action.as_deref(), Some("NONE"));
    assert_eq!(transport.count(), 2);
    let records = records_of(&data);
    assert_eq!(records.len(), 1);
    assert_eq!(
        record_field(&records[0], "guardrail_status"),
        json!("success")
    );
}

#[tokio::test(start_paused = true)]
async fn the_second_throttle_waits_twice_as_long() {
    let messages = vec![message("user", json!("hello"))];
    let transport = Scripted::new(vec![
        throttle(),
        throttle(),
        Reply::ok(json!({ "action": "NONE" })),
    ]);
    let mut data = BTreeMap::new();
    let started = Instant::now();
    let outcome = run(&guardrail(), &mut data, input_scan(&messages), &transport).await;
    assert_eq!(started.elapsed(), Duration::from_millis(1_500));
    drop(outcome.expect("the second retry passes"));
    assert_eq!(transport.count(), 3);
}

#[tokio::test(start_paused = true)]
async fn four_throttles_exhaust_the_retry_budget() {
    let messages = vec![message("user", json!("hello"))];
    let transport = Scripted::new(vec![throttle(), throttle(), throttle(), throttle()]);
    let mut data = BTreeMap::new();
    let started = Instant::now();
    let outcome = run(&guardrail(), &mut data, input_scan(&messages), &transport).await;
    assert_eq!(started.elapsed(), Duration::from_millis(3_500));
    assert_eq!(transport.count(), 4);
    let failure = outcome.expect_err("every retry was throttled");
    let GuardrailFailure::Http { status, detail } = failure else {
        panic!("an exhausted throttle is a plain http failure");
    };
    assert_eq!(status, 429);
    assert_eq!(detail, "Rate exceeded");
    let records = records_of(&data);
    assert_eq!(records.len(), 1);
    assert_eq!(
        record_field(&records[0], "guardrail_status"),
        json!("guardrail_failed_to_respond")
    );
}

#[tokio::test(start_paused = true)]
async fn a_validation_error_is_not_retried() {
    let messages = vec![message("user", json!("hello"))];
    let transport = Scripted::new(vec![Reply::failed(
        400,
        json!({ "message": "the request is malformed" }),
    )]);
    let mut data = BTreeMap::new();
    let started = Instant::now();
    let outcome = run(&guardrail(), &mut data, input_scan(&messages), &transport).await;
    assert_eq!(started.elapsed(), Duration::from_millis(0));
    let failure = outcome.expect_err("a 400 fails the call");
    let GuardrailFailure::Http { status, .. } = failure else {
        panic!("a validation error is a plain http failure");
    };
    assert_eq!(status, 400);
    assert_eq!(transport.count(), 1);
}
