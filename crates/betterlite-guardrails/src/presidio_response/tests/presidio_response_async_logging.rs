#![cfg(test)]

use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse as _, Response};
use axum::routing::post;
use serde::Deserialize;
use tokio::net::TcpListener;
use tokio::sync::Barrier;
use tokio::task::JoinSet;
use tokio::time::timeout;

use super::shared::{
    analyzer, anonymizer, anthropic_texts, chat_content, content_text, context, logged_count,
    messages, request_data, response_body,
};
use crate::presidio_response::error::PresidioResponseError;
use crate::presidio_response::logging::async_logging_hook;
use crate::presidio_response::wire::{ChatMessage, Content, ContentItem};

const DETECTIONS: &str = r#"[{"entity_type":"PERSON","start":0,"end":5,"score":0.9}]"#;
const MASKED_REPLY: &str = r#"{"text":"MASKED","items":[{"entity_type":"PERSON"}]}"#;

#[derive(Clone)]
struct MockState {
    barrier: Arc<Barrier>,
    seen: Arc<Mutex<Vec<String>>>,
}

#[derive(Deserialize)]
struct SeenAnalyze {
    text: String,
}

async fn analyze(State(state): State<MockState>, body: Bytes) -> Response {
    if let Ok(seen) = serde_json::from_slice::<SeenAnalyze>(&body) {
        state.seen.lock().expect("seen requests").push(seen.text);
    }
    if timeout(Duration::from_secs(2), state.barrier.wait())
        .await
        .is_err()
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "the checks ran one at a time",
        )
            .into_response();
    }
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        DETECTIONS,
    )
        .into_response()
}

async fn anonymize() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        MASKED_REPLY,
    )
        .into_response()
}

async fn failing_analyze() -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, "boom").into_response()
}

async fn spawn_mock(
    concurrent: usize,
) -> (String, Arc<Mutex<Vec<String>>>, JoinSet<io::Result<()>>) {
    let state = MockState {
        barrier: Arc::new(Barrier::new(concurrent)),
        seen: Arc::new(Mutex::new(Vec::new())),
    };
    let seen = Arc::clone(&state.seen);
    let router = Router::new()
        .route("/analyze", post(analyze))
        .route("/anonymize", post(anonymize))
        .with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("address"));
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (base, seen, server)
}

async fn spawn_failing_mock() -> (String, JoinSet<io::Result<()>>) {
    let router = Router::new()
        .route("/analyze", post(failing_analyze))
        .route("/anonymize", post(anonymize));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("address"));
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (base, server)
}

fn assert_masked_parts(parsed: &[ChatMessage]) {
    assert_eq!(parsed.len(), 2);
    assert_eq!(
        content_text(parsed[0].content.as_ref()).as_deref(),
        Some("MASKED")
    );
    let Some(Content::Blocks(items)) = parsed[1].content.as_ref() else {
        panic!("the second message keeps a content list");
    };
    assert_eq!(items.len(), 5);
    let ContentItem::Object(first) = &items[0] else {
        panic!("the first part is an object");
    };
    assert_eq!(first.text.as_deref(), Some("MASKED"));
    let ContentItem::Object(image) = &items[1] else {
        panic!("the image part is an object");
    };
    assert_eq!(image.kind.as_deref(), Some("image_url"));
    assert!(image.text.is_none());
    let ContentItem::Object(empty) = &items[2] else {
        panic!("the textless part is an object");
    };
    assert!(empty.text.is_none());
    let ContentItem::Object(number_text) = &items[3] else {
        panic!("the number text part is an object");
    };
    assert!(number_text.text.is_none());
    assert_eq!(
        number_text.extra.get("text").map(|text| text.get()),
        Some("5")
    );
    assert!(matches!(items[4], ContentItem::Other(_)));
}

