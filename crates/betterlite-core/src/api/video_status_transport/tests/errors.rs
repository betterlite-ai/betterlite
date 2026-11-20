#![cfg(test)]

use super::harness::{Call, harness};
use super::support::{
    FAILING, PROBE_STATUS, async_responses, async_sends, complete_urls, conversions, error_classes,
    pre_calls, provider_failure, request_transforms, sync_responses, sync_sends, validate_calls,
};
use crate::api::video_status_transport::error::{BaseLlmException, VideoStatusError};
use crate::api::video_status_transport::records::Params;

fn converted(error: VideoStatusError) -> BaseLlmException {
    let VideoStatusError::Converted(converted) = error else {
        panic!("the shared converter's answer")
    };
    converted
}

fn unconverted(error: VideoStatusError) -> String {
    let VideoStatusError::Unconverted(failure) = error else {
        panic!("the provider's own failure")
    };
    failure.message
}

#[test]
fn error_scope_send_failure_is_converted() {
    let mut run = harness();
    run.deps.factories.sync = FAILING;
    let params = Params::new();
    let failure = converted(
        run.sync(Call::new(&params))
            .expect_err("the transport is closed"),
    );
    assert_eq!(failure.status_code, PROBE_STATUS);
    assert_eq!(failure.message, "fake error class: probe");
    assert!(failure.status_code_is_synthesized);
    let events = run.events();
    assert_eq!(conversions(&events), vec![String::from("send")]);
    assert_eq!(
        error_classes(&events),
        vec![(String::from("probe"), PROBE_STATUS)]
    );
    assert_eq!(pre_calls(&events).len(), 1);
    assert!(sync_responses(&events).is_empty());
}

#[tokio::test]
async fn error_scope_async_send_failure_is_converted() {
    let mut run = harness();
    run.deps.factories.asynchronous = FAILING;
    let params = Params::new();
    let failure = converted(
        run.asynchronous(Call::new(&params))
            .await
            .expect_err("the transport is closed"),
    );
    assert_eq!(failure.status_code, PROBE_STATUS);
    let events = run.events();
    assert_eq!(conversions(&events), vec![String::from("send")]);
    assert_eq!(async_sends(&events).len(), 1);
    assert!(async_responses(&events).is_empty());
}

#[test]
fn error_scope_response_transform_failure_is_converted() {
    let mut run = harness();
    run.deps.config.response = Err(provider_failure("boom"));
    let params = Params::new();
    let failure = converted(
        run.sync(Call::new(&params))
            .expect_err("the transform refuses the reply"),
    );
    assert_eq!(failure.status_code, PROBE_STATUS);
    assert_eq!(failure.message, "fake error class: probe");
    let events = run.events();
    assert_eq!(conversions(&events), vec![String::from("transform: boom")]);
    assert_eq!(sync_sends(&events).len(), 1);
}

#[tokio::test]
async fn error_scope_async_response_transform_failure_is_converted() {
    let mut run = harness();
    run.deps.config.asynchronous_response = Err(provider_failure("async boom"));
    let params = Params::new();
    let failure = converted(
        run.asynchronous(Call::new(&params))
            .await
            .expect_err("the async transform refuses the reply"),
    );
    assert_eq!(failure.status_code, PROBE_STATUS);
    assert_eq!(
        conversions(&run.events()),
        vec![String::from("transform: async boom")]
    );
}

#[test]
fn error_scope_validate_environment_failure_is_not_converted() {
    let mut run = harness();
    run.deps.config.environment = Err(provider_failure("bad headers"));
    let params = Params::new();
    let message = unconverted(
        run.sync(Call::new(&params))
            .expect_err("validate_environment refuses"),
    );
    assert_eq!(message, "bad headers");
    let events = run.events();
    assert!(conversions(&events).is_empty());
    assert!(error_classes(&events).is_empty());
    assert!(complete_urls(&events).is_empty());
    assert!(pre_calls(&events).is_empty());
    assert!(sync_sends(&events).is_empty());
}

#[test]
fn error_scope_complete_url_failure_is_not_converted() {
    let mut run = harness();
    run.deps.config.complete_url = Err(provider_failure("no base"));
    let params = Params::new();
    let message = unconverted(
        run.sync(Call::new(&params))
            .expect_err("get_complete_url refuses"),
    );
    assert_eq!(message, "no base");
    let events = run.events();
    assert!(conversions(&events).is_empty());
    assert!(error_classes(&events).is_empty());
    assert!(request_transforms(&events).is_empty());
    assert!(pre_calls(&events).is_empty());
    assert!(sync_sends(&events).is_empty());
}

#[test]
fn error_scope_request_transform_failure_is_not_converted() {
    let mut run = harness();
    run.deps.config.transformed = Err(provider_failure("no url"));
    let params = Params::new();
    let message = unconverted(
        run.sync(Call::new(&params))
            .expect_err("the request transform refuses"),
    );
    assert_eq!(message, "no url");
    let events = run.events();
    assert!(conversions(&events).is_empty());
    assert!(error_classes(&events).is_empty());
    assert!(pre_calls(&events).is_empty());
    assert!(sync_sends(&events).is_empty());
}

#[test]
fn error_scope_client_build_failure_is_not_converted() {
    let mut run = harness();
    run.deps.factories.refuse_sync = true;
    let params = Params::new();
    let error = run
        .sync(Call::new(&params))
        .expect_err("the pool is closed");
    let VideoStatusError::Client(_) = error else {
        panic!("the pool's own error")
    };
    let events = run.events();
    assert!(conversions(&events).is_empty());
    assert!(validate_calls(&events).is_empty());
    assert!(sync_sends(&events).is_empty());
}

#[tokio::test]
async fn error_scope_async_client_build_failure_is_not_converted() {
    let mut run = harness();
    run.deps.factories.refuse_async = true;
    let params = Params::new();
    let error = run
        .asynchronous(Call::new(&params))
        .await
        .expect_err("the pool is closed");
    let VideoStatusError::Client(_) = error else {
        panic!("the pool's own error")
    };
    let events = run.events();
    assert!(conversions(&events).is_empty());
    assert!(validate_calls(&events).is_empty());
    assert!(async_sends(&events).is_empty());
}
