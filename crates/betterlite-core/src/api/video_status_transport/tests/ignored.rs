#![cfg(test)]

use super::harness::{Call, harness};
use super::support::{
    VIDEO_ID, async_responses, conversions, duration, object, reply, sync_responses, sync_sends,
    unused_body,
};
use crate::api::video_status_transport::records::Params;

#[test]
fn ignored_params_timeout_does_not_change_the_request() {
    let mut timed = harness();
    let mut plain = harness();
    let params = Params::new();
    let mut call = Call::new(&params);
    call.timeout = Some(duration());
    timed.sync(call).expect("the status call succeeds");
    plain
        .sync(Call::new(&params))
        .expect("the status call succeeds");
    assert_eq!(sync_sends(&timed.events()), sync_sends(&plain.events()));
}

#[test]
fn ignored_params_extra_body_does_not_change_the_request() {
    let mut carrying = harness();
    let mut plain = harness();
    let params = Params::new();
    let body = unused_body();
    let mut call = Call::new(&params);
    call.extra_body = Some(&body);
    carrying.sync(call).expect("the status call succeeds");
    plain
        .sync(Call::new(&params))
        .expect("the status call succeeds");
    assert_eq!(sync_sends(&carrying.events()), sync_sends(&plain.events()));
}

#[test]
fn ignored_params_an_error_status_reaches_the_transform() {
    let mut run = harness();
    run.deps.factories.sync = reply(500, "server error");
    let params = Params::new();
    let video = run
        .sync(Call::new(&params))
        .expect("a 500 reaches the provider's transform");
    assert_eq!(video, object(VIDEO_ID));
    let events = run.events();
    assert_eq!(sync_responses(&events)[0].status, 500);
    assert!(conversions(&events).is_empty());
}

#[tokio::test]
async fn ignored_params_an_error_status_reaches_the_async_transform() {
    let mut run = harness();
    run.deps.factories.asynchronous = reply(503, "unavailable");
    let params = Params::new();
    let video = run
        .asynchronous(Call::new(&params))
        .await
        .expect("a 503 reaches the provider's transform");
    assert_eq!(video, object(VIDEO_ID));
    let events = run.events();
    assert_eq!(async_responses(&events)[0].status, 503);
    assert!(conversions(&events).is_empty());
}