#[tokio::test]
async fn masks_string_content_and_unsupported_parts() {
    let analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut request = request_data(
        r#"{"messages":[{"role":"user","content":"Alice"},{"role":"user","content":[{"type":"text","text":"Bobby"},{"type":"image_url","image_url":{"url":"x"}},{"type":"text"},{"text":5},"bare"]}]}"#,
    );
    let mut response = response_body(r#"{"choices":[]}"#);

    async_logging_hook(context, &mut request, &mut response, "completion")
        .await
        .expect("logging hook");

    assert_masked_parts(&messages(&request));
}

#[tokio::test]
async fn a_missing_messages_field_leaves_the_response_untouched() {
    let analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut request = request_data(r#"{"model":"gpt"}"#);
    let mut response =
        response_body(r#"{"choices":[{"message":{"role":"assistant","content":"Bob"}}]}"#);

    async_logging_hook(context, &mut request, &mut response, "acompletion")
        .await
        .expect("logging hook");

    assert_eq!(chat_content(&response).as_deref(), Some("Bob"));
}

#[tokio::test]
async fn masks_the_model_response_after_the_messages() {
    let analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut request = request_data(r#"{"messages":[{"role":"user","content":"Alice"}]}"#);
    let mut response =
        response_body(r#"{"choices":[{"message":{"role":"assistant","content":"Bob"}}]}"#);

    async_logging_hook(context, &mut request, &mut response, "acompletion")
        .await
        .expect("logging hook");

    assert_eq!(chat_content(&response).as_deref(), Some("MASKED"));
    assert_eq!(logged_count(&request), 2);
}

#[tokio::test]
async fn masks_an_anthropic_message_response() {
    let analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut request = request_data(r#"{"messages":[{"role":"user","content":"Alice"}]}"#);
    let mut response = response_body(
        r#"{"type":"message","content":[{"type":"text","text":"Bob"},{"type":"tool_use","id":"call-1"}]}"#,
    );

    async_logging_hook(context, &mut request, &mut response, "acompletion")
        .await
        .expect("logging hook");

    assert_eq!(anthropic_texts(&response), vec!["MASKED".to_owned()]);
}

#[tokio::test]
async fn gathers_the_message_checks_concurrently() {
    let (base, seen, _server) = spawn_mock(2).await;
    let analyzer = analyzer(&base, None);
    let anonymizer = anonymizer(&base);
    let context = context(&analyzer, &anonymizer);
    let mut request = request_data(
        r#"{"messages":[{"role":"user","content":"Alice"},{"role":"user","content":"Bobby"}],"metadata":{}}"#,
    );
    let mut response = response_body(r#"{"choices":[]}"#);

    async_logging_hook(context, &mut request, &mut response, "acompletion")
        .await
        .expect("both checks pass the barrier");

    let parsed = messages(&request);
    assert_eq!(
        content_text(parsed[0].content.as_ref()).as_deref(),
        Some("MASKED")
    );
    assert_eq!(
        content_text(parsed[1].content.as_ref()).as_deref(),
        Some("MASKED")
    );
    assert_eq!(seen.lock().expect("seen requests").len(), 2);
    assert_eq!(logged_count(&request), 2);
}

#[tokio::test]
async fn a_failed_check_leaves_the_messages_unmasked() {
    let (base, _server) = spawn_failing_mock().await;
    let mut analyzer = analyzer(&base, None);
    analyzer.settings.pii_entities = vec!["PERSON".to_owned()];
    let anonymizer = anonymizer(&base);
    let context = context(&analyzer, &anonymizer);
    let mut request = request_data(r#"{"messages":[{"role":"user","content":"Alice"}]}"#);
    let mut response = response_body(r#"{"choices":[]}"#);

    let error = async_logging_hook(context, &mut request, &mut response, "acompletion")
        .await
        .expect_err("the analyzer failure fails the check");

    assert!(matches!(error, PresidioResponseError::Actions(_)));
    assert_eq!(
        content_text(messages(&request)[0].content.as_ref()).as_deref(),
        Some("Alice")
    );
    assert_eq!(logged_count(&request), 1);
}
