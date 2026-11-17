#![cfg(test)]

use std::fs;

use betterlite_openai::embedding::{
    EmbeddingInput, EmbeddingRecord, EmbeddingResponse, EmbeddingUsage, EmbeddingVector,
};
use betterlite_openai::watsonx_auth::environment::WatsonxEnvironment;
use betterlite_openai::watsonx_auth::types::WatsonxParams;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::Number;

use super::WatsonxEmbeddingConfig;
use super::error::WatsonxEmbeddingError;
use super::reply::decode_watsonx_embedding_reply;
use crate::watsonx_rerank::reply::decode_watsonx_rerank_reply;

fn params(wire: &str) -> WatsonxParams {
    serde_json::from_str(wire).unwrap()
}

fn environment(values: &[(&str, &str)]) -> WatsonxEnvironment {
    let find = |name: &str| {
        values
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| *value)
    };
    let secret = |name: &str| find(name).map(|value| SecretString::from(value.to_owned()));
    let text = |name: &str| find(name).map(str::to_owned);
    WatsonxEnvironment {
        watsonx_iam_url: text("WATSONX_IAM_URL"),
        wx_api_key: secret("WX_API_KEY"),
        watsonx_api_key: secret("WATSONX_API_KEY"),
        watsonx_apikey: secret("WATSONX_APIKEY"),
        watsonx_zenapikey: secret("WATSONX_ZENAPIKEY"),
        watsonx_token: secret("WATSONX_TOKEN"),
        watsonx_api_base: text("WATSONX_API_BASE"),
        watsonx_url: text("WATSONX_URL"),
        wx_url: text("WX_URL"),
        wml_url: text("WML_URL"),
        watsonx_project_id: text("WATSONX_PROJECT_ID"),
        wx_project_id: text("WX_PROJECT_ID"),
        project_id: text("PROJECT_ID"),
        watsonx_region: text("WATSONX_REGION"),
        wx_region: text("WX_REGION"),
        region: text("REGION"),
        watsonx_deployment_space_id: text("WATSONX_DEPLOYMENT_SPACE_ID"),
        watsonx_space_id: text("WATSONX_SPACE_ID"),
        wx_space_id: text("WX_SPACE_ID"),
        space_id: text("SPACE_ID"),
    }
}

fn project_environment() -> WatsonxEnvironment {
    environment(&[("WATSONX_PROJECT_ID", "project")])
}

fn config(environment: &WatsonxEnvironment) -> WatsonxEmbeddingConfig<'_> {
    WatsonxEmbeddingConfig { environment }
}

fn body_map(body: &super::request::WatsonxEmbeddingBody) -> WatsonxParams {
    serde_json::from_str(&serde_json::to_string(body).unwrap()).unwrap()
}

#[rstest]
#[case::trailing_slash(
    "https://host/",
    "{}",
    "https://host/ml/v1/text/embeddings?version=2024-03-13"
)]
#[case::version_used(
    "https://host",
    r#"{"api_version":"2024-01-01"}"#,
    "https://host/ml/v1/text/embeddings?version=2024-01-01"
)]
#[case::empty_version(
    "https://host",
    r#"{"api_version":""}"#,
    "https://host/ml/v1/text/embeddings?version=2024-03-13"
)]
fn url_is_base_plus_embeddings_path(
    #[case] api_base: &str,
    #[case] wire: &str,
    #[case] expected: &str,
) {
    let mut options = params(wire);
    let url = config(&WatsonxEnvironment::default())
        .get_complete_url(Some(api_base), &mut options)
        .unwrap();
    assert_eq!(url, expected);
    assert!(!options.contains_key("api_version"));
}

#[test]
fn url_falls_back_to_environment_base() {
    let environment = environment(&[("WATSONX_API_BASE", "https://env-host/")]);
    let url = config(&environment)
        .get_complete_url(None, &mut WatsonxParams::new())
        .unwrap();
    assert_eq!(
        url,
        "https://env-host/ml/v1/text/embeddings?version=2024-03-13"
    );
}

