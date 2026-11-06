#![cfg(test)]

use std::io;

use axum::Router;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use axum::routing::post;
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use super::shared::{
    analyzer, anonymizer, anthropic_texts, chat_content, content_text, context, logged_count,
    logged_entity_types, logged_names, messages, request_data, response_body, spawn_mock,
};
use crate::presidio_response::error::PresidioResponseError;
use crate::presidio_response::logging::async_logging_hook;
use crate::presidio_response::wire::{ChatMessage, Content, ContentItem};

async fn failing_analyze() -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, "boom").into_response()
}

async fn spawn_failing_mock() -> (String, JoinSet<io::Result<()>>) {
    let router = Router::new().route("/analyze", post(failing_analyze));
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
async fn a_null_messages_field_leaves_the_response_untouched() {
    let analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut request = request_data(r#"{"messages":null}"#);
    let mut response =
        response_body(r#"{"choices":[{"message":{"role":"assistant","content":"Bob"}}]}"#);

    async_logging_hook(context, &mut request, &mut response, "acompletion")
        .await
        .expect("a null messages field is not an error");

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
    let mock = spawn_mock(2).await;
    let analyzer = analyzer(&mock.base, None);
    let anonymizer = anonymizer(&mock.base);
    let context = context(&analyzer, &anonymizer);
    let mut request = request_data(
        r#"{"messages":[{"role":"user","content":"Alice"},{"role":"user","content":"Bobby"}],"metadata":{"guardrail_config":{"language":"de"}}}"#,
    );
    let mut response = response_body(r#"{"choices":[]}"#);

    async_logging_hook(context, &mut request, &mut response, "acompletion")
        .await
        .expect("both checks pass the barrier");

    let parsed = messages(&request);
    assert_eq!(
        content_text(parsed[0].content.as_ref()).as_deref(),
        Some("MASKED-Alice")
    );
    assert_eq!(
        content_text(parsed[1].content.as_ref()).as_deref(),
        Some("MASKED-Bobby")
    );
    let seen_requests = mock.seen.lock().expect("seen requests").clone();
    assert_eq!(seen_requests.len(), 2);
    assert!(
        seen_requests
            .iter()
            .all(|analyze| analyze.language.as_deref() == Some("de"))
    );
    let mut texts: Vec<&str> = seen_requests
        .iter()
        .map(|analyze| analyze.text.as_str())
        .collect();
    texts.sort_unstable();
    assert_eq!(texts, vec!["Alice", "Bobby"]);
    assert_eq!(logged_count(&request), 2);
}

#[tokio::test]
async fn maps_each_masked_text_back_to_its_message_and_part() {
    let mock = spawn_mock(3).await;
    let analyzer = analyzer(&mock.base, None);
    let anonymizer = anonymizer(&mock.base);
    let context = context(&analyzer, &anonymizer);
    let mut request = request_data(
        r#"{"messages":[{"role":"user","content":"Alice"},{"role":"user","content":[{"type":"text","text":"Bobby"},{"type":"text","text":"Carla"}]}],"metadata":{}}"#,
    );
    let mut response = response_body(r#"{"choices":[]}"#);

    async_logging_hook(context, &mut request, &mut response, "acompletion")
        .await
        .expect("logging hook");

    let parsed = messages(&request);
    assert_eq!(parsed.len(), 2);
    assert_eq!(
        content_text(parsed[0].content.as_ref()).as_deref(),
        Some("MASKED-Alice")
    );
    let Some(Content::Blocks(items)) = parsed[1].content.as_ref() else {
        panic!("the second message keeps a content list");
    };
    assert_eq!(items.len(), 2);
    let ContentItem::Object(bobby) = &items[0] else {
        panic!("the first part is an object");
    };
    assert_eq!(bobby.text.as_deref(), Some("MASKED-Bobby"));
    let ContentItem::Object(carla) = &items[1] else {
        panic!("the second part is an object");
    };
    assert_eq!(carla.text.as_deref(), Some("MASKED-Carla"));
    assert_eq!(
        logged_entity_types(&request),
        vec![
            "PERSON".to_owned(),
            "EMAIL_ADDRESS".to_owned(),
            "PHONE_NUMBER".to_owned()
        ]
    );
}

#[tokio::test]
async fn keeps_existing_guardrail_records_and_appends_the_new_ones() {
    let analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut request = request_data(
        r#"{"messages":[{"role":"user","content":"Alice"},{"role":"user","content":"Bobby"}],"metadata":{"standard_logging_guardrail_information":[{"guardrail_name":"earlier"}]}}"#,
    );
    let mut response = response_body(r#"{"choices":[]}"#);

    async_logging_hook(context, &mut request, &mut response, "acompletion")
        .await
        .expect("logging hook");

    assert_eq!(logged_count(&request), 3);
    assert_eq!(
        logged_names(&request),
        vec![
            "earlier".to_owned(),
            "presidio-global".to_owned(),
            "presidio-global".to_owned()
        ]
    );
}

#[tokio::test]
async fn records_go_to_the_litellm_metadata_bucket() {
    let analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut request = request_data(
        r#"{"messages":[{"role":"user","content":"Alice"}],"metadata":{"keep":true},"litellm_metadata":{}}"#,
    );
    let mut response = response_body(r#"{"choices":[]}"#);

    async_logging_hook(context, &mut request, &mut response, "acompletion")
        .await
        .expect("logging hook");

    assert_eq!(logged_count(&request), 1);
    assert_eq!(
        request.get("metadata").expect("metadata").get(),
        r#"{"keep":true}"#
    );
}

#[tokio::test]
async fn a_stream_chunk_response_stays_untouched() {
    let analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut request = request_data(r#"{"messages":[{"role":"user","content":"Alice"}]}"#);
    let body = r#"{"object":"chat.completion.chunk","choices":[{"delta":{"content":"Bob"}}]}"#;
    let mut response = response_body(body);

    async_logging_hook(context, &mut request, &mut response, "acompletion")
        .await
        .expect("logging hook");

    assert_eq!(
        content_text(messages(&request)[0].content.as_ref()).as_deref(),
        Some("MASKED")
    );
    assert_eq!(response.into_raw().expect("response JSON").get(), body);
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
