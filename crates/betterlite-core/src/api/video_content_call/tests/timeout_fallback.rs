use std::time::Duration;

use super::support::{Bench, Outcome, REQUEST_TIMEOUT, VIDEO_ID, call, handled, ready, request};

#[test]
fn timeout_fallback_content_none_takes_the_request_timeout() {
    let mut bench = Bench::new(Outcome::Bytes(b"video".to_vec()));
    let mut request = request(VIDEO_ID);
    request.timeout = None;
    ready(call(&mut bench, request));
    assert_eq!(handled(&bench.journal).timeout, REQUEST_TIMEOUT);
}

#[test]
fn timeout_fallback_content_zero_takes_the_request_timeout() {
    let mut bench = Bench::new(Outcome::Bytes(b"video".to_vec()));
    bench.caller.request_timeout = Duration::from_secs(11);
    let mut request = request(VIDEO_ID);
    request.timeout = Some(Duration::ZERO);
    ready(call(&mut bench, request));
    assert_eq!(handled(&bench.journal).timeout, Duration::from_secs(11));
}

#[test]
fn timeout_fallback_content_given_timeout_is_used() {
    let mut bench = Bench::new(Outcome::Bytes(b"video".to_vec()));
    let mut request = request(VIDEO_ID);
    request.timeout = Some(Duration::from_secs(30));
    ready(call(&mut bench, request));
    assert_eq!(handled(&bench.journal).timeout, Duration::from_secs(30));
}
