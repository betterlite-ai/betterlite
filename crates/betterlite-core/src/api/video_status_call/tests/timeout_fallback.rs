use std::time::Duration;

use super::support::{Bench, Outcome, REQUEST_TIMEOUT, VIDEO_ID, call, handled, ready, request};
use crate::api::video_status_call::DEFAULT_TIMEOUT;

#[test]
fn timeout_fallback_status_none_takes_the_declared_default() {
    let mut bench = Bench::new(Outcome::answers());
    let mut request = request(VIDEO_ID);
    request.timeout = None;
    ready(call(&mut bench, request));
    assert_eq!(handled(&bench.journal).timeout, Some(DEFAULT_TIMEOUT));
}

#[test]
fn timeout_fallback_status_zero_takes_the_request_timeout() {
    let mut bench = Bench::new(Outcome::answers());
    bench.caller.request_timeout = Duration::from_secs(11);
    let mut request = request(VIDEO_ID);
    request.timeout = Some(Duration::ZERO);
    ready(call(&mut bench, request));
    assert_eq!(
        handled(&bench.journal).timeout,
        Some(Duration::from_secs(11))
    );
}

#[test]
fn timeout_fallback_status_given_timeout_is_used() {
    let mut bench = Bench::new(Outcome::answers());
    let mut request = request(VIDEO_ID);
    request.timeout = Some(Duration::from_secs(120));
    ready(call(&mut bench, request));
    assert_eq!(
        handled(&bench.journal).timeout,
        Some(Duration::from_secs(120))
    );
    assert_ne!(REQUEST_TIMEOUT, Duration::from_secs(120));
}
