use std::time::Duration;

use rstest::rstest;

use super::support::{
    Fixture, REQUEST_TIMEOUT, Spy, Which, attributes, chunking, file, json, listing, object,
    params, recorded,
};
use crate::api::vector_store_http_write::service::ClientKind;

const CHUNKING: &str =
    r#"{"type":"static","static":{"max_chunk_size_tokens":800,"chunk_overlap_tokens":400}}"#;

#[rstest]
#[case::unset(None, REQUEST_TIMEOUT, 6000)]
#[case::zero(Some(0.0_f64), REQUEST_TIMEOUT, 6000)]
#[case::seven(Some(7.0_f64), REQUEST_TIMEOUT, 7)]
#[case::unset_with_request_timeout(None, 12.0_f64, 12)]
#[case::zero_with_request_timeout(Some(0.0_f64), 12.0_f64, 12)]
#[case::seven_with_request_timeout(Some(7.0_f64), 12.0_f64, 7)]
#[tokio::test]
async fn attachment_timeout_context(
    #[case] requested: Option<f64>,
    #[case] request_timeout_seconds: f64,
    #[case] expected: u64,
) {
    let mut fixture = Fixture::new();
    fixture.timeout = requested;
    let spy = Spy::replying(file());
    let seen = recorded(Which::Create, &spy, &fixture, None, request_timeout_seconds).await;
    assert_eq!(seen.timeout, Duration::from_secs(expected));
    assert_eq!(spy.call_count(), 1);
}

#[tokio::test]
async fn attachment_logging_context() {
    let mut fixture = Fixture::new();
    fixture.attributes = Some(attributes(r#"{"tag":"x","count":2}"#));
    fixture.chunking_strategy = Some(chunking());
    fixture
        .kwargs
        .insert(String::from("api_key"), json(r#""sk-caller""#));
    fixture
        .kwargs
        .insert(String::from("ssl_verify"), json("true"));
    fixture.kwargs.insert(String::from("unknown"), json("1"));
    let spy = Spy::replying(file());
    let seen = recorded(Which::Create, &spy, &fixture, None, REQUEST_TIMEOUT).await;
    assert_eq!(seen.model, "");
    assert_eq!(seen.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(
        seen.optional_params,
        object(&format!(
            r#"{{"vector_store_id":"vs_a","file_id":"file_a","attributes":{{"tag":"x","count":2}},"chunking_strategy":{CHUNKING}}}"#
        ))
    );
    assert_eq!(
        seen.litellm_params,
        object(
            r#"{"vector_store_id":"vs_a","litellm_call_id":null,"allow_client_keepalive_override":false,"merge_reasoning_content_in_choices":false,"use_in_pass_through":false,"use_litellm_proxy":false,"use_xai_oauth":false,"api_key":"sk-caller","ssl_verify":true,"unknown":1}"#
        )
    );
}

#[tokio::test]
async fn attachment_logging_call_id() {
    let mut fixture = Fixture::new();
    fixture
        .kwargs
        .insert(String::from("litellm_call_id"), json(r#""call-a""#));
    let spy = Spy::replying(file());
    let seen = recorded(Which::List, &spy, &fixture, None, REQUEST_TIMEOUT).await;
    assert_eq!(
        seen.litellm_params.get("litellm_call_id"),
        Some(&json(r#""call-a""#))
    );
}

#[tokio::test]
async fn attachment_list_logging_fields() {
    let mut fixture = Fixture::new();
    fixture.after = Some(String::from("file_a"));
    fixture.before = Some(String::from("file_z"));
    fixture.filter = Some(String::from("failed"));
    fixture.limit = Some(25);
    fixture.order = Some(String::from("desc"));
    let spy = Spy::replying(listing());
    let seen = recorded(Which::List, &spy, &fixture, None, REQUEST_TIMEOUT).await;
    assert_eq!(
        seen.optional_params,
        object(
            r#"{"vector_store_id":"vs_a","after":"file_a","before":"file_z","filter":"failed","limit":25,"order":"desc"}"#
        )
    );
    assert_eq!(
        seen.fields,
        params(
            r#"{"after":"file_a","before":"file_z","filter":"failed","limit":25,"order":"desc"}"#
        )
    );
}

#[tokio::test]
async fn attachment_async_flag_selection() {
    let mut fixture = Fixture::new();
    fixture.kwargs.insert(String::from("acreate"), json("true"));
    let flag_spy = Spy::replying(file());
    let flagged = recorded(Which::Create, &flag_spy, &fixture, None, REQUEST_TIMEOUT).await;
    assert_eq!(flagged.mode, ClientKind::Asynchronous);
    assert!(!flagged.litellm_params.contains_key("acreate"));

    fixture.kwargs.insert(String::from("acreate"), json("1"));
    let integer_spy = Spy::replying(file());
    let integer = recorded(Which::Create, &integer_spy, &fixture, None, REQUEST_TIMEOUT).await;
    assert_eq!(integer.mode, ClientKind::Sync);

    fixture.kwargs.shift_remove("acreate");
    fixture.kwargs.insert(String::from("alist"), json("1"));
    let list_integer_spy = Spy::replying(listing());
    let list_integer = recorded(
        Which::List,
        &list_integer_spy,
        &fixture,
        None,
        REQUEST_TIMEOUT,
    )
    .await;
    assert_eq!(list_integer.mode, ClientKind::Sync);

    fixture.kwargs.insert(String::from("alist"), json("true"));
    let list_flag_spy = Spy::replying(listing());
    let list_flag = recorded(Which::List, &list_flag_spy, &fixture, None, REQUEST_TIMEOUT).await;
    assert_eq!(list_flag.mode, ClientKind::Asynchronous);
    assert!(!list_flag.litellm_params.contains_key("alist"));
}
