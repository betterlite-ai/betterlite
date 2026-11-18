use super::{Counter, FakeTransport, TestCredentials, Workspace, runtime, timeout_failure};
use crate::sagemaker_runtime::{chat, completion};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::sagemaker_completion::params::Options;
use serde_json::json;

#[tokio::test]
async fn completion_reports_a_transport_timeout_as_a_500() {
    let counter = Counter;
    let workspace = Workspace::new();
    let failure = completion::completion(
        &runtime(
            FakeTransport::failure(timeout_failure().await),
            TestCredentials::working(),
            &counter,
        ),
        &workspace.completion(),
    )
    .await
    .expect_err("timeout status");
    assert_eq!(failure.status_code(), Some(500));
}

#[tokio::test]
async fn completion_stream_reports_a_transport_timeout_as_a_408() {
    let counter = Counter;
    let mut workspace = Workspace::new();
    workspace.options = Options::from([(String::from("stream"), json!(true))]);
    let error = completion::completion_stream(
        &runtime(
            FakeTransport::failure(timeout_failure().await),
            TestCredentials::working(),
            &counter,
        ),
        &workspace.completion(),
    )
    .await;
    assert!(
        matches!(error, Err(failure) if failure.status_code == 408 && failure.message == "Timeout error occurred.")
    );
}

#[tokio::test]
async fn chat_reports_a_transport_timeout_as_a_408() {
    let counter = Counter;
    let workspace = Workspace::new();
    let error = chat::chat(
        &runtime(
            FakeTransport::failure(timeout_failure().await),
            TestCredentials::working(),
            &counter,
        ),
        &workspace.chat(false),
    )
    .await
    .expect_err("timeout status");
    assert_eq!(error.status_code, 408);
}

#[tokio::test]
async fn chat_reports_other_transport_failures_as_a_500() {
    let counter = Counter;
    let workspace = Workspace::new();
    let error = chat::chat(
        &runtime(
            FakeTransport::failure(TransportError::Closed),
            TestCredentials::working(),
            &counter,
        ),
        &workspace.chat(false),
    )
    .await
    .expect_err("closed transport");
    assert_eq!(error.status_code, 500);
}
