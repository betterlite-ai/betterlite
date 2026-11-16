#![cfg(test)]

use betterlite_error::error::ErrorClass;

use super::ENDPOINT_NAME;
use super::aupload_container_file;
use super::test_support::{CONTAINER_ID, Fixture, FixtureFailure, StubExecutor, object};
use crate::api::response_id_codecs::container::build_container_id;

#[tokio::test]
async fn async_upload_returns_the_uploaded_object() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let executor = StubExecutor::replying(object("file_1"));

    let reply = aupload_container_file(&executor, fixture.deps(), &request)
        .await
        .expect("the upload succeeds");

    assert_eq!(reply, object("file_1"));
    let seen = executor.seen().expect("the executor ran");
    assert_eq!(seen.endpoint_name, ENDPOINT_NAME);
    assert_eq!(seen.call_type, "upload_container_file");
    assert_eq!(seen.container_id, CONTAINER_ID);
}

#[tokio::test]
async fn async_upload_maps_a_provider_failure_with_its_status() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let executor = StubExecutor::failing(FixtureFailure::Provider { status: 404 });

    let error = aupload_container_file(&executor, fixture.deps(), &request)
        .await
        .expect_err("the provider answered 404");

    let ErrorClass::NotFound(details) = &error.class else {
        panic!("expected a not found, got {:?}", error.class);
    };
    assert_eq!(details.status.0, 404);
    assert_eq!(details.model.as_deref(), Some(""));
    assert_eq!(details.provider.as_deref(), Some("openai"));
}

#[tokio::test]
async fn async_upload_uses_the_decoded_provider_for_failures() {
    let mut fixture = Fixture::new();
    fixture.container_id = build_container_id(Some("azure"), None, CONTAINER_ID);
    let request = fixture.request();
    let executor = StubExecutor::failing(FixtureFailure::Transport);

    let error = aupload_container_file(&executor, fixture.deps(), &request)
        .await
        .expect_err("the connection drops");

    let ErrorClass::ApiConnection(context) = &error.class else {
        panic!("expected an api connection, got {:?}", error.class);
    };
    assert_eq!(context.provider.as_deref(), Some("azure"));
    assert_eq!(context.model.as_deref(), Some(""));
}
