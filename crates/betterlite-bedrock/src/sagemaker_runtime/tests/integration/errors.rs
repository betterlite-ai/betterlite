use super::{Counter, FakeTransport, TestCredentials, Workspace, classify, runtime, upstream};
use crate::sagemaker_runtime::{chat, completion, embedding};
use betterlite_error::error::ErrorClass;
use betterlite_openai::sagemaker_completion::params::Options;
use serde_json::json;

#[tokio::test]
async fn buffered_calls_preserve_error_status() {
    let counter = Counter;
    let workspace = Workspace::new();
    let completion = upstream(
        completion::completion(
            &runtime(
                FakeTransport::new(429, "throttled", &[]),
                TestCredentials::working(),
                &counter,
            ),
            &workspace.completion(),
        )
        .await
        .expect_err("completion status"),
    );
    assert_eq!(
        (completion.status_code, completion.message.as_str()),
        (429, "throttled")
    );

    let chat = chat::chat(
        &runtime(
            FakeTransport::new(429, "chat throttled", &[]),
            TestCredentials::working(),
            &counter,
        ),
        &workspace.chat(false),
    )
    .await
    .expect_err("chat status");
    assert_eq!(
        (chat.status_code, chat.message.as_str()),
        (429, "chat throttled")
    );
}

#[tokio::test]
async fn stream_calls_preserve_error_status() {
    let counter = Counter;
    let mut workspace = Workspace::new();
    workspace.options = Options::from([(String::from("stream"), json!(true))]);
    let completion_stream = completion::completion_stream(
        &runtime(
            FakeTransport::new(201, "", &[b"failed"]),
            TestCredentials::working(),
            &counter,
        ),
        &workspace.completion(),
    )
    .await;
    assert!(
        matches!(completion_stream, Err(error) if error.status_code == 201 && error.message == "failed")
    );

    let chat_stream = chat::chat_stream(
        &runtime(
            FakeTransport::new(201, "", &[b"failed"]),
            TestCredentials::working(),
            &counter,
        ),
        &workspace.chat(true),
    )
    .await;
    assert!(
        matches!(chat_stream, Err(error) if error.status_code == 201 && error.message == "failed")
    );
}

#[tokio::test]
async fn embedding_preserves_wire_error() {
    let counter = Counter;
    let workspace = Workspace::new();
    let embedding = upstream(
        embedding::embedding(
            &runtime(
                FakeTransport::new(503, "embedding failed", &[]),
                TestCredentials::working(),
                &counter,
            ),
            &workspace.embedding("voyage-3"),
        )
        .await
        .expect_err("embedding status"),
    );
    assert_eq!(
        (embedding.status_code, embedding.message.as_str()),
        (503, "embedding failed")
    );
}

#[tokio::test]
async fn unreadable_replies_classify_as_connection_errors() {
    let counter = Counter;
    let workspace = Workspace::new();
    let completion = completion::completion(
        &runtime(
            FakeTransport::new(200, "not-json", &[]),
            TestCredentials::working(),
            &counter,
        ),
        &workspace.completion(),
    )
    .await
    .expect_err("completion reply");
    assert_eq!(completion.status_code(), None);
    assert!(matches!(
        classify(&completion),
        ErrorClass::ApiConnection(_)
    ));
    let embedding = embedding::embedding(
        &runtime(
            FakeTransport::new(200, "not-json", &[]),
            TestCredentials::working(),
            &counter,
        ),
        &workspace.embedding("voyage-3"),
    )
    .await
    .expect_err("embedding reply");
    assert_eq!(embedding.status_code(), None);
    assert!(matches!(classify(&embedding), ErrorClass::ApiConnection(_)));
}

#[tokio::test]
async fn wire_failures_classify_as_unavailable() {
    let counter = Counter;
    let workspace = Workspace::new();
    let wire = completion::completion(
        &runtime(
            FakeTransport::new(500, "boom", &[]),
            TestCredentials::working(),
            &counter,
        ),
        &workspace.completion(),
    )
    .await
    .expect_err("wire status");
    assert_eq!(wire.status_code(), Some(500));
    assert!(matches!(classify(&wire), ErrorClass::ServiceUnavailable(_)));
}
