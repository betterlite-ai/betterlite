#![cfg(test)]

use std::time::Duration;

use super::support::{Bench, Case, Plan, VIDEO_BODY, remix_call};
use crate::api::video_remix_call::error::VideoCallError;

const REQUEST_TIMEOUT: f64 = 6000.0;

#[test]
fn remix_timeout_defaults_to_600_seconds() {
    let mut bench = Bench::new(200, VIDEO_BODY);
    let call = remix_call();
    bench
        .video_answer(Plan::seconds(Case::Remix(&call), REQUEST_TIMEOUT))
        .expect("the remix answers");
    assert_eq!(bench.requests()[0].timeout, Some(Duration::from_secs(600)));
}

#[test]
fn remix_timeout_none_uses_the_request_timeout() {
    let mut bench = Bench::new(200, VIDEO_BODY);
    let mut call = remix_call();
    call.timeout = None;
    bench
        .video_answer(Plan::seconds(Case::Remix(&call), 7.5))
        .expect("the remix answers");
    assert_eq!(
        bench.requests()[0].timeout,
        Some(Duration::from_millis(7_500))
    );
}

#[test]
fn remix_timeout_zero_uses_the_request_timeout() {
    let mut bench = Bench::new(200, VIDEO_BODY);
    let mut call = remix_call();
    call.timeout = Some(0.0_f64);
    bench
        .video_answer(Plan::seconds(Case::Remix(&call), 7.5))
        .expect("the remix answers");
    assert_eq!(
        bench.requests()[0].timeout,
        Some(Duration::from_millis(7_500))
    );
}

#[test]
fn remix_timeout_given_is_used() {
    let mut bench = Bench::new(200, VIDEO_BODY);
    let mut call = remix_call();
    call.timeout = Some(12.5_f64);
    bench
        .video_answer(Plan::seconds(Case::Remix(&call), REQUEST_TIMEOUT))
        .expect("the remix answers");
    assert_eq!(
        bench.requests()[0].timeout,
        Some(Duration::from_millis(12_500))
    );
}

#[test]
fn remix_timeout_negative_is_a_value_error() {
    let mut bench = Bench::new(200, VIDEO_BODY);
    let mut call = remix_call();
    call.timeout = Some(-1.0_f64);
    let failure = bench
        .video_answer(Plan::seconds(Case::Remix(&call), REQUEST_TIMEOUT))
        .expect_err("a negative timeout fails");
    assert!(matches!(failure.error, VideoCallError::Timeout { .. }));
    assert_eq!(failure.class.metadata().status_code, 500);
    assert_eq!(bench.fixture.state.call_type, "video_remix");
    assert!(bench.requests().is_empty());
}
