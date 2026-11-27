use betterlite_context::resource_config_selection::vector_stores::VectorStoreFilesConfigTag;
use betterlite_models::identity::provider_id::ProviderId;

use super::super::service::Operation;
use super::support::{
    Fixture, REQUEST_TIMEOUT, Spy, Which, attributes, call, chunking, file, json, object, params,
    recorded, refused,
};
use crate::api::vector_store_http_write::service::ClientKind;

#[tokio::test]
async fn attach_file_projection() {
    let mut fixture = Fixture::new();
    fixture.attributes = Some(attributes(r#"{"tag":"x","count":2}"#));
    fixture.chunking_strategy = Some(chunking());
    fixture.extra_query = Some(object(r#"{"ignored":"y"}"#));
    fixture.extra_body = Some(object(r#"{"file_id":"file_b"}"#));
    fixture.kwargs.insert(String::from("unknown"), json("1"));
    let spy = Spy::replying(file());
    let seen = recorded(Which::Create, &spy, &fixture, None, REQUEST_TIMEOUT).await;
    assert_eq!(seen.operation, Operation::Create);
    assert_eq!(seen.mode, ClientKind::Sync);
    assert_eq!(seen.provider, ProviderId::Openai);
    assert_eq!(seen.config, VectorStoreFilesConfigTag::OpenAI);
    assert_eq!(seen.vector_store_id, "vs_a");
    assert_eq!(
        seen.fields,
        params(
            r#"{"file_id":"file_a","attributes":{"tag":"x","count":2},"chunking_strategy":{"type":"static","static":{"max_chunk_size_tokens":800,"chunk_overlap_tokens":400}}}"#
        )
    );
    assert_eq!(seen.extra_query, None);
    assert_eq!(seen.extra_body, Some(object(r#"{"file_id":"file_b"}"#)));
    let answer = call(Which::Create, &spy, &fixture, None, REQUEST_TIMEOUT)
        .await
        .expect("the attachment call answers");
    assert_eq!(answer, file());
}

#[tokio::test]
async fn attach_file_async_projection() {
    let mut fixture = Fixture::new();
    fixture.attributes = Some(attributes(r#"{"tag":"x","count":2}"#));
    fixture.chunking_strategy = Some(chunking());
    fixture.extra_query = Some(object(r#"{"ignored":"y"}"#));
    fixture.extra_body = Some(object(r#"{"file_id":"file_b"}"#));
    fixture.kwargs.insert(String::from("unknown"), json("1"));
    let spy = Spy::replying(file());
    let seen = recorded(Which::Acreate, &spy, &fixture, None, REQUEST_TIMEOUT).await;
    assert_eq!(seen.operation, Operation::Create);
    assert_eq!(seen.mode, ClientKind::Asynchronous);
    assert_eq!(
        seen.fields,
        params(
            r#"{"file_id":"file_a","attributes":{"tag":"x","count":2},"chunking_strategy":{"type":"static","static":{"max_chunk_size_tokens":800,"chunk_overlap_tokens":400}}}"#
        )
    );
    assert_eq!(seen.extra_query, None);
    assert_eq!(seen.extra_body, Some(object(r#"{"file_id":"file_b"}"#)));
    assert!(!seen.litellm_params.contains_key("acreate"));
    assert_eq!(spy.call_count(), 1);
}

#[tokio::test]
async fn attach_file_missing_optionals() {
    let fixture = Fixture::new();
    let spy = Spy::replying(file());
    let seen = recorded(Which::Create, &spy, &fixture, None, REQUEST_TIMEOUT).await;
    assert_eq!(seen.fields, params(r#"{"file_id":"file_a"}"#));
    assert_eq!(seen.extra_body, None);
    assert_eq!(seen.extra_query, None);
    let answer = call(Which::Create, &spy, &fixture, None, REQUEST_TIMEOUT)
        .await
        .expect("the attachment call answers");
    assert_eq!(answer, file());
}

#[tokio::test]
async fn attach_file_empty_attributes() {
    let mut fixture = Fixture::new();
    fixture.attributes = Some(attributes("{}"));
    let spy = Spy::replying(file());
    let seen = recorded(Which::Create, &spy, &fixture, None, REQUEST_TIMEOUT).await;
    assert_eq!(
        seen.fields,
        params(r#"{"file_id":"file_a","attributes":{}}"#)
    );
}

#[tokio::test]
async fn attach_file_field_types() {
    let mut fixture = Fixture::new();
    fixture
        .kwargs
        .insert(String::from("api_key"), json(r#""sk-caller""#));
    fixture
        .kwargs
        .insert(String::from("ssl_verify"), json(r#""yes""#));
    let spy = Spy::replying(file());
    let failure = refused(Which::Create, &spy, &fixture, None).await;
    assert_eq!(failure.class, "APIConnectionError");
    assert_eq!(failure.status, 500);
    assert_eq!(failure.message, "the ssl_verify field must be a boolean");
    assert_eq!(spy.call_count(), 0);
}
