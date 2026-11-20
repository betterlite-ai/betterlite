#![cfg(test)]

use super::harness::{Call, harness};
use super::support::{
    Owner, Supply, async_builds, async_responses, async_sends, conversions, provider, runway,
    ssl_params, sync_builds, sync_sends, text_ssl_params,
};
use crate::api::video_status_transport::error::VideoStatusError;
use crate::api::video_status_transport::records::Params;

#[test]
fn client_choice_sync_uses_the_supplied_sync_client() {
    let mut run = harness();
    let params = Params::new();
    let mut call = Call::new(&params);
    call.supply = Supply::Sync;
    run.sync(call).expect("the status call succeeds");
    let events = run.events();
    assert!(sync_builds(&events).is_empty());
    assert_eq!(sync_sends(&events)[0].0, Owner::Supplied);
    assert!(async_sends(&events).is_empty());
}

#[test]
fn client_choice_sync_ignores_a_supplied_async_client() {
    let mut run = harness();
    let params = Params::new();
    let mut call = Call::new(&params);
    call.supply = Supply::Async;
    run.sync(call).expect("the status call succeeds");
    let events = run.events();
    assert_eq!(sync_builds(&events), vec![None]);
    assert_eq!(sync_sends(&events)[0].0, Owner::Built);
}

#[test]
fn client_choice_sync_defaults_ssl_verify_to_none() {
    let mut run = harness();
    let params = Params::new();
    run.sync(Call::new(&params))
        .expect("the status call succeeds");
    assert_eq!(sync_builds(&run.events()), vec![None]);
}

#[test]
fn client_choice_sync_passes_ssl_verify_true() {
    let mut run = harness();
    let params = ssl_params(true);
    run.sync(Call::new(&params))
        .expect("the status call succeeds");
    assert_eq!(sync_builds(&run.events()), vec![Some(true)]);
}

#[test]
fn client_choice_sync_passes_ssl_verify_false() {
    let mut run = harness();
    let params = ssl_params(false);
    run.sync(Call::new(&params))
        .expect("the status call succeeds");
    assert_eq!(sync_builds(&run.events()), vec![Some(false)]);
}

#[test]
fn client_choice_sync_refuses_an_ssl_verify_of_another_type() {
    let mut run = harness();
    let params = text_ssl_params();
    let error = run
        .sync(Call::new(&params))
        .expect_err("the string is not a boolean");
    let VideoStatusError::FieldType { field, expected } = error else {
        panic!("a field type error, not a converted one")
    };
    assert_eq!(field, "ssl_verify");
    assert_eq!(expected, "a boolean");
    assert!(sync_sends(&run.events()).is_empty());
}

#[tokio::test]
async fn client_choice_async_uses_the_supplied_async_client() {
    let mut run = harness();
    let params = Params::new();
    let mut call = Call::new(&params);
    call.supply = Supply::Async;
    run.asynchronous(call)
        .await
        .expect("the status call succeeds");
    let events = run.events();
    assert!(async_builds(&events).is_empty());
    assert_eq!(async_sends(&events)[0].0, Owner::Supplied);
    assert!(sync_sends(&events).is_empty());
}

#[tokio::test]
async fn client_choice_async_ignores_a_supplied_sync_client() {
    let mut run = harness();
    let params = Params::new();
    let mut call = Call::new(&params);
    call.supply = Supply::Sync;
    run.asynchronous(call)
        .await
        .expect("the status call succeeds");
    let events = run.events();
    assert_eq!(async_builds(&events), vec![(provider(), None)]);
    assert_eq!(async_sends(&events)[0].0, Owner::Built);
}

#[tokio::test]
async fn client_choice_async_builds_the_pooled_client_for_the_provider() {
    let mut run = harness();
    let params = Params::new();
    run.asynchronous(Call::new(&params))
        .await
        .expect("the status call succeeds");
    assert_eq!(async_builds(&run.events()), vec![(provider(), None)]);
}

#[tokio::test]
async fn client_choice_async_builds_the_client_for_the_calls_own_provider() {
    let mut run = harness();
    let params = Params::new();
    let mut call = Call::new(&params);
    call.custom_llm_provider = "runwayml";
    run.asynchronous(call)
        .await
        .expect("the status call succeeds");
    let events = run.events();
    assert_eq!(async_builds(&events), vec![(runway(), None)]);
    assert_eq!(
        async_responses(&events)[0].provider.as_deref(),
        Some("runwayml")
    );
}

#[tokio::test]
async fn client_choice_async_passes_ssl_verify_true() {
    let mut run = harness();
    let params = ssl_params(true);
    run.asynchronous(Call::new(&params))
        .await
        .expect("the status call succeeds");
    assert_eq!(async_builds(&run.events()), vec![(provider(), Some(true))]);
}

#[tokio::test]
async fn client_choice_async_passes_ssl_verify_false() {
    let mut run = harness();
    let params = ssl_params(false);
    run.asynchronous(Call::new(&params))
        .await
        .expect("the status call succeeds");
    assert_eq!(async_builds(&run.events()), vec![(provider(), Some(false))]);
}

#[tokio::test]
async fn client_choice_async_refuses_an_ssl_verify_of_another_type() {
    let mut run = harness();
    let params = text_ssl_params();
    let error = run
        .asynchronous(Call::new(&params))
        .await
        .expect_err("the string is not a boolean");
    let VideoStatusError::FieldType { field, expected } = error else {
        panic!("a field type error, not a converted one")
    };
    assert_eq!(field, "ssl_verify");
    assert_eq!(expected, "a boolean");
    assert!(async_sends(&run.events()).is_empty());
}

#[tokio::test]
async fn client_choice_async_unknown_provider_is_not_converted() {
    let mut run = harness();
    let params = Params::new();
    let mut call = Call::new(&params);
    call.custom_llm_provider = "unknown-provider";
    let error = run
        .asynchronous(call)
        .await
        .expect_err("the provider name is unknown");
    let VideoStatusError::UnknownProvider { provider: named } = error else {
        panic!("the provider error, not a converted one")
    };
    assert_eq!(named, "unknown-provider");
    let events = run.events();
    assert!(async_sends(&events).is_empty());
    assert!(conversions(&events).is_empty());
}

#[tokio::test]
async fn client_choice_async_supplied_client_skips_the_provider_lookup() {
    let mut run = harness();
    let params = Params::new();
    let mut call = Call::new(&params);
    call.custom_llm_provider = "unknown-provider";
    call.supply = Supply::Async;
    run.asynchronous(call)
        .await
        .expect("a supplied client needs no provider lookup");
    let events = run.events();
    assert!(async_builds(&events).is_empty());
    assert_eq!(async_sends(&events)[0].0, Owner::Supplied);
}

#[test]
fn client_choice_sync_needs_no_provider() {
    let mut run = harness();
    let params = Params::new();
    let mut call = Call::new(&params);
    call.custom_llm_provider = "unknown-provider";
    run.sync(call)
        .expect("the sync client is not looked up by provider");
    assert_eq!(sync_sends(&run.events())[0].0, Owner::Built);
}
