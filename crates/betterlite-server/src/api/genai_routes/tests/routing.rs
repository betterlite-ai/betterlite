use std::sync::Arc;

use axum::http::{HeaderValue, StatusCode};
use betterlite_prompts::arguments::json::Json;
use rstest::rstest;

use crate::api::genai_routes::ports::GenerationBody;

use super::support::{
    FakeGeneration, answer, call, gen_router, get, json_body, parse, post, text, unauthenticated,
};

fn not_found() -> Json {
    parse(r#"{"detail":"Not Found"}"#)
}

fn not_allowed() -> Json {
    parse(r#"{"detail":"Method Not Allowed"}"#)
}

#[tokio::test]
async fn a_model_name_may_carry_slashes() {
    let generation = FakeGeneration::answering(answer(GenerationBody::Json(Json::Null)));
    let (status, _headers, _body) = call(
        gen_router(Arc::clone(&generation)),
        post(
            "/v1beta/models/projects/p/locations/us/models/gemini-2.0-flash:generateContent",
            "{}",
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let calls = generation.calls();
    assert_eq!(
        calls[0].model,
        "projects/p/locations/us/models/gemini-2.0-flash"
    );
    assert_eq!(
        calls[0].data.get("model"),
        Some(&text("projects/p/locations/us/models/gemini-2.0-flash"))
    );
    assert!(!calls[0].data.contains_key("stream"));
}

#[rstest]
#[case::v1beta("/v1beta/models/gemini-2.0-flash:embedContent")]
#[case::unversioned("/models/gemini-2.0-flash:embedContent")]
#[tokio::test]
async fn an_unknown_operation_is_a_404(#[case] path: &str) {
    let generation = FakeGeneration::silent();
    let (status, _headers, body) =
        call(gen_router(Arc::clone(&generation)), post(path, "{}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json_body(&body), not_found());
    assert!(generation.calls().is_empty());
    assert!(generation.streamed().is_empty());
}

#[rstest]
#[case::v1beta("/v1beta/models/gemini-2.0-flash")]
#[case::unversioned("/models/gemini-2.0-flash")]
#[tokio::test]
async fn a_path_without_an_operation_is_a_404(#[case] path: &str) {
    let generation = FakeGeneration::silent();
    let (status, _headers, body) = call(gen_router(generation), post(path, "{}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json_body(&body), not_found());
}

#[rstest]
#[case::v1beta_generate("/v1beta/models/gemini-2.0-flash:generateContent")]
#[case::unversioned_generate("/models/gemini-2.0-flash:generateContent")]
#[case::v1beta_stream("/v1beta/models/gemini-2.0-flash:streamGenerateContent")]
#[case::unversioned_stream("/models/gemini-2.0-flash:streamGenerateContent")]
#[case::v1beta_count("/v1beta/models/gemini-2.0-flash:countTokens")]
#[case::unversioned_count("/models/gemini-2.0-flash:countTokens")]
#[tokio::test]
async fn another_method_on_a_known_operation_is_a_405(#[case] path: &str) {
    let (status, headers, body) = call(gen_router(FakeGeneration::silent()), get(path)).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(
        headers.get("allow"),
        Some(&HeaderValue::from_static("POST"))
    );
    assert_eq!(json_body(&body), not_allowed());
}

#[tokio::test]
async fn another_method_on_an_unknown_operation_is_a_404() {
    let (status, headers, body) = call(
        gen_router(FakeGeneration::silent()),
        get("/v1beta/models/gemini-2.0-flash:embedContent"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(headers.get("allow").is_none());
    assert_eq!(json_body(&body), not_found());
}

#[tokio::test]
async fn a_request_without_the_auth_context_never_reaches_the_pipeline() {
    let generation = FakeGeneration::silent();
    let (status, _headers, _body) = call(
        gen_router(Arc::clone(&generation)),
        unauthenticated("/v1beta/models/gemini-2.0-flash:countTokens", "{}"),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(generation.calls().is_empty());
    assert!(generation.streamed().is_empty());
}

#[tokio::test]
async fn the_last_colon_decides_the_operation() {
    let generation = FakeGeneration::answering(answer(GenerationBody::Json(Json::Null)));
    let (status, _headers, _body) = call(
        gen_router(Arc::clone(&generation)),
        post(
            "/v1beta/models/publishers/google:gemini:generateContent",
            "{}",
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let calls = generation.calls();
    assert_eq!(calls[0].model, "publishers/google:gemini");
}
