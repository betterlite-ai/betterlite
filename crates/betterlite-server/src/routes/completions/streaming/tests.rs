#![cfg(test)]

use std::time::{Duration, Instant};

use axum::body::to_bytes;
use betterlite_config::types::{LiteLlmParams, ModelDeployment, ModelInfo};
use betterlite_core::message_utilities::response_streams::StreamError;
use betterlite_core::operation::text_stream::TextStreamResult;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::deployment::{ChatDeployment, OpenAiDefaults};
use betterlite_openai::text_completion_stream::{DONE_FRAME, without_nulls};
use futures_util::StreamExt as _;
use http::HeaderMap;
use rstest::rstest;
use secrecy::SecretString;

use super::render;

fn deployment() -> ChatDeployment {
    let listed = ModelDeployment {
        model_name: String::from("text-model"),
        litellm_params: LiteLlmParams {
            ssl_verify: None,
            model: Some(String::from("openai/gpt-3.5-turbo-instruct")),
            api_base: Some(String::from("http://127.0.0.1:9/v1")),
            api_key: Some(SecretString::from("configured-key")),
            api_version: None,
            timeout: None,
            stream_timeout: None,
            max_retries: None,
            rpm: None,
            tpm: None,
            weight: None,
        },
        model_info: ModelInfo::default(),
    };
    ChatDeployment::resolve(&listed, &OpenAiDefaults::default()).expect("deployment")
}

fn chunk(text: &str) -> Json {
    serde_json::from_str(text).expect("chunk")
}

fn result(items: Vec<Result<Json, StreamError>>) -> TextStreamResult {
    TextStreamResult {
        body: futures_util::stream::iter(items).boxed(),
        upstream_headers: HeaderMap::new(),
        upstream_time: Duration::from_millis(2),
    }
}

#[derive(Clone, Copy, Debug)]
enum Scenario {
    NullsDropped,
    DoneLast,
    ErrorTruncates,
    StreamHeaders,
}

#[rstest]
#[case::nulls_dropped(Scenario::NullsDropped)]
#[case::done_last(Scenario::DoneLast)]
#[case::error_truncates(Scenario::ErrorTruncates)]
#[case::stream_headers(Scenario::StreamHeaders)]
#[tokio::test]
async fn text_streaming_text_stream_tests_behavior(#[case] scenario: Scenario) {
    match scenario {
        Scenario::NullsDropped => {
            let answer = result(vec![Ok(chunk(
                r#"{"id":"cmpl-1","object":"text_completion","created":1,"model":"text-model","choices":[{"finish_reason":null,"index":0,"text":" Hel","logprobs":null}],"usage":{"completion_tokens":0,"prompt_tokens":0,"total_tokens":0}}"#,
            ))]);
            let response = render(answer, &deployment(), "call", Instant::now());
            let body = to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("body");
            let text = String::from_utf8(body.to_vec()).expect("utf8");
            assert!(text.starts_with("data: "), "{text}");
            assert!(text.ends_with("\n\n"), "{text}");
            let event: Json =
                serde_json::from_str(text.trim_start_matches("data: ").trim_end()).expect("event");
            assert_eq!(
                event,
                chunk(
                    r#"{"id":"cmpl-1","object":"text_completion","created":1,"model":"text-model","choices":[{"index":0,"text":" Hel"}],"usage":{"completion_tokens":0,"prompt_tokens":0,"total_tokens":0}}"#
                )
            );
        }
        Scenario::DoneLast => {
            let response = render(result(Vec::new()), &deployment(), "call", Instant::now());
            let body = to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("body");
            // the whole body of an empty stream is the closing frame the route takes from DONE_FRAME
            assert_eq!(String::from_utf8(body.to_vec()).expect("utf8"), DONE_FRAME);
        }
        Scenario::ErrorTruncates => {
            let answer = result(vec![Err(StreamError::MissingChunk)]);
            let response = render(answer, &deployment(), "call", Instant::now());
            assert!(to_bytes(response.into_body(), usize::MAX).await.is_err());
        }
        Scenario::StreamHeaders => {
            let response = render(result(Vec::new()), &deployment(), "call", Instant::now());
            let headers = response.headers();
            assert_eq!(
                headers
                    .get("content-type")
                    .and_then(|value| value.to_str().ok()),
                Some("text/event-stream; charset=utf-8")
            );
            assert_eq!(
                headers
                    .get("cache-control")
                    .and_then(|value| value.to_str().ok()),
                Some("no-cache")
            );
            assert_eq!(
                headers
                    .get("x-accel-buffering")
                    .and_then(|value| value.to_str().ok()),
                Some("no")
            );
        }
    }
}

#[test]
fn the_null_drop_keeps_a_null_inside_a_list() {
    let cleaned = without_nulls(
        serde_json::to_value(chunk(
            r#"{"a":null,"b":[null,{"c":null,"d":1}],"e":"keep"}"#,
        ))
        .expect("value"),
    );
    let expected =
        serde_json::to_value(chunk(r#"{"b":[null,{"d":1}],"e":"keep"}"#)).expect("value");
    assert_eq!(cleaned, expected);
}
