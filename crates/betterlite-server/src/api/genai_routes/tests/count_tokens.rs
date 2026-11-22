use std::sync::Arc;

use axum::http::{HeaderValue, StatusCode};
use betterlite_openai::chat::Role as ChatRole;
use betterlite_prompts::arguments::json::Json;
use rstest::rstest;

use crate::api::genai_routes::ports::{
    Modality, PromptTokenDetail, TokenCountOriginal, TokenCountRequest, TokenCountResponse,
};
use crate::api::genai_routes::response::TokenCountDetails;

use super::support::{
    FakeCounter, FakeTranslator, call, count_router, detail, error_body, failure, json_body,
    message, parse, post, with_header,
};

const GEMINI: &str = "gemini-2.0-flash";
const COUNT: &str = "/v1beta/models/gemini-2.0-flash:countTokens";
const BODY: &str = r#"{"contents":[{"parts":[{"text":"hello"}]}]}"#;

fn contents() -> Json {
    parse(r#"[{"parts":[{"text":"hello"}]}]"#)
}

fn details(total_tokens: i64, prompt: Vec<PromptTokenDetail>) -> TokenCountDetails {
    TokenCountDetails {
        total_tokens,
        prompt_tokens_details: prompt,
    }
}

fn counted(total_tokens: i64, original_response: Option<TokenCountOriginal>) -> TokenCountResponse {
    TokenCountResponse {
        total_tokens,
        original_response,
    }
}

#[rstest]
#[case::v1beta("/v1beta/models/gemini-2.0-flash:countTokens")]
#[case::unversioned("/models/gemini-2.0-flash:countTokens")]
#[tokio::test]
async fn both_prefixes_build_the_token_request_from_the_translation(#[case] path: &str) {
    let chat_messages = vec![message(ChatRole::User, "hello")];
    let translator = FakeTranslator::answering(chat_messages.clone());
    let counter = FakeCounter::returning(Some(counted(
        31,
        Some(TokenCountOriginal {
            total_tokens: Some(31),
            prompt_tokens_details: Some(vec![detail(Modality::Text, 31)]),
        }),
    )));
    let (status, headers, body) = call(
        count_router(Arc::clone(&translator), Arc::clone(&counter)),
        post(path, BODY),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get("content-type"),
        Some(&HeaderValue::from_static("application/json"))
    );
    assert_eq!(
        serde_json::from_slice::<TokenCountDetails>(&body).unwrap(),
        details(31, vec![detail(Modality::Text, 31)])
    );
    assert_eq!(translator.calls(), vec![(GEMINI.to_owned(), contents())]);
    assert_eq!(
        counter.calls(),
        vec![(
            TokenCountRequest {
                model: GEMINI.to_owned(),
                contents: contents(),
                messages: chat_messages,
            },
            true,
        )]
    );
}

#[tokio::test]
async fn the_original_response_total_wins_over_the_response_total() {
    let counter = FakeCounter::returning(Some(counted(
        99,
        Some(TokenCountOriginal {
            total_tokens: Some(31),
            prompt_tokens_details: Some(vec![detail(Modality::Text, 31)]),
        }),
    )));
    let translator = FakeTranslator::answering(Vec::new());
    let (status, _headers, body) = call(count_router(translator, counter), post(COUNT, BODY)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        serde_json::from_slice::<TokenCountDetails>(&body).unwrap(),
        details(31, vec![detail(Modality::Text, 31)])
    );
}

#[tokio::test]
async fn an_original_response_without_a_total_counts_zero() {
    let counter = FakeCounter::returning(Some(counted(
        99,
        Some(TokenCountOriginal {
            total_tokens: None,
            prompt_tokens_details: Some(vec![detail(Modality::Text, 5)]),
        }),
    )));
    let translator = FakeTranslator::answering(Vec::new());
    let (status, _headers, body) = call(count_router(translator, counter), post(COUNT, BODY)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        serde_json::from_slice::<TokenCountDetails>(&body).unwrap(),
        details(0, vec![detail(Modality::Text, 5)])
    );
}

#[tokio::test]
async fn an_original_response_without_details_has_an_empty_list() {
    let counter = FakeCounter::returning(Some(counted(
        99,
        Some(TokenCountOriginal {
            total_tokens: Some(31),
            prompt_tokens_details: None,
        }),
    )));
    let translator = FakeTranslator::answering(Vec::new());
    let (status, _headers, body) = call(count_router(translator, counter), post(COUNT, BODY)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        serde_json::from_slice::<TokenCountDetails>(&body).unwrap(),
        details(31, Vec::new())
    );
}

#[tokio::test]
async fn the_response_total_is_used_when_original_response_is_absent() {
    let counter = FakeCounter::returning(Some(counted(17, None)));
    let translator = FakeTranslator::answering(Vec::new());
    let (status, _headers, body) = call(count_router(translator, counter), post(COUNT, BODY)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        serde_json::from_slice::<TokenCountDetails>(&body).unwrap(),
        details(17, Vec::new())
    );
}

#[tokio::test]
async fn zero_is_returned_when_the_counter_answers_none() {
    let counter = FakeCounter::returning(None);
    let translator = FakeTranslator::answering(Vec::new());
    let (status, _headers, body) = call(count_router(translator, counter), post(COUNT, BODY)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        serde_json::from_slice::<TokenCountDetails>(&body).unwrap(),
        details(0, Vec::new())
    );
}

#[rstest]
#[case::missing("{}")]
#[case::null(r#"{"contents":null}"#)]
#[tokio::test]
async fn contents_of_no_value_take_the_empty_list(#[case] body: &str) {
    let translator = FakeTranslator::answering(Vec::new());
    let counter = FakeCounter::returning(None);
    let (status, _headers, _body) = call(
        count_router(Arc::clone(&translator), Arc::clone(&counter)),
        post(COUNT, body),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        translator.calls(),
        vec![(GEMINI.to_owned(), Json::List(Vec::new()))]
    );
    let requests = counter.calls();
    assert_eq!(requests[0].0.contents, Json::List(Vec::new()));
    assert!(requests[0].0.messages.is_empty());
}

#[tokio::test]
async fn contents_that_are_not_a_list_are_refused() {
    let translator = FakeTranslator::silent();
    let counter = FakeCounter::silent();
    let (status, _headers, body) = call(
        count_router(Arc::clone(&translator), Arc::clone(&counter)),
        post(COUNT, r#"{"contents":{"parts":[]}}"#),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        json_body(&body),
        error_body(
            "'contents' has the wrong type",
            "invalid_request_error",
            Some("contents"),
            "400"
        )
    );
    assert!(translator.calls().is_empty());
    assert!(counter.calls().is_empty());
}

#[tokio::test]
async fn a_translation_error_is_rendered_as_the_pipeline_reports_it() {
    let translator =
        FakeTranslator::failing(failure("400", "invalid_request_error", Some("contents")));
    let counter = FakeCounter::silent();
    let (status, _headers, body) = call(
        count_router(translator, Arc::clone(&counter)),
        post(COUNT, BODY),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        json_body(&body),
        error_body(
            "the provider refused",
            "invalid_request_error",
            Some("contents"),
            "400"
        )
    );
    assert!(counter.calls().is_empty());
}

#[tokio::test]
async fn a_token_counter_error_keeps_its_status_type_param_and_code_and_headers() {
    let exception = with_header(
        failure("503", "token_counting_error", Some("model")),
        "x-litellm-call-id",
        "call-1",
    );
    let counter = FakeCounter::failing(exception);
    let translator = FakeTranslator::answering(Vec::new());
    let (status, headers, body) = call(count_router(translator, counter), post(COUNT, BODY)).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        headers.get("x-litellm-call-id"),
        Some(&HeaderValue::from_static("call-1"))
    );
    assert_eq!(
        json_body(&body),
        error_body(
            "the provider refused",
            "token_counting_error",
            Some("model"),
            "503"
        )
    );
}

#[tokio::test]
async fn the_text_of_a_count_without_a_total_is_still_a_number() {
    let counter = FakeCounter::returning(None);
    let translator = FakeTranslator::answering(Vec::new());
    let (status, _headers, body) = call(
        count_router(translator, counter),
        post(COUNT, r#"{"contents":[]}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        json_body(&body),
        parse(r#"{"totalTokens":0,"promptTokensDetails":[]}"#)
    );
}

#[tokio::test]
async fn the_path_model_is_used_even_when_the_body_sends_one() {
    let translator = FakeTranslator::answering(Vec::new());
    let counter = FakeCounter::returning(None);
    let (status, _headers, _body) = call(
        count_router(Arc::clone(&translator), Arc::clone(&counter)),
        post(COUNT, r#"{"model":"body-model","contents":[]}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        translator.calls(),
        vec![(GEMINI.to_owned(), Json::List(Vec::new()))]
    );
    let requests = counter.calls();
    assert_eq!(requests[0].0.model, GEMINI);
}

#[test]
fn the_original_response_reads_the_provider_camel_case_fields() {
    let original: TokenCountOriginal = serde_json::from_str(
        r#"{"totalTokens":31,"promptTokensDetails":[{"modality":"TEXT","tokenCount":31}]}"#,
    )
    .unwrap();
    assert_eq!(
        original,
        TokenCountOriginal {
            total_tokens: Some(31),
            prompt_tokens_details: Some(vec![detail(Modality::Text, 31)]),
        }
    );
}