#[test]
fn url_without_base_is_401() {
    let error = config(&WatsonxEnvironment::default())
        .get_complete_url(None, &mut WatsonxParams::new())
        .unwrap_err();
    assert_eq!(error.status(), Some(401));
}

#[test]
fn url_with_non_text_version_names_the_field() {
    let error = config(&WatsonxEnvironment::default())
        .get_complete_url(Some("https://host"), &mut params(r#"{"api_version":5}"#))
        .unwrap_err();
    assert!(matches!(
        error,
        WatsonxEmbeddingError::InvalidField {
            field: "api_version",
            ..
        }
    ));
    assert_eq!(error.status(), None);
}

#[rstest]
#[case::text(r#""hello""#, r#"["hello"]"#)]
#[case::list(r#"["a","b"]"#, r#"["a","b"]"#)]
#[case::empty_list("[]", "[]")]
fn request_copies_text_input(#[case] wire: &str, #[case] inputs: &str) {
    let input: EmbeddingInput = serde_json::from_str(wire).unwrap();
    let body = config(&project_environment())
        .transform_embedding_request("granite", &input, WatsonxParams::new())
        .unwrap();
    assert_eq!(
        body_map(&body),
        params(&format!(
            r#"{{"inputs":{inputs},"parameters":{{}},"model_id":"granite","project_id":"project"}}"#
        ))
    );
}

#[test]
fn request_keeps_options_and_pops_identity_keys() {
    let body = config(&WatsonxEnvironment::default())
        .transform_embedding_request(
            "granite",
            &EmbeddingInput::Text(String::from("a")),
            params(r#"{"project_id":"p","api_version":"v","temperature":0.2}"#),
        )
        .unwrap();
    assert_eq!(
        body_map(&body),
        params(
            r#"{"inputs":["a"],"parameters":{"api_version":"v","temperature":0.2},"model_id":"granite","project_id":"p"}"#
        )
    );
}

#[test]
fn request_with_space_id_keeps_space_identity() {
    let body = config(&WatsonxEnvironment::default())
        .transform_embedding_request(
            "granite",
            &EmbeddingInput::Text(String::from("a")),
            params(r#"{"space_id":"s"}"#),
        )
        .unwrap();
    assert_eq!(
        body_map(&body),
        params(r#"{"inputs":["a"],"parameters":{},"model_id":"granite","space_id":"s"}"#)
    );
}

#[test]
fn request_for_deployment_model_has_no_identity() {
    let body = config(&WatsonxEnvironment::default())
        .transform_embedding_request(
            "deployment/abc",
            &EmbeddingInput::Text(String::from("a")),
            params(r#"{"space_id":"s"}"#),
        )
        .unwrap();
    assert_eq!(
        body_map(&body),
        params(r#"{"inputs":["a"],"parameters":{}}"#)
    );
}

#[test]
fn request_without_project_or_space_is_401() {
    let error = config(&WatsonxEnvironment::default())
        .transform_embedding_request(
            "granite",
            &EmbeddingInput::Text(String::from("a")),
            WatsonxParams::new(),
        )
        .unwrap_err();
    assert_eq!(error.status(), Some(401));
}

#[rstest]
#[case::token_ids("[1,2]")]
#[case::token_lists("[[1],[2]]")]
fn request_rejects_token_input(#[case] wire: &str) {
    let input: EmbeddingInput = serde_json::from_str(wire).unwrap();
    let error = config(&project_environment())
        .transform_embedding_request("granite", &input, WatsonxParams::new())
        .unwrap_err();
    assert!(matches!(error, WatsonxEmbeddingError::UnsupportedInput));
    assert_eq!(
        error.to_string(),
        "WatsonX embeddings require a string or list of strings"
    );
    assert_eq!(error.status(), None);
}

#[test]
fn request_missing_project_is_reported_before_token_input() {
    let error = config(&WatsonxEnvironment::default())
        .transform_embedding_request(
            "granite",
            &EmbeddingInput::Tokens(vec![1, 2]),
            WatsonxParams::new(),
        )
        .unwrap_err();
    assert_eq!(error.status(), Some(401));
}

#[test]
fn response_maps_results_and_usage() {
    let reply = decode_watsonx_embedding_reply(
        br#"{"results":[{"embedding":[0.1,-2]},{"embedding":[3]}],"input_token_count":7}"#,
    )
    .unwrap();
    let response = WatsonxEmbeddingConfig::transform_embedding_response("granite", reply);
    assert_eq!(
        response,
        EmbeddingResponse {
            object: String::from("list"),
            data: vec![
                EmbeddingRecord {
                    object: String::from("embedding"),
                    embedding: EmbeddingVector::Floats(vec![
                        Number::from_f64(0.1).unwrap(),
                        Number::from(-2_i32),
                    ]),
                    index: 0,
                },
                EmbeddingRecord {
                    object: String::from("embedding"),
                    embedding: EmbeddingVector::Floats(vec![Number::from(3_i32)]),
                    index: 1,
                },
            ],
            model: String::from("granite"),
            usage: EmbeddingUsage {
                prompt_tokens: 7,
                completion_tokens: 0,
                completion_tokens_details: None,
                prompt_tokens_details: None,
                total_tokens: 7,
            },
        }
    );
}

#[test]
fn response_without_usage_defaults_to_zero() {
    let reply = decode_watsonx_embedding_reply(br#"{"results":[]}"#).unwrap();
    let response = WatsonxEmbeddingConfig::transform_embedding_response("m", reply);
    assert_eq!(response.usage.prompt_tokens, 0);
    assert_eq!(response.usage.total_tokens, 0);
    assert_eq!(response.usage.completion_tokens, 0);
    assert!(response.data.is_empty());
    assert_eq!(response.model, "m");
}

#[test]
fn response_null_usage_count_defaults_to_zero() {
    let reply = decode_watsonx_embedding_reply(br#"{"input_token_count":null}"#).unwrap();
    let response = WatsonxEmbeddingConfig::transform_embedding_response("m", reply);
    assert_eq!(response.usage.prompt_tokens, 0);
}

#[rstest]
#[case::missing_embedding(br#"{"results":[{}]}"#)]
#[case::null_results(br#"{"results":null}"#)]
#[case::object_results(br#"{"results":{"embedding":[1]}}"#)]
#[case::string_embedding(br#"{"results":[{"embedding":["a"]}]}"#)]
#[case::nested_embedding(br#"{"results":[{"embedding":[[1]]}]}"#)]
#[case::null_embedding(br#"{"results":[{"embedding":null}]}"#)]
#[case::not_an_object(b"[]")]
#[case::not_json(b"not json")]
#[case::fractional_token_count(br#"{"input_token_count":7.5}"#)]
fn response_rejects_malformed_bodies(#[case] body: &[u8]) {
    let error = decode_watsonx_embedding_reply(body).unwrap_err();
    assert!(matches!(
        error,
        WatsonxEmbeddingError::InvalidResponse { .. }
    ));
    assert_eq!(error.status(), None);
}

#[test]
fn response_error_names_the_body() {
    let error = decode_watsonx_embedding_reply(b"not json").unwrap_err();
    assert!(error.to_string().contains("not json"));
}

#[test]
fn replays_the_watsonx_embed_rerank_reply_corpus() {
    let seen = fs::read_dir("fuzz/corpus/watsonx_embed_rerank_reply")
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file())
        .inspect(|path| {
            let bytes = fs::read(path).unwrap();
            let _embedding = decode_watsonx_embedding_reply(&bytes);
            let _rerank = decode_watsonx_rerank_reply(&bytes);
        })
        .count();
    assert!(seen > 0_usize);
}
