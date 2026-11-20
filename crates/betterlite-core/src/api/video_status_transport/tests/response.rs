#![cfg(test)]

use betterlite_models::decisions::Json;

use super::harness::{Call, harness};
use super::support::{Owner, PROVIDER, Supply, async_responses, object, reply, sync_responses};
use crate::api::video_status_transport::records::Params;

#[test]
fn response_transform_sync_gets_the_built_client() {
    let mut run = harness();
    let params = Params::new();
    run.sync(Call::new(&params))
        .expect("the status call succeeds");
    let events = run.events();
    let responses = sync_responses(&events);
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0].owner, Some(Owner::Built));
    assert!(async_responses(&events).is_empty());
}

#[test]
fn response_transform_sync_gets_the_supplied_client() {
    let mut run = harness();
    let params = Params::new();
    let mut call = Call::new(&params);
    call.supply = Supply::Sync;
    run.sync(call).expect("the status call succeeds");
    assert_eq!(
        sync_responses(&run.events())[0].owner,
        Some(Owner::Supplied)
    );
}

#[tokio::test]
async fn response_transform_async_gets_the_built_client() {
    let mut run = harness();
    let params = Params::new();
    run.asynchronous(Call::new(&params))
        .await
        .expect("the status call succeeds");
    let events = run.events();
    let responses = async_responses(&events);
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0].owner, Some(Owner::Built));
    assert!(sync_responses(&events).is_empty());
}

#[tokio::test]
async fn response_transform_async_gets_the_supplied_client() {
    let mut run = harness();
    let params = Params::new();
    let mut call = Call::new(&params);
    call.supply = Supply::Async;
    run.asynchronous(call)
        .await
        .expect("the status call succeeds");
    assert_eq!(
        async_responses(&run.events())[0].owner,
        Some(Owner::Supplied)
    );
}

#[test]
fn response_transform_gets_the_raw_reply_and_the_provider() {
    let mut run = harness();
    run.deps.factories.sync = reply(201, "video_body");
    let params = Params::new();
    run.sync(Call::new(&params))
        .expect("the status call succeeds");
    let responses = sync_responses(&run.events());
    assert_eq!(responses[0].status, 201);
    assert_eq!(responses[0].body, b"video_body".to_vec());
    assert_eq!(responses[0].header, Some("yes".to_owned()));
    assert_eq!(responses[0].provider, Some(PROVIDER.to_owned()));
}

#[test]
fn response_transform_sync_writes_into_the_calls_logging_state() {
    let mut run = harness();
    let params = Params::new();
    run.sync(Call::new(&params))
        .expect("the status call succeeds");
    let details = &run.state.model_call_details;
    assert_eq!(details.get("sync_transform"), Some(&Json::Bool(true)));
    assert_eq!(details.get("async_transform"), None);
}

#[tokio::test]
async fn response_transform_async_writes_into_the_calls_logging_state() {
    let mut run = harness();
    let params = Params::new();
    run.asynchronous(Call::new(&params))
        .await
        .expect("the status call succeeds");
    let details = &run.state.model_call_details;
    assert_eq!(details.get("async_transform"), Some(&Json::Bool(true)));
    assert_eq!(details.get("sync_transform"), None);
}

#[test]
fn response_transform_returns_the_providers_object() {
    let mut run = harness();
    run.deps.config.response = Ok(object("converted"));
    let params = Params::new();
    let video = run
        .sync(Call::new(&params))
        .expect("the status call succeeds");
    assert_eq!(video, object("converted"));
}
