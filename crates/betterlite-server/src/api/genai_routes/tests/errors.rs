use std::sync::Arc;

use axum::http::{HeaderValue, StatusCode};
use betterlite_prompts::arguments::json::Json;
use rstest::rstest;

use crate::api::genai_routes::ports::GenerationBody;

use super::support::{
    FakeGeneration, answer, call, error_body, failure, gen_router,
    gen_router_with_call_id_in_errors, json_body, object, parse, post, post_bytes, text,
    with_header,
};

const GEMINI: &str = "gemini-2.0-flash";
const GENERATE: &str = "/v1beta/models/gemini-2.0-flash:generateContent";
const STREAM: &str = "/v1beta/models/gemini-2.0-flash:streamGenerateContent";

#[rstest]
#[case::v1beta("/v1beta/models/gemini-2.0-flash:generateContent")]
#[case::unversioned("/models/gemini-2.0-flash:generateContent")]
#[tokio::test]
async fn a_body_that_is_not_json_is_refused(#[case] path: &str) {
    let generation = FakeGeneration::silent();
    let (status, _headers, body) =
        call(gen_router(Arc::clone(&generation)), post(path, "{not json")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        json_body(&body),
        error_body(
            "the request body is not valid JSON",
            "invalid_request_error",
            Some("request_body"),
            "400"
        )
    );
    assert!(generation.calls().is_empty());
}

#[tokio::test]
async fn a_body_that_is_not_an_object_is_refused() {
    let generation = FakeGeneration::silent();
    let (status, _headers, body) =
        call(gen_router(Arc::clone(&generation)), post(GENERATE, "[1,2]")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        json_body(&body),
        error_body(
            "the request body must be a JSON object",
            "invalid_request_error",
            Some("request_body"),
            "400"
        )
    );
    assert!(generation.calls().is_empty());
}

#[tokio::test]
async fn a_body_that_is_not_utf8_is_an_empty_object() {
    let generation = FakeGeneration::answering(answer(GenerationBody::Json(Json::Null)));
    let (status, _headers, _body) = call(
        gen_router(Arc::clone(&generation)),
        post_bytes(GENERATE, vec![0xff_u8, 0xfe, 0xfd]),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let calls = generation.calls();
    assert_eq!(calls[0].model, GEMINI);
    assert_eq!(calls[0].data, object(vec![("model", text(GEMINI))]));
}

#[tokio::test]
async fn a_body_over_the_json_limit_is_refused_with_the_boundary_facts() {
    let generation = FakeGeneration::silent();
    let mut body = vec![b' '; 16 * 1024 * 1024];
    body.push(b' ');
    let (status, _headers, response_body) = call(
        gen_router(Arc::clone(&generation)),
        post_bytes(GENERATE, body),
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        json_body(&response_body),
        error_body(
            "the request JSON body is 16777217 bytes; the maximum is 16777216",
            "invalid_request_error",
            Some("body"),
            "request_too_large"
        )
    );
    assert!(generation.calls().is_empty());
}

#[tokio::test]
async fn a_pipeline_error_keeps_its_status_type_param_and_code_and_headers() {
    let exception = with_header(
        failure("429", "rate_limit_error", Some("model")),
        "x-litellm-call-id",
        "call-1",
    );
    let generation = FakeGeneration::failing(exception);
    let (status, headers, body) = call(gen_router(generation), post(GENERATE, "{}")).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        headers.get("x-litellm-call-id"),
        Some(&HeaderValue::from_static("call-1"))
    );
    assert_eq!(
        json_body(&body),
        error_body(
            "the provider refused",
            "rate_limit_error",
            Some("model"),
            "429"
        )
    );
}

#[tokio::test]
async fn a_pipeline_error_carries_the_call_id_when_the_setting_is_on() {
    let exception = with_header(
        failure("429", "rate_limit_error", Some("model")),
        "x-litellm-call-id",
        "call-1",
    );
    let generation = FakeGeneration::failing(exception);
    let (status, _headers, body) = call(
        gen_router_with_call_id_in_errors(generation),
        post(GENERATE, "{}"),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        json_body(&body),
        parse(
            r#"{"error":{"message":"the provider refused","type":"rate_limit_error","param":"model","code":"429","litellm_call_id":"call-1"}}"#
        )
    );
}

#[tokio::test]
async fn a_stream_pipeline_error_is_rendered_as_the_pipeline_reports_it() {
    let generation =
        FakeGeneration::stream_failing(failure("424", "provider_error", Some("deployment")));
    let (status, _headers, body) = call(gen_router(generation), post(STREAM, "{}")).await;
    assert_eq!(status, StatusCode::FAILED_DEPENDENCY);
    assert_eq!(
        json_body(&body),
        error_body(
            "the provider refused",
            "provider_error",
            Some("deployment"),
            "424"
        )
    );
}

#[rstest]
#[case::not_a_number("not-a-status")]
#[case::not_an_http_status("1000")]
#[tokio::test]
async fn a_code_that_is_not_a_status_answers_the_generic_500(#[case] code: &str) {
    let generation = FakeGeneration::failing(failure(code, "provider_error", None));
    let (status, _headers, body) = call(gen_router(generation), post(GENERATE, "{}")).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        json_body(&body),
        error_body("the provider refused", "provider_error", None, code)
    );
}

#[tokio::test]
async fn headers_that_are_not_valid_are_dropped() {
    let exception = with_header(
        with_header(
            with_header(
                failure("429", "rate_limit_error", None),
                "bad header name",
                "value",
            ),
            "x-good",
            "kept",
        ),
        "x-bad-value",
        "line\nbreak",
    );
    let generation = FakeGeneration::failing(exception);
    let (status, headers, _body) = call(gen_router(generation), post(GENERATE, "{}")).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        headers.get("x-good"),
        Some(&HeaderValue::from_static("kept"))
    );
    assert!(headers.get("x-bad-value").is_none());
    assert!(headers.get("bad header name").is_none());
}

#[tokio::test]
async fn a_generation_answer_of_json_null_is_served_as_null() {
    let generation = FakeGeneration::answering(answer(GenerationBody::Json(Json::Null)));
    let (status, _headers, body) = call(gen_router(generation), post(GENERATE, "{}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_body(&body), Json::Null);
}
