use betterlite_models::decisions::Json;

use super::support::{
    Bench, Journal, Outcome, SyncStub, VIDEO_ID, async_call, call, handled, kwargs, pending_object,
    ready, request, text,
};
use crate::api::video_status_transport::records::SuppliedClient;

#[test]
fn async_dispatch_false_flag_answers_with_the_ready_result() {
    let mut bench = Bench::new(Outcome::answers());
    let mut request = request(VIDEO_ID);
    request.kwargs = kwargs(&[("async_call", Json::Bool(false))]);
    assert_eq!(ready(call(&mut bench, request)).id, VIDEO_ID);
    let handling = handled(&bench.journal);
    assert!(!handling.is_async);
    assert!(!handling.litellm_params.contains_key("async_call"));
}

#[test]
fn async_dispatch_non_bool_truthy_flag_stays_synchronous() {
    let mut bench = Bench::new(Outcome::answers());
    let mut request = request(VIDEO_ID);
    request.kwargs = kwargs(&[("async_call", text("true"))]);
    assert_eq!(ready(call(&mut bench, request)).id, VIDEO_ID);
    assert!(!handled(&bench.journal).is_async);
}

#[tokio::test]
async fn async_dispatch_true_flag_returns_the_pending_handler() {
    let mut bench = Bench::new(Outcome::answers());
    let mut request = request(VIDEO_ID);
    request.kwargs = kwargs(&[("async_call", Json::Bool(true))]);
    let video = pending_object(call(&mut bench, request)).await;
    assert_eq!(video.id, VIDEO_ID);
    let handling = handled(&bench.journal);
    assert!(handling.is_async);
    assert!(!handling.litellm_params.contains_key("async_call"));
}

#[tokio::test]
async fn async_dispatch_facade_forces_the_flag_and_awaits() {
    let mut bench = Bench::new(Outcome::answers());
    let mut request = request(VIDEO_ID);
    request.kwargs = kwargs(&[("async_call", Json::Bool(false))]);
    let video = async_call(&mut bench, request)
        .await
        .expect("the call answers");
    assert_eq!(video.id, VIDEO_ID);
    assert!(handled(&bench.journal).is_async);
}

#[tokio::test]
async fn async_dispatch_facade_forwards_the_supplied_client() {
    let mut bench = Bench::new(Outcome::answers());
    let stub = SyncStub {
        journal: Journal::new(),
    };
    let video = bench
        .caller
        .avideo_status(
            request(VIDEO_ID),
            Some(SuppliedClient::Sync(&stub)),
            &mut bench.fixture.state,
        )
        .await
        .expect("the call answers");
    assert_eq!(video.id, VIDEO_ID);
    assert_eq!(handled(&bench.journal).client, Some("sync"));
}
