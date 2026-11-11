use betterlite_openai::embedding::EmbeddingVector;
use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use super::support::{FailingEncoder, WordsEncoder, options};
use crate::ollama_generate::embed::{
    embed_url, transform_embedding_request, transform_embedding_response,
};

fn prompts(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| String::from(*value)).collect()
}

#[test]
fn a_plain_request_carries_only_the_model_and_input() {
    let found = transform_embedding_request("m", &prompts(&["a"]), &IndexMap::new()).unwrap();
    assert_eq!(found.model, "m");
    assert_eq!(found.input, vec![String::from("a")]);
    assert_eq!(found.truncate, None);
    assert_eq!(found.options, None);
    assert_eq!(found.keep_alive, None);
    assert_eq!(found.dimensions, None);
}

#[test]
fn the_four_body_level_options_stay_outside_options() {
    let found = transform_embedding_request(
        "m",
        &prompts(&["a"]),
        &options(r#"{"truncate":true,"keep_alive":"5m","dimensions":3,"options":{"num_ctx":2}}"#),
    )
    .unwrap();
    assert_eq!(found.truncate, Some(Json::Bool(true)));
    assert_eq!(found.keep_alive, Some(Json::Text(String::from("5m"))));
    assert_eq!(found.dimensions, Some(Json::Number(Number::from(3_i32))));
    let mut expected = IndexMap::new();
    expected.insert(String::from("num_ctx"), Json::Number(Number::from(2_i32)));
    assert_eq!(found.options, Some(expected));
}

#[test]
fn every_other_option_merges_into_options() {
    let found = transform_embedding_request(
        "m",
        &prompts(&["a"]),
        &options(r#"{"options":{"num_ctx":2},"temperature":0.5}"#),
    )
    .unwrap();
    let mut expected = IndexMap::new();
    expected.insert(String::from("num_ctx"), Json::Number(Number::from(2_i32)));
    expected.insert(
        String::from("temperature"),
        Json::Number(Number::from_f64(0.5).unwrap()),
    );
    assert_eq!(found.options, Some(expected));
}

#[test]
fn an_options_object_after_other_keys_replaces_them() {
    let found = transform_embedding_request(
        "m",
        &prompts(&["a"]),
        &options(r#"{"temperature":0.5,"options":{"num_ctx":2}}"#),
    )
    .unwrap();
    let mut expected = IndexMap::new();
    expected.insert(String::from("num_ctx"), Json::Number(Number::from(2_i32)));
    assert_eq!(found.options, Some(expected));
}

#[test]
fn a_null_options_key_is_ignored_for_later_options() {
    let found = transform_embedding_request(
        "m",
        &prompts(&["a"]),
        &options(r#"{"options":null,"temperature":0.5}"#),
    )
    .unwrap();
    assert_eq!(
        found
            .options
            .as_ref()
            .and_then(|options| options.get("temperature")),
        Some(&Json::Number(Number::from_f64(0.5).unwrap()))
    );
}

#[test]
fn a_null_special_option_is_kept() {
    let found =
        transform_embedding_request("m", &prompts(&["a"]), &options(r#"{"truncate":null}"#))
            .unwrap();
    assert_eq!(found.truncate, Some(Json::Null));
}

#[test]
fn a_non_object_options_value_is_rejected() {
    let error =
        transform_embedding_request("m", &prompts(&["a"]), &options(r#"{"options":"bad"}"#))
            .unwrap_err();
    assert_eq!(error.status(), Some(400));
}

#[rstest]
#[case::plain("http://h", "http://h/api/embed")]
#[case::already_complete("http://h/api/embed", "http://h/api/embed")]
#[case::trailing_slash("http://h/api/embed/", "http://h/api/embed//api/embed")]
#[case::empty("", "/api/embed")]
fn embed_url_appends_the_embed_path(#[case] api_base: &str, #[case] expected: &str) {
    assert_eq!(embed_url(api_base), expected);
}

#[tokio::test]
async fn a_reply_carries_the_records_and_the_prompt_count() {
    let found = transform_embedding_response(
        200,
        br#"{"embeddings":[[0.5,1],[0.25]],"prompt_eval_count":4}"#,
        "m",
        &prompts(&["a"]),
        None,
    )
    .await
    .unwrap();
    assert_eq!(found.object, "list");
    assert_eq!(found.model, "ollama/m");
    assert_eq!(found.data.len(), 2_usize);
    assert_eq!(found.data[0].object, "embedding");
    assert_eq!(found.data[0].index, 0_u32);
    assert_eq!(found.data[1].index, 1_u32);
    assert_eq!(
        found.data[0].embedding,
        EmbeddingVector::Floats(vec![Number::from_f64(0.5).unwrap(), Number::from(1_u64),])
    );
    assert_eq!(found.usage.prompt_tokens, 4_u32);
    assert_eq!(found.usage.completion_tokens, 0_u32);
    assert_eq!(found.usage.total_tokens, 4_u32);
    assert_eq!(found.usage.prompt_tokens_details, None);
    assert_eq!(found.usage.completion_tokens_details, None);
}

#[tokio::test]
async fn a_missing_prompt_count_uses_the_encoder_on_the_joined_prompts() {
    let found = transform_embedding_response(
        200,
        br#"{"embeddings":[[0.5]]}"#,
        "m",
        &prompts(&["a b", "c"]),
        Some(&WordsEncoder),
    )
    .await
    .unwrap();
    assert_eq!(found.usage.prompt_tokens, 2_u32);
    assert_eq!(found.usage.total_tokens, 2_u32);
}

#[tokio::test]
async fn a_null_prompt_count_uses_the_encoder() {
    let found = transform_embedding_response(
        200,
        br#"{"embeddings":[],"prompt_eval_count":null}"#,
        "m",
        &prompts(&["a b"]),
        Some(&WordsEncoder),
    )
    .await
    .unwrap();
    assert_eq!(found.usage.prompt_tokens, 2_u32);
    assert!(found.data.is_empty());
}

#[tokio::test]
async fn a_missing_prompt_count_without_an_encoder_counts_zero() {
    let found = transform_embedding_response(
        200,
        br#"{"embeddings":[[0.5]]}"#,
        "m",
        &prompts(&["a b"]),
        None,
    )
    .await
    .unwrap();
    assert_eq!(found.usage.prompt_tokens, 0_u32);
}

#[tokio::test]
async fn an_encoder_error_propagates() {
    let error = transform_embedding_response(
        200,
        br#"{"embeddings":[[0.5]]}"#,
        "m",
        &prompts(&["a"]),
        Some(&FailingEncoder),
    )
    .await
    .unwrap_err();
    assert_eq!(error.status(), None);
}

#[rstest]
#[case::not_json("not json")]
#[case::missing_embeddings(r#"{"prompt_eval_count":3}"#)]
#[case::wrong_embedding_text(r#"{"embeddings":"bad"}"#)]
#[case::non_numeric_vector(r#"{"embeddings":[["bad"]]}"#)]
#[case::wrong_count(r#"{"embeddings":[[0.5]],"prompt_eval_count":"4"}"#)]
#[tokio::test]
async fn a_malformed_reply_is_a_shape_error(#[case] body: &str) {
    let error = transform_embedding_response(200, body.as_bytes(), "m", &prompts(&["a"]), None)
        .await
        .unwrap_err();
    assert_eq!(error.status(), None);
}

#[tokio::test]
async fn an_error_status_keeps_the_status_and_the_body() {
    let error =
        transform_embedding_response(418, br#"{"error":"teapot"}"#, "m", &prompts(&["a"]), None)
            .await
            .unwrap_err();
    assert_eq!(error.status(), Some(418));
    assert!(error.to_string().contains("teapot"));
}

#[tokio::test]
async fn a_non_utf8_error_body_is_described_by_its_length() {
    let error = transform_embedding_response(500, &[0xFF, 0xFE], "m", &prompts(&["a"]), None)
        .await
        .unwrap_err();
    assert_eq!(error.status(), Some(500));
    assert!(error.to_string().contains("2 bytes that are not UTF-8"));
}
