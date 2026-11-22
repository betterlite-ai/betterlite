#![cfg(test)]

use std::time::Duration;

use super::support::{Bench, Case, Plan, VIDEO_BODY};
use crate::api::video_remix_call::VideoRemixCall;
use crate::api::video_remix_call::error::VideoCallError;

const REQUEST_TIMEOUT: f64 = 6000.0;

#[test]
fn remix_timeout_defaults_to_600_seconds() {
    let mut bench = Bench::new(200, VIDEO_BODY);
    bench
        .video_answer(Plan::seconds(
            Case::Remix(&VideoRemixCall::default()),
            REQUEST_TIMEOUT,
        ))
        .expect("the remix answers");
    assert_eq!(bench.requests()[0].timeout, Some(Duration::from_secs(600)));
}

#[test]
fn remix_timeout_none_uses_the_request_timeout() {
    let mut bench = Bench::new(200, VIDEO_BODY);
    let call = VideoRemixCall {
        timeout: None,
        ..VideoRemixCall::default()
    };
    bench
        .video_answer(Plan::seconds(Case::Remix(&call), 7.5))
        .expect("the remix answers");
    assert_eq!(
        bench.requests()[0].timeout,
        Some(Duration::from_secs_f64(7.5))
    );
}

#[test]
fn remix_timeout_zero_uses_the_request_timeout() {
    let mut bench = Bench::new(200, VIDEO_BODY);
    let call = VideoRemixCall {
        timeout: Some(0.0),
        ..VideoRemixCall::default()
    };
    bench
        .video_answer(Plan::seconds(Case::Remix(&call), 7.5))
        .expect("the remix answers");
    assert_eq!(
        bench.requests()[0].timeout,
        Some(Duration::from_secs_f64(7.5))
    );
}

#[test]
fn remix_timeout_given_is_used() {
    let mut bench = Bench::new(200, VIDEO_BODY);
    let call = VideoRemixCall {
        timeout: Some(12.5),
        ..VideoRemixCall::default()
    };
    bench
        .video_answer(Plan::seconds(Case::Remix(&call), REQUEST_TIMEOUT))
        .expect("the remix answers");
    assert_eq!(
        bench.requests()[0].timeout,
        Some(Duration::from_secs_f64(12.5))
    );
}

#[test]
fn remix_timeout_negative_is_a_value_error() {
    let mut bench = Bench::new(200, VIDEO_BODY);
    let call = VideoRemixCall {
        timeout: Some(-1.0),
        ..VideoRemixCall::default()
    };
    let failure = bench
        .video_answer(Plan::seconds(Case::Remix(&call), REQUEST_TIMEOUT))
        .expect_err("a negative timeout fails");
    assert!(matches!(failure.error, VideoCallError::Timeout { .. }));
    assert_eq!(failure.class.metadata().status_code, 500);
    assert_eq!(bench.fixture.state.call_type, "video_remix");
    assert!(bench.requests().is_empty());
}
