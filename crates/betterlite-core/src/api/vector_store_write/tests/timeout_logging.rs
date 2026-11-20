use std::time::Duration;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use super::super::service::Operation;
use super::support::{
    API_BASE, Fixture, REQUEST_TIMEOUT, Recorded, Resolution, Spy, VECTOR_STORE_ID, call_adelete,
    call_aupdate, call_delete, call_update, deleted, success,
};
use crate::api::vector_store_http_write::service::ClientKind;

async fn seen_update(
    spy: &Spy,
    fixture: &Fixture,
    resolution: &Resolution,
    request_timeout_seconds: f64,
) -> Recorded {
    call_update(spy, fixture, resolution, request_timeout_seconds)
        .await
        .expect("the write succeeds");
    spy.seen().expect("the service saw the write")
}

async fn seen_aupdate(
    spy: &Spy,
    fixture: &Fixture,
    resolution: &Resolution,
    request_timeout_seconds: f64,
) -> Recorded {
    call_aupdate(spy, fixture, resolution, request_timeout_seconds)
        .await
        .expect("the write succeeds");
    spy.seen().expect("the service saw the write")
}

async fn seen_delete(
    spy: &Spy,
    fixture: &Fixture,
    resolution: &Resolution,
    request_timeout_seconds: f64,
) -> Recorded {
    call_delete(spy, fixture, resolution, request_timeout_seconds)
        .await
        .expect("the write succeeds");
    spy.seen().expect("the service saw the write")
}

async fn seen_adelete(
    spy: &Spy,
    fixture: &Fixture,
    resolution: &Resolution,
    request_timeout_seconds: f64,
) -> Recorded {
    call_adelete(spy, fixture, resolution, request_timeout_seconds)
        .await
        .expect("the write succeeds");
    spy.seen().expect("the service saw the write")
}

fn file_ids() -> Json {
    Json::List(vec![Json::Text(String::from("file_a"))])
}

#[tokio::test]
async fn write_timeout_and_logging() {
    let spy = Spy::replying(success());
    let resolution = Resolution::new();
    let mut fixture = Fixture::new();
    fixture.name = Some(String::from("notes"));
    fixture.kwargs.insert(String::from("file_ids"), file_ids());
    let seen = seen_update(&spy, &fixture, &resolution, REQUEST_TIMEOUT).await;
    assert_eq!(seen.timeout, Duration::from_secs(6000));
    assert_eq!(seen.api_base.as_deref(), Some(API_BASE));
    assert_eq!(seen.model, "");
    assert_eq!(seen.call_type, "endpoint_func");
    assert_eq!(
        seen.litellm_params.get("litellm_call_id"),
        Some(&Json::Null)
    );
    assert_eq!(
        seen.optional.get("vector_store_id"),
        Some(&Json::Text(String::from(VECTOR_STORE_ID)))
    );
    assert_eq!(
        seen.optional.get("name"),
        Some(&Json::Text(String::from("notes")))
    );
    assert_eq!(seen.optional.get("file_ids"), Some(&file_ids()));
    assert_eq!(seen.provider_name.as_deref(), Some("openai"));
    assert_eq!(
        seen.details.get("custom_llm_provider"),
        Some(&Json::Text(String::from("openai")))
    );
}

#[tokio::test]
async fn timeout_zero_selects_request_timeout() {
    let spy = Spy::replying(success());
    let resolution = Resolution::new();
    let mut zero = Fixture::new();
    zero.timeout = Some(0.0_f64);
    let default_seen = seen_update(&spy, &zero, &resolution, REQUEST_TIMEOUT).await;
    assert_eq!(default_seen.timeout, Duration::from_secs(6000));
    let environment_seen = seen_update(&spy, &zero, &resolution, 12.0_f64).await;
    assert_eq!(environment_seen.timeout, Duration::from_secs(12));
    let absent = Fixture::new();
    let unset_seen = seen_update(&spy, &absent, &resolution, 12.0_f64).await;
    assert_eq!(unset_seen.timeout, Duration::from_secs(12));
}

#[tokio::test]
async fn a_nonzero_timeout_wins() {
    let spy = Spy::replying(success());
    let resolution = Resolution::new();
    let mut seven = Fixture::new();
    seven.timeout = Some(7.0_f64);
    let default_seen = seen_update(&spy, &seven, &resolution, REQUEST_TIMEOUT).await;
    assert_eq!(default_seen.timeout, Duration::from_secs(7));
    let environment_seen = seen_update(&spy, &seven, &resolution, 12.0_f64).await;
    assert_eq!(environment_seen.timeout, Duration::from_secs(7));
}

#[tokio::test]
async fn async_logging_keeps_the_supplied_call_id() {
    let spy = Spy::replying(success());
    let resolution = Resolution::new();
    let mut fixture = Fixture::new();
    fixture.timeout = Some(7.0_f64);
    fixture.kwargs.insert(
        String::from("litellm_call_id"),
        Json::Text(String::from("call-a")),
    );
    let seen = seen_aupdate(&spy, &fixture, &resolution, 12.0_f64).await;
    assert_eq!(seen.mode, ClientKind::Asynchronous);
    assert_eq!(seen.timeout, Duration::from_secs(7));
    assert_eq!(seen.model, "");
    assert_eq!(
        seen.litellm_params.get("litellm_call_id"),
        Some(&Json::Text(String::from("call-a")))
    );
    // LiteLLM pops the flag before logging, so it never reaches litellm_params
    assert_eq!(seen.litellm_params.get("aupdate"), None);
}

#[tokio::test]
async fn delete_logs_the_id_alone() {
    let spy = Spy::replying(deleted());
    let resolution = Resolution::new();
    let fixture = Fixture::new();
    let seen = seen_delete(&spy, &fixture, &resolution, REQUEST_TIMEOUT).await;
    assert_eq!(seen.operation, Operation::Delete);
    assert_eq!(
        seen.optional,
        IndexMap::from([(
            String::from("vector_store_id"),
            Json::Text(String::from(VECTOR_STORE_ID)),
        )])
    );
}

#[tokio::test]
async fn adelete_sets_the_flag_itself() {
    let spy = Spy::replying(deleted());
    let resolution = Resolution::new();
    let fixture = Fixture::new();
    let seen = seen_adelete(&spy, &fixture, &resolution, REQUEST_TIMEOUT).await;
    assert_eq!(seen.operation, Operation::Delete);
    assert_eq!(seen.mode, ClientKind::Asynchronous);
    assert_eq!(seen.litellm_params.get("adelete"), None);
}

#[tokio::test]
async fn integer_flags_do_not_select_async() {
    let spy = Spy::replying(success());
    let resolution = Resolution::new();
    let mut update_fixture = Fixture::new();
    update_fixture
        .kwargs
        .insert(String::from("aupdate"), Json::Number(Number::from(1_u32)));
    let update_seen = seen_update(&spy, &update_fixture, &resolution, REQUEST_TIMEOUT).await;
    assert_eq!(update_seen.mode, ClientKind::Sync);
    // the pop takes the flag whatever its value, as LiteLLM's kwargs.pop does
    assert_eq!(update_seen.litellm_params.get("aupdate"), None);
    let mut delete_fixture = Fixture::new();
    delete_fixture
        .kwargs
        .insert(String::from("adelete"), Json::Number(Number::from(1_u32)));
    let delete_seen = seen_delete(&spy, &delete_fixture, &resolution, REQUEST_TIMEOUT).await;
    assert_eq!(delete_seen.mode, ClientKind::Sync);
    assert_eq!(delete_seen.litellm_params.get("adelete"), None);
}
