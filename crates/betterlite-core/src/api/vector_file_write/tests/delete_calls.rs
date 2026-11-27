use std::time::Duration;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::support::{
    FILE_ID, Fixture, REQUEST_TIMEOUT, Registry, Spy, VECTOR_STORE_ID, Which, call, deleted,
    kept_headers, recorded,
};
use crate::api::vector_store_http_write::service::Operation;

fn ignored() -> Json {
    Json::Object(IndexMap::from([(
        String::from("ignored"),
        Json::Bool(true),
    )]))
}

#[tokio::test]
async fn delete_ignores_extra_body_and_query() {
    let spy = Spy::replying(deleted());
    let mut fixture = Fixture::new();
    fixture.arguments.extra_headers = Some(kept_headers());
    fixture
        .arguments
        .kwargs
        .insert(String::from("extra_body"), ignored());
    fixture
        .arguments
        .kwargs
        .insert(String::from("extra_query"), ignored());
    fixture.registry = Some(Registry::replying(IndexMap::new()));
    let seen = recorded(Which::Delete, &spy, &fixture, REQUEST_TIMEOUT).await;
    assert_eq!(seen.operation, Operation::Delete);
    // no body, no attribute and no query: the delete handler takes ids, headers and a timeout only
    assert_eq!(seen.update, "{}");
    assert_eq!(seen.extra_body, None);
    assert_eq!(seen.extra_headers, Some(kept_headers()));
    assert_eq!(seen.timeout, Duration::from_mins(100));
    assert_eq!(spy.call_count(), 1);
    // the registry is read once and left as it was
    assert_eq!(
        fixture.registry.as_ref().map(Registry::lookup_count),
        Some(1)
    );
}

#[tokio::test]
async fn delete_returns_the_provider_answer() {
    let spy = Spy::replying(deleted());
    let fixture = Fixture::new();
    let direct = call(Which::Delete, &spy, &fixture, REQUEST_TIMEOUT)
        .await
        .expect("the delete answers");
    assert_eq!(direct, deleted());
    let awaited = call(Which::Adelete, &spy, &fixture, REQUEST_TIMEOUT)
        .await
        .expect("the adelete answers");
    assert_eq!(awaited, deleted());
    assert_eq!(spy.call_count(), 2);
}

#[tokio::test]
async fn delete_logs_the_ids_alone() {
    let spy = Spy::replying(deleted());
    let fixture = Fixture::new();
    let seen = recorded(Which::Delete, &spy, &fixture, REQUEST_TIMEOUT).await;
    assert_eq!(
        seen.optional,
        IndexMap::from([
            (
                String::from("vector_store_id"),
                Json::Text(String::from(VECTOR_STORE_ID)),
            ),
            (String::from("file_id"), Json::Text(String::from(FILE_ID)),),
        ])
    );
    assert_eq!(
        seen.litellm_params.get("litellm_call_id"),
        Some(&Json::Null)
    );
}
