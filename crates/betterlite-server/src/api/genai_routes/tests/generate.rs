use std::sync::Arc;

use axum::http::{HeaderMap, HeaderValue, StatusCode};
use betterlite_prompts::arguments::json::Json;
use rstest::rstest;

use crate::api::genai_routes::ports::{GenerationAnswer, GenerationBody};

use super::support::{FakeGeneration, answer, call, gen_router, json_body, object, post, text};

const GEMINI: &str = "gemini-2.0-flash";
const GENERATE: &str = "/v1beta/models/gemini-2.0-flash:generateContent";

fn generated() -> Json {
    Json::Object(object(vec![
        ("candidates", Json::List(Vec::new())),
        ("text", text("hello")),
    ]))
}

#[rstest]
#[case::v1beta("/v1beta/models/gemini-2.0-flash:generateContent")]
#[case::unversioned("/models/gemini-2.0-flash:generateContent")]
#[tokio::test]
async fn both_prefixes_serve_generate_content_with_the_path_model(#[case] path: &str) {
    let generation = FakeGeneration::answering(answer(GenerationBody::Json(generated())));
    let mut request = post(path, r#"{"contents":[]}"#);
    request
        .headers_mut()
        .insert("x-request-id", HeaderValue::from_static("request-1"));
    let (status, _headers, body) = call(gen_router(Arc::clone(&generation)), request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_body(&body), generated());
    let calls = generation.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, GEMINI);
    assert_eq!(calls[0].data.get("model"), Some(&text(GEMINI)));
    assert_eq!(
        calls[0].headers.get("x-request-id"),
        Some(&HeaderValue::from_static("request-1"))
    );
    assert_eq!(calls[0].subject, "admin-user");
    assert!(!calls[0].data.contains_key("stream"));
    assert!(!calls[0].data.contains_key("_litellm_raw_sse_stream"));
    assert!(generation.streamed().is_empty());
}

#[tokio::test]
async fn a_model_in_the_body_wins_over_the_path_model() {
    let generation = FakeGeneration::answering(answer(GenerationBody::Json(Json::Null)));
    let (status, _headers, _body) = call(
        gen_router(Arc::clone(&generation)),
        post(GENERATE, r#"{"model":"body-model"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let calls = generation.calls();
    assert_eq!(calls[0].model, GEMINI);
    assert_eq!(calls[0].data.get("model"), Some(&text("body-model")));
}

#[tokio::test]
async fn an_explicit_null_model_is_preserved() {
    let generation = FakeGeneration::answering(answer(GenerationBody::Json(Json::Null)));
    let (status, _headers, _body) = call(
        gen_router(Arc::clone(&generation)),
        post(GENERATE, r#"{"model":null}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let calls = generation.calls();
    assert_eq!(calls[0].model, GEMINI);
    assert_eq!(calls[0].data.get("model"), Some(&Json::Null));
}

#[tokio::test]
async fn an_empty_body_is_an_empty_object_with_the_path_model() {
    let generation = FakeGeneration::answering(answer(GenerationBody::Json(Json::Null)));
    let (status, _headers, _body) =
        call(gen_router(Arc::clone(&generation)), post(GENERATE, "")).await;
    assert_eq!(status, StatusCode::OK);
    let calls = generation.calls();
    assert_eq!(calls[0].data.get("model"), Some(&text(GEMINI)));
    assert_eq!(calls[0].data.len(), 1);
}

#[tokio::test]
async fn the_pipeline_headers_are_copied_onto_the_answer() {
    let mut headers = HeaderMap::new();
    headers.insert("x-litellm-call-id", HeaderValue::from_static("call-1"));
    headers.insert("x-litellm-model-id", HeaderValue::from_static("model-1"));
    let generation = FakeGeneration::answering(GenerationAnswer {
        headers,
        body: GenerationBody::Json(Json::Null),
    });
    let (status, response_headers, _body) =
        call(gen_router(generation), post(GENERATE, "{}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        response_headers.get("x-litellm-call-id"),
        Some(&HeaderValue::from_static("call-1"))
    );
    assert_eq!(
        response_headers.get("x-litellm-model-id"),
        Some(&HeaderValue::from_static("model-1"))
    );
    assert_eq!(
        response_headers.get("content-type"),
        Some(&HeaderValue::from_static("application/json"))
    );
}
