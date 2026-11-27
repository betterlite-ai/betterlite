use std::time::Duration;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use super::support::{
    FILE_ID, Fixture, REQUEST_TIMEOUT, Spy, VECTOR_STORE_ID, Which, attributes, attributes_json,
    deleted, recorded, success,
};
use crate::api::vector_store_http_write::service::{ClientKind, Operation};

#[rstest]
#[case::absent(None, 6000.0_f64, 6000_u64)]
#[case::default_zero(Some(0.0_f64), 6000.0_f64, 6000_u64)]
#[case::explicit(Some(7.0_f64), 6000.0_f64, 7_u64)]
#[case::environment_absent(None, 12.0_f64, 12_u64)]
#[case::environment_zero(Some(0.0_f64), 12.0_f64, 12_u64)]
#[case::environment_explicit(Some(7.0_f64), 12.0_f64, 7_u64)]
#[tokio::test]
async fn write_timeout(
    #[case] requested: Option<f64>,
    #[case] request_timeout_seconds: f64,
    #[case] expected: u64,
) {
    let spy = Spy::replying(success());
    let mut fixture = Fixture::new();
    fixture.arguments.timeout = requested;
    let seen = recorded(Which::Update, &spy, &fixture, request_timeout_seconds).await;
    assert_eq!(seen.timeout, Duration::from_secs(expected));
}

#[tokio::test]
async fn update_logs_the_ids_and_the_attributes() {
    let spy = Spy::replying(success());
    let mut fixture = Fixture::new();
    fixture.arguments.attributes = Some(attributes());
    let seen = recorded(Which::Update, &spy, &fixture, REQUEST_TIMEOUT).await;
    assert_eq!(seen.model, "");
    assert_eq!(seen.provider_name.as_deref(), Some("openai"));
    // the log keeps the argument as the caller gave it, before the wire cleaning drops non-strings
    assert_eq!(
        seen.optional,
        IndexMap::from([
            (
                String::from("vector_store_id"),
                Json::Text(String::from(VECTOR_STORE_ID)),
            ),
            (String::from("file_id"), Json::Text(String::from(FILE_ID)),),
            (String::from("attributes"), attributes_json()),
        ])
    );
    assert_eq!(
        seen.litellm_params.get("vector_store_id"),
        Some(&Json::Text(String::from(VECTOR_STORE_ID)))
    );
    assert_eq!(
        seen.litellm_params.get("litellm_call_id"),
        Some(&Json::Null)
    );
    assert_eq!(
        seen.details.get("custom_llm_provider"),
        Some(&Json::Text(String::from("openai")))
    );
}

#[tokio::test]
async fn async_logging_keeps_the_call_id() {
    let spy = Spy::replying(success());
    let mut fixture = Fixture::new();
    fixture.arguments.attributes = Some(attributes());
    fixture.arguments.kwargs.insert(
        String::from("litellm_call_id"),
        Json::Text(String::from("call-a")),
    );
    let seen = recorded(Which::Aupdate, &spy, &fixture, REQUEST_TIMEOUT).await;
    assert_eq!(seen.operation, Operation::Update);
    assert_eq!(seen.mode, ClientKind::Asynchronous);
    assert_eq!(seen.model, "");
    assert_eq!(seen.optional.get("attributes"), Some(&attributes_json()));
    assert_eq!(
        seen.litellm_params.get("litellm_call_id"),
        Some(&Json::Text(String::from("call-a")))
    );
    // LiteLLM pops the flag before logging, so it never reaches litellm_params
    assert_eq!(seen.litellm_params.get("aupdate"), None);
}

#[tokio::test]
async fn adelete_sets_the_flag_itself() {
    let spy = Spy::replying(deleted());
    let fixture = Fixture::new();
    let seen = recorded(Which::Adelete, &spy, &fixture, REQUEST_TIMEOUT).await;
    assert_eq!(seen.operation, Operation::Delete);
    assert_eq!(seen.mode, ClientKind::Asynchronous);
    assert_eq!(seen.litellm_params.get("adelete"), None);
    assert_eq!(
        seen.litellm_params.get("litellm_call_id"),
        Some(&Json::Null)
    );
}

#[tokio::test]
async fn integer_flags_do_not_select_async() {
    let spy = Spy::replying(success());
    let mut update_fixture = Fixture::new();
    update_fixture
        .arguments
        .kwargs
        .insert(String::from("aupdate"), Json::Number(Number::from(1_u32)));
    let update_seen = recorded(Which::Update, &spy, &update_fixture, REQUEST_TIMEOUT).await;
    assert_eq!(update_seen.mode, ClientKind::Sync);
    // the pop takes the flag whatever its value, as LiteLLM's kwargs.pop does
    assert_eq!(update_seen.litellm_params.get("aupdate"), None);
    let mut delete_fixture = Fixture::new();
    delete_fixture
        .arguments
        .kwargs
        .insert(String::from("adelete"), Json::Number(Number::from(1_u32)));
    let delete_seen = recorded(Which::Delete, &spy, &delete_fixture, REQUEST_TIMEOUT).await;
    assert_eq!(delete_seen.mode, ClientKind::Sync);
    assert_eq!(delete_seen.litellm_params.get("adelete"), None);
}
