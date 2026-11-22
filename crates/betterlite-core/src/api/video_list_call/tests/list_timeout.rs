#![cfg(test)]

use std::time::Duration;

use super::prepare::prepare_list;
use crate::api::video_list_call::VideoListCall;
use crate::api::video_remix_call::error::VideoCallError;
use crate::api::video_remix_call::support::{Bench, Case, Fixture, LIST_BODY, Plan, list_call};

const REQUEST_TIMEOUT: f64 = 6000.0;

#[test]
fn list_timeout_defaults_to_600_seconds() {
    let mut fixture = Fixture::new();
    let prepared = prepare_list(
        &mut fixture.state,
        &VideoListCall::default(),
        "openai",
        REQUEST_TIMEOUT,
    )
    .expect("the list prepares");
    assert_eq!(prepared.timeout, Duration::from_secs(600));
}

#[test]
fn list_timeout_none_uses_the_request_timeout() {
    let mut fixture = Fixture::new();
    let mut call = list_call();
    call.timeout = None;
    let prepared =
        prepare_list(&mut fixture.state, &call, "openai", 7.5).expect("the list prepares");
    assert_eq!(prepared.timeout, Duration::from_millis(7_500));
}

#[test]
fn list_timeout_zero_uses_the_request_timeout() {
    let mut fixture = Fixture::new();
    let mut call = list_call();
    call.timeout = Some(0.0);
    let prepared =
        prepare_list(&mut fixture.state, &call, "openai", 7.5).expect("the list prepares");
    assert_eq!(prepared.timeout, Duration::from_millis(7_500));
}

#[test]
fn list_entry_uses_the_calls_timeout() {
    let mut bench = Bench::new(200, LIST_BODY);
    let mut call = list_call();
    call.timeout = Some(f64::NAN);
    let failure = bench
        .list_answer(Plan::seconds(Case::List(&call), REQUEST_TIMEOUT))
        .expect_err("NaN is not a usable duration");
    assert!(matches!(failure.error, VideoCallError::Timeout { .. }));
    assert!(bench.requests().is_empty());
}

#[test]
fn list_entry_forwards_the_request_timeout() {
    let mut bench = Bench::new(200, LIST_BODY);
    let mut call = list_call();
    call.timeout = None;
    let failure = bench
        .list_answer(Plan::seconds(Case::List(&call), -1.0))
        .expect_err("a negative request timeout is rejected");
    assert!(matches!(failure.error, VideoCallError::Timeout { .. }));
    assert!(bench.requests().is_empty());
}
