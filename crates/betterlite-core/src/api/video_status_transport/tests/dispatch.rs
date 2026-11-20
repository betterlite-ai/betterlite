#![cfg(test)]

use secrecy::SecretString;

use super::harness::{Call, Harness, harness};
use super::support::{
    COMPLETE_URL, Owner, Supply, VIDEO_ID, async_builds, async_responses, async_sends, duration,
    extra_headers, merged_headers, object, operation, provider, provider_headers,
    request_transforms, secret, ssl_params, sync_responses, unused_body, validate_calls,
};
use crate::api::video_status_transport::records::{Headers, Params};

fn configured() -> Harness {
    let mut built = harness();
    built.deps.config.environment = Ok(provider_headers());
    built.deps.config.transformed = Ok((COMPLETE_URL.to_owned(), operation()));
    built
}

fn full_call<'call>(
    params: &'call Params,
    headers: &'call Headers,
    body: &'call Params,
    key: &'call SecretString,
) -> Call<'call> {
    let mut call = Call::new(params);
    call.extra_headers = Some(headers);
    call.extra_body = Some(body);
    call.timeout = Some(duration());
    call.api_key = Some(key);
    call.supply = Supply::Async;
    call
}

#[tokio::test]
async fn async_flag_returns_future_before_choosing_a_client() {
    let mut run = configured();
    let params = ssl_params(true);
    let (before, outcome) = run.flagged(Call::new(&params)).await;
    assert!(before.is_empty());
    assert_eq!(
        outcome.expect("the flagged call succeeds"),
        object(VIDEO_ID)
    );
    let events = run.events();
    assert_eq!(async_builds(&events), vec![(provider(), Some(true))]);
    assert_eq!(async_sends(&events).len(), 1);
    assert_eq!(async_responses(&events).len(), 1);
    assert!(sync_responses(&events).is_empty());
}

#[tokio::test]
async fn async_flag_returns_future_with_the_async_transforms_result() {
    let mut run = configured();
    run.deps.config.response = Ok(object("sync-result"));
    run.deps.config.asynchronous_response = Ok(object("async-result"));
    let params = Params::new();
    let (_, outcome) = run.flagged(Call::new(&params)).await;
    assert_eq!(
        outcome.expect("the flagged call succeeds"),
        object("async-result")
    );
    assert!(sync_responses(&run.events()).is_empty());
}

#[tokio::test]
async fn async_flag_returns_future_forwarding_every_argument() {
    let mut flagged_run = configured();
    let mut direct_run = configured();
    let params = ssl_params(true);
    let headers = extra_headers();
    let body = unused_body();
    let key = secret("sk-test");
    let (before, flagged_outcome) = flagged_run
        .flagged(full_call(&params, &headers, &body, &key))
        .await;
    assert!(before.is_empty());
    let direct_outcome = direct_run
        .asynchronous(full_call(&params, &headers, &body, &key))
        .await;
    assert_eq!(
        flagged_outcome.expect("the flagged call succeeds"),
        direct_outcome.expect("the async call succeeds")
    );
    assert_eq!(flagged_run.events(), direct_run.events());
    let events = flagged_run.events();
    assert_eq!(
        validate_calls(&events)[0].api_key,
        Some("sk-test".to_owned())
    );
    assert_eq!(request_transforms(&events)[0].headers, merged_headers());
    assert_eq!(async_sends(&events)[0].0, Owner::Built);
}
