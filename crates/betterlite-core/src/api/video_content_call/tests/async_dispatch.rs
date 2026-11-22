use betterlite_models::decisions::Json;

use super::support::{
    Bench, Outcome, VIDEO_ID, Wire, async_call, call, handled, kwargs, pending_work, ready,
    request, text,
};
use crate::api::videos_http_transport::client::{ClientKind, SuppliedClient};

#[test]
fn async_dispatch_false_flag_answers_with_the_ready_result() {
    let mut bench = Bench::new(Outcome::Bytes(b"video".to_vec()));
    let mut request = request(VIDEO_ID);
    request.kwargs = kwargs(&[("async_call", Json::Bool(false))]);
    assert_eq!(ready(call(&mut bench, request)), b"video".to_vec());
    let handling = handled(&bench.journal);
    assert!(!handling.is_async);
    assert!(!handling.litellm_params.contains_key("async_call"));
}

#[test]
fn async_dispatch_non_bool_truthy_flag_stays_synchronous() {
    let mut bench = Bench::new(Outcome::Bytes(b"video".to_vec()));
    let mut request = request(VIDEO_ID);
    request.kwargs = kwargs(&[("async_call", text("true"))]);
    assert_eq!(ready(call(&mut bench, request)), b"video".to_vec());
    assert!(!handled(&bench.journal).is_async);
}

#[tokio::test]
async fn async_dispatch_true_flag_returns_the_pending_handler() {
    let mut bench = Bench::new(Outcome::Bytes(b"video".to_vec()));
    let mut request = request(VIDEO_ID);
    request.kwargs = kwargs(&[("async_call", Json::Bool(true))]);
    let bytes = pending_work(call(&mut bench, request))
        .await
        .expect("the pending handler answers");
    assert_eq!(bytes, b"video".to_vec());
    let handling = handled(&bench.journal);
    assert!(handling.is_async);
    assert!(!handling.litellm_params.contains_key("async_call"));
}

#[tokio::test]
async fn async_dispatch_facade_forces_the_flag_and_awaits() {
    let mut bench = Bench::new(Outcome::Bytes(b"video".to_vec()));
    let mut request = request(VIDEO_ID);
    request.kwargs = kwargs(&[("async_call", Json::Bool(false))]);
    let bytes = async_call(&mut bench, request)
        .await
        .expect("the call answers");
    assert_eq!(bytes, b"video".to_vec());
    assert!(handled(&bench.journal).is_async);
}

#[tokio::test]
async fn async_dispatch_facade_forwards_the_supplied_client() {
    let mut bench = Bench::new(Outcome::Bytes(b"video".to_vec()));
    let wire = Wire::new();
    let bytes = bench
        .caller
        .avideo_content(
            request(VIDEO_ID),
            Some(SuppliedClient {
                kind: ClientKind::Sync,
                transport: &wire,
            }),
            &mut bench.fixture.state,
        )
        .await
        .expect("the call answers");
    assert_eq!(bytes, b"video".to_vec());
    assert_eq!(handled(&bench.journal).client, Some(ClientKind::Sync));
}
