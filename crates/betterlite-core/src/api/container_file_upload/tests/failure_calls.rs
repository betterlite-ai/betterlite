#![cfg(test)]

use betterlite_error::error::ErrorClass;

use super::error::{ContainerFileUploadError, failure};
use super::test_support::{
    CONTAINER_ID, Fixture, FixtureFailure, StubExecutor, object, redactor, timeout_failure,
};
use super::upload_container_file;
use crate::api::response_id_codecs::container::build_container_id;

#[test]
fn provider_failure_keeps_its_status() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let executor = StubExecutor::failing(FixtureFailure::Provider { status: 404 });

    let error = upload_container_file(&executor, fixture.deps(), &request)
        .expect_err("the provider answered 404");

    let ErrorClass::NotFound(details) = &error.class else {
        panic!("expected a not found, got {:?}", error.class);
    };
    assert_eq!(details.status.0, 404);
    assert_eq!(details.model.as_deref(), Some(""));
    assert_eq!(details.provider.as_deref(), Some("openai"));
    assert_eq!(executor.call_count(), 1);
}

#[test]
fn failure_without_a_status_is_an_api_connection() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let executor = StubExecutor::failing(FixtureFailure::Transport);

    let error = upload_container_file(&executor, fixture.deps(), &request)
        .expect_err("the connection drops");

    let ErrorClass::ApiConnection(context) = &error.class else {
        panic!("expected an api connection, got {:?}", error.class);
    };
    assert_eq!(context.provider.as_deref(), Some("openai"));
    assert_eq!(context.model.as_deref(), Some(""));
    assert_eq!(executor.call_count(), 1);
}

#[tokio::test]
async fn a_request_timeout_is_a_timeout_failure() {
    let timeout = timeout_failure().await;

    let error = failure(
        ContainerFileUploadError::Provider(timeout),
        "openai",
        &redactor(),
    );

    let ErrorClass::Timeout(details) = &error.class else {
        panic!("expected a timeout, got {:?}", error.class);
    };
    assert_eq!(details.original_status.0, 408);
    assert_eq!(details.context.provider.as_deref(), Some("openai"));
    assert_eq!(details.context.model.as_deref(), Some(""));
}

#[test]
fn failure_carries_the_provider_the_managed_id_resolved_to() {
    let mut fixture = Fixture::new();
    fixture.container_id = build_container_id(Some("azure"), None, CONTAINER_ID);
    let request = fixture.request();
    let executor = StubExecutor::failing(FixtureFailure::Transport);

    let error = upload_container_file(&executor, fixture.deps(), &request)
        .expect_err("the connection drops");

    let ErrorClass::ApiConnection(context) = &error.class else {
        panic!("expected an api connection, got {:?}", error.class);
    };
    assert_eq!(context.provider.as_deref(), Some("azure"));
    assert_eq!(executor.call_count(), 1);
}

#[test]
fn upload_a_known_provider_without_a_container_config_is_a_bad_request() {
    let mut fixture = Fixture::new();
    fixture.provider = Some("anthropic".to_owned());
    let request = fixture.request();
    let executor = StubExecutor::replying(object("file_1"));

    let error = upload_container_file(&executor, fixture.deps(), &request)
        .expect_err("anthropic has no container config");

    assert_eq!(
        error.error.to_string(),
        "Container provider config not found for provider: anthropic"
    );
    let ErrorClass::BadRequest(details) = &error.class else {
        panic!("expected a bad request, got {:?}", error.class);
    };
    assert_eq!(details.status.0, 400);
    assert_eq!(
        details.message,
        "Container provider config not found for provider: anthropic"
    );
    assert_eq!(details.provider.as_deref(), Some("anthropic"));
    assert_eq!(details.model.as_deref(), Some(""));
    assert_eq!(executor.call_count(), 0);
}
