use betterlite_context::resource_config_selection::vector_stores::VectorStoresConfigTag;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use indexmap::IndexMap;
use serde_json::Number;

use super::super::service::Operation;
use super::support::{
    Fixture, REQUEST_TIMEOUT, Resolution, Spy, VECTOR_STORE_ID, auto_chunking, call_update,
    expires_after, metadata, success,
};
use crate::api::vector_store_http_write::params::UpdateParams;
use crate::api::vector_store_http_write::service::ClientKind;

// the kwargs an update may carry: two create fields the signature has no argument for, and one unknown key
fn create_fields_kwargs() -> IndexMap<String, Json> {
    IndexMap::from([
        (
            String::from("file_ids"),
            Json::List(vec![Json::Text(String::from("file_a"))]),
        ),
        (
            String::from("chunking_strategy"),
            Json::Object(IndexMap::from([(
                String::from("type"),
                Json::Text(String::from("auto")),
            )])),
        ),
        (String::from("unknown"), Json::Number(Number::from(1_u32))),
    ])
}

fn full_fixture() -> Fixture {
    let mut fixture = Fixture::new();
    fixture.name = Some(String::from("notes"));
    fixture.expires_after = Some(expires_after());
    fixture.metadata = Some(metadata());
    fixture.kwargs = create_fields_kwargs();
    fixture
}

// the create-parameter filter keeps the five recognized fields, metadata's count included: B2402/B668 cleans metadata
fn expected_optional() -> UpdateParams {
    UpdateParams {
        name: Some(Some(String::from("notes"))),
        file_ids: Some(Some(vec![String::from("file_a")])),
        expires_after: Some(Some(expires_after())),
        chunking_strategy: Some(Some(auto_chunking())),
        metadata: Some(Some(metadata())),
    }
}

#[tokio::test]
async fn update_accepts_create_fields() {
    let spy = Spy::replying(success());
    let resolution = Resolution::new();
    let fixture = full_fixture();
    let value = call_update(&spy, &fixture, &resolution, REQUEST_TIMEOUT)
        .await
        .expect("the mock answer is returned");
    assert_eq!(value, success());
    let seen = spy.seen().expect("the service saw the write");
    assert_eq!(seen.operation, Operation::Update);
    assert_eq!(seen.provider, ProviderId::Openai);
    assert_eq!(seen.config, VectorStoresConfigTag::OpenAI);
    assert_eq!(seen.vector_store_id, VECTOR_STORE_ID);
    // unknown and vector_store_id are not UpdateParams fields, and serde refuses unknown ones outright
    assert_eq!(seen.optional_params, expected_optional());
}

#[tokio::test]
async fn update_accepts_create_fields_asynchronously() {
    let spy = Spy::replying(success());
    let resolution = Resolution::new();
    let mut fixture = full_fixture();
    fixture
        .kwargs
        .insert(String::from("aupdate"), Json::Bool(true));
    let value = call_update(&spy, &fixture, &resolution, REQUEST_TIMEOUT)
        .await
        .expect("the mock answer is returned");
    assert_eq!(value, success());
    let seen = spy.seen().expect("the service saw the write");
    assert_eq!(seen.mode, ClientKind::Asynchronous);
    assert_eq!(seen.optional_params, expected_optional());
}

#[tokio::test]
async fn update_keeps_empty_name_and_metadata() {
    let spy = Spy::replying(success());
    let resolution = Resolution::new();
    let mut fixture = Fixture::new();
    fixture.name = Some(String::new());
    fixture.metadata = Some(IndexMap::new());
    call_update(&spy, &fixture, &resolution, REQUEST_TIMEOUT)
        .await
        .expect("the mock answer is returned");
    let seen = spy.seen().expect("the service saw the write");
    assert_eq!(
        seen.optional_params,
        UpdateParams {
            name: Some(Some(String::new())),
            file_ids: None,
            expires_after: None,
            chunking_strategy: None,
            metadata: Some(Some(IndexMap::new())),
        }
    );
}

#[tokio::test]
async fn update_drops_null_fields() {
    let spy = Spy::replying(success());
    let resolution = Resolution::new();
    let mut fixture = Fixture::new();
    fixture.kwargs.insert(String::from("name"), Json::Null);
    fixture.kwargs.insert(String::from("metadata"), Json::Null);
    call_update(&spy, &fixture, &resolution, REQUEST_TIMEOUT)
        .await
        .expect("the mock answer is returned");
    let seen = spy.seen().expect("the service saw the write");
    assert_eq!(seen.optional_params, UpdateParams::default());
}
