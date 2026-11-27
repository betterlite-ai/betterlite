use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_file_contracts::records::Attributes;
use serde_json::Number;

use super::support::{
    Fixture, REQUEST_TIMEOUT, Spy, Which, attributes, call, recorded, replacement_body, success,
};
use crate::api::vector_store_http_write::service::{ClientKind, Operation};

#[tokio::test]
async fn update_sends_the_attributes_argument() {
    let spy = Spy::replying(success());
    let mut fixture = Fixture::new();
    fixture.arguments.attributes = Some(attributes());
    fixture
        .arguments
        .kwargs
        .insert(String::from("unknown"), Json::Number(Number::from(1_u32)));
    let seen = recorded(Which::Update, &spy, &fixture, REQUEST_TIMEOUT).await;
    assert_eq!(seen.operation, Operation::Update);
    // the unknown kwarg is not an update field: B1071 keeps only attributes
    assert_eq!(
        seen.update,
        r#"{"attributes":{"count":2,"enabled":false,"tag":"x"}}"#
    );
    assert_eq!(seen.extra_body, None);
}

#[tokio::test]
async fn update_keeps_the_empty_and_null_attributes() {
    let spy = Spy::replying(success());
    let mut empty = Fixture::new();
    empty.arguments.attributes = Some(Attributes::new());
    let kept = recorded(Which::Update, &spy, &empty, REQUEST_TIMEOUT).await;
    assert_eq!(kept.update, r#"{"attributes":{}}"#);
    // the filter would have dropped a null, and the explicit argument puts it back
    let bare = Fixture::new();
    let restored = recorded(Which::Update, &spy, &bare, REQUEST_TIMEOUT).await;
    assert_eq!(restored.update, r#"{"attributes":null}"#);
}

#[tokio::test]
async fn extra_body_replaces_the_map_downstream() {
    let spy = Spy::replying(success());
    let mut fixture = Fixture::new();
    fixture.arguments.attributes = Some(attributes());
    fixture.arguments.extra_body = Some(replacement_body());
    let seen = recorded(Which::Update, &spy, &fixture, REQUEST_TIMEOUT).await;
    // the argument is still the body request's map; B2482 overlays extra_body on the wire
    assert_eq!(
        seen.update,
        r#"{"attributes":{"count":2,"enabled":false,"tag":"x"}}"#
    );
    assert_eq!(seen.extra_body, Some(replacement_body()));
}

#[tokio::test]
async fn both_modes_return_the_provider_answer() {
    let spy = Spy::replying(success());
    let fixture = Fixture::new();
    let direct = call(Which::Update, &spy, &fixture, REQUEST_TIMEOUT)
        .await
        .expect("the update answers");
    assert_eq!(direct, success());
    let awaited = call(Which::Aupdate, &spy, &fixture, REQUEST_TIMEOUT)
        .await
        .expect("the aupdate answers");
    assert_eq!(awaited, success());
    let seen = spy.seen().expect("the service saw the write");
    assert_eq!(seen.mode, ClientKind::Asynchronous);
    assert_eq!(spy.call_count(), 2);
}
