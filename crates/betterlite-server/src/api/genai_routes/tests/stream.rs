use std::sync::Arc;

use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use betterlite_prompts::arguments::json::Json;
use rstest::rstest;

use crate::api::genai_routes::ports::{GenerationAnswer, GenerationBody};

use super::support::{FakeGeneration, call, events, gen_router, post, text};

const GEMINI: &str = "gemini-2.0-flash";
const STREAM: &str = "/v1beta/models/gemini-2.0-flash:streamGenerateContent";

fn chunk() -> Vec<u8> {
    b"data: {\"text\":\"hi\"}\n\n".to_vec()
}

fn streaming_generation() -> Arc<FakeGeneration> {
    let mut headers = HeaderMap::new();
    headers.insert("x-litellm-call-id", HeaderValue::from_static("call-1"));
    FakeGeneration::streaming(GenerationAnswer {
        headers,
        body: GenerationBody::Events(events(vec![chunk()])),
    })
}

#[rstest]
#[case::v1beta("/v1beta/models/gemini-2.0-flash:streamGenerateContent")]
#[case::unversioned("/models/gemini-2.0-flash:streamGenerateContent")]
#[tokio::test]
async fn both_prefixes_serve_stream_generate_content_with_raw_sse(#[case] path: &str) {
    let generation = streaming_generation();
    let (status, headers, body) = call(
        gen_router(Arc::clone(&generation)),
        post(path, r#"{"contents":[]}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.to_vec(), chunk());
    assert_eq!(
        headers.get("content-type"),
        Some(&HeaderValue::from_static(
            "text/event-stream; charset=utf-8"
        ))
    );
    assert_eq!(
        headers.get("cache-control"),
        Some(&HeaderValue::from_static("no-cache"))
    );
    assert_eq!(
        headers.get("x-accel-buffering"),
        Some(&HeaderValue::from_static("no"))
    );
    assert_eq!(
        headers.get("x-litellm-call-id"),
        Some(&HeaderValue::from_static("call-1"))
    );
}

#[tokio::test]
async fn streaming_sets_the_stream_field_and_both_raw_sse_flags() {
    let generation = streaming_generation();
    let (status, _headers, _body) = call(
        gen_router(Arc::clone(&generation)),
        post(STREAM, r#"{"contents":[]}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let calls = generation.streamed();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, GEMINI);
    assert_eq!(calls[0].data.get("model"), Some(&text(GEMINI)));
    assert_eq!(calls[0].data.get("stream"), Some(&Json::Bool(true)));
    assert_eq!(
        calls[0].data.get("_litellm_skip_openai_stream_done"),
        Some(&Json::Bool(true))
    );
    assert_eq!(
        calls[0].data.get("_litellm_raw_sse_stream"),
        Some(&Json::Bool(true))
    );
    assert!(generation.calls().is_empty());
}

#[tokio::test]
async fn streaming_assigns_the_flags_over_caller_values() {
    let generation = streaming_generation();
    let (status, _headers, _body) = call(
        gen_router(Arc::clone(&generation)),
        post(
            STREAM,
            r#"{"stream":false,"_litellm_skip_openai_stream_done":false,"_litellm_raw_sse_stream":"no"}"#,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let calls = generation.streamed();
    assert_eq!(calls[0].data.get("stream"), Some(&Json::Bool(true)));
    assert_eq!(
        calls[0].data.get("_litellm_skip_openai_stream_done"),
        Some(&Json::Bool(true))
    );
    assert_eq!(
        calls[0].data.get("_litellm_raw_sse_stream"),
        Some(&Json::Bool(true))
    );
}

#[tokio::test]
async fn a_content_type_from_the_pipeline_is_kept_on_the_stream() {
    let mut headers = HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/x-ndjson"),
    );
    let generation = FakeGeneration::streaming(GenerationAnswer {
        headers,
        body: GenerationBody::Events(events(vec![chunk()])),
    });
    let (status, response_headers, _body) = call(gen_router(generation), post(STREAM, "{}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        response_headers.get("content-type"),
        Some(&HeaderValue::from_static("application/x-ndjson"))
    );
    assert_eq!(
        response_headers.get("cache-control"),
        Some(&HeaderValue::from_static("no-cache"))
    );
}
