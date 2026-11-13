#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::{RerankDocument, RerankResponse};
use betterlite_openai::dashscope_embedding::identity::FamilyIdentity;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::Number;
use uuid::Uuid;

use super::decode::decode_dashscope_rerank_reply;
use super::{DashscopeRerankConfig, DashscopeRerankError};
use crate::api::rerank_contracts::{
    BaseRerankConfig, EnvironmentRequest, Headers, MapCohereRerankParams, Params, RerankLogging,
    RerankProviderResponse, ResponseTransform,
};
use crate::dashscope_common::DASHSCOPE_RERANK_URL;

fn identity() -> FamilyIdentity {
    FamilyIdentity {
        missing_key_message: "DashScope API key is required. Set 'DASHSCOPE_API_KEY' env var or pass api_key explicitly.".to_owned(),
        default_base: "https://dashscope.aliyuncs.com/compatible-mode/v1".to_owned(),
        env_base: None,
        env_key: None,
        fallback_key: None,
        empty_key_is_set: true,
    }
}

fn config(env_value: &str) -> DashscopeRerankConfig {
    DashscopeRerankConfig {
        identity: identity(),
        rerank_env_value: env_value.to_owned(),
        default_rerank_url: DASHSCOPE_RERANK_URL.to_owned(),
    }
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn documents() -> Vec<RerankDocument> {
    vec![
        RerankDocument::Text("first".to_owned()),
        RerankDocument::Text("second".to_owned()),
    ]
}

fn params(entries: &[(&str, Json)]) -> Params {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

#[test]
fn returns_the_default_rerank_url_untouched() {
    assert_eq!(
        config("").get_complete_url(None, "qwen3-rerank", None),
        DASHSCOPE_RERANK_URL
    );
}

#[rstest]
#[case::a_versioned_base("https://my.rerank.test/v1", "https://my.rerank.test/v1/reranks")]
#[case::a_bare_host("https://my.rerank.test", "https://my.rerank.test/reranks")]
#[case::a_reranks_base(
    "https://my.rerank.test/custom/reranks",
    "https://my.rerank.test/custom/reranks"
)]
#[case::a_rerank_base(
    "https://my.rerank.test/custom/rerank",
    "https://my.rerank.test/custom/rerank"
)]
#[case::trailing_slashes("https://my.rerank.test/v1///", "https://my.rerank.test/v1/reranks")]
fn completes_a_custom_base_with_the_rerank_route(#[case] base: &str, #[case] expected: &str) {
    assert_eq!(
        config("").get_complete_url(Some(base), "qwen3-rerank", None),
        expected
    );
}

#[test]
fn the_environment_value_sets_the_url() {
    assert_eq!(
        config("https://env.rerank.test/v1").get_complete_url(
            Some("https://dashscope.aliyuncs.com/compatible-mode/v1"),
            "qwen3-rerank",
            None
        ),
        "https://env.rerank.test/v1/reranks"
    );
}

#[test]
fn headers_carry_the_bearer_key_accept_and_content_type() {
    let key = SecretString::from("sk-rerank");
    let mut caller = Headers::new();
    caller.insert("x-custom".to_owned(), "value".to_owned());
    let request = EnvironmentRequest {
        headers: &caller,
        model: "qwen3-rerank",
        api_key: Some(&key),
        optional_params: None,
        litellm_params: None,
    };
    let headers = config("").validate_environment(request).unwrap();
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-rerank")
    );
    assert_eq!(
        headers.get("accept").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(headers.get("x-custom").map(String::as_str), Some("value"));
}

#[test]
fn headers_fail_with_the_missing_key_message() {
    let caller = Headers::new();
    let request = EnvironmentRequest {
        headers: &caller,
        model: "qwen3-rerank",
        api_key: None,
        optional_params: None,
        litellm_params: None,
    };
    let error = config("").validate_environment(request).unwrap_err();
    assert_eq!(
        error.to_string(),
        "DashScope API key is required. Set 'DASHSCOPE_API_KEY' env var or pass api_key explicitly."
    );
}

#[test]
fn supports_the_four_cohere_params() {
    assert_eq!(
        config("").supported_rerank_params("qwen3-rerank"),
        vec!["query", "documents", "top_n", "return_documents"]
    );
}

#[test]
fn maps_the_cohere_params_it_supports() {
    let documents = documents();
    let mapped = config("")
        .map_cohere_rerank_params(MapCohereRerankParams {
            non_default_params: &mut Params::new(),
            model: "qwen3-rerank",
            drop_params: false,
            query: "which one",
            documents: &documents,
            custom_llm_provider: None,
            top_n: Some(1),
            rank_fields: Some(&["title".to_owned()]),
            return_documents: Some(false),
            max_chunks_per_doc: Some(2),
            max_tokens_per_doc: Some(3),
            instruction: None,
        })
        .unwrap();
    assert_eq!(mapped.get("query"), Some(&text("which one")));
    assert_eq!(
        mapped.get("documents"),
        Some(&Json::List(vec![text("first"), text("second")]))
    );
    assert_eq!(mapped.get("top_n"), Some(&Json::Number(Number::from(1))));
    assert_eq!(mapped.get("return_documents"), Some(&Json::Bool(false)));
    assert_eq!(mapped.get("rank_fields"), None);
    assert_eq!(mapped.get("max_chunks_per_doc"), None);
    assert_eq!(mapped.get("instruction"), None);
}

#[test]
fn maps_only_the_query_and_documents_when_the_optionals_are_unset() {
    let documents = documents();
    let mut non_default_params = Params::new();
    let mapped = config("")
        .map_cohere_rerank_params(MapCohereRerankParams {
            non_default_params: &mut non_default_params,
            model: "qwen3-rerank",
            drop_params: false,
            query: "which one",
            documents: &documents,
            custom_llm_provider: None,
            top_n: None,
            rank_fields: None,
            return_documents: None,
            max_chunks_per_doc: None,
            max_tokens_per_doc: None,
            instruction: None,
        })
        .unwrap();
    let kept: Vec<&str> = mapped.keys().map(String::as_str).collect();
    assert_eq!(kept, vec!["query", "documents"]);
}

#[test]
fn the_request_body_carries_the_model_query_and_documents() {
    let request = params(&[
        ("query", text("which one")),
        ("documents", Json::List(vec![text("first")])),
        ("top_n", Json::Number(Number::from(1))),
        ("return_documents", Json::Bool(true)),
    ]);
    let body = config("")
        .transform_rerank_request("qwen3-rerank", &request, &Headers::new(), None)
        .unwrap();
    assert_eq!(body.get("model"), Some(&text("qwen3-rerank")));
    assert_eq!(body.get("query"), Some(&text("which one")));
    assert_eq!(
        body.get("documents"),
        Some(&Json::List(vec![text("first")]))
    );
    assert_eq!(body.get("top_n"), Some(&Json::Number(Number::from(1))));
    assert_eq!(body.get("return_documents"), Some(&Json::Bool(true)));
}

#[test]
fn the_request_body_leaves_out_a_null_top_n() {
    let request = params(&[
        ("query", text("which one")),
        ("documents", Json::List(vec![text("first")])),
        ("top_n", Json::Null),
        ("return_documents", Json::Null),
    ]);
    let body = config("")
        .transform_rerank_request("qwen3-rerank", &request, &Headers::new(), None)
        .unwrap();
    assert_eq!(body.get("top_n"), None);
    assert_eq!(body.get("return_documents"), None);
}

#[rstest]
#[case::a_missing_query(params(&[("documents", Json::List(Vec::new()))]), "query is required for DashScope rerank")]
#[case::missing_documents(params(&[("query", text("which one"))]), "documents is required for DashScope rerank")]
fn the_request_body_requires_the_query_and_documents(
    #[case] request: Params,
    #[case] expected: &str,
) {
    let error = config("")
        .transform_rerank_request("qwen3-rerank", &request, &Headers::new(), None)
        .unwrap_err();
    assert!(matches!(
        error,
        DashscopeRerankError::MissingQuery | DashscopeRerankError::MissingDocuments
    ));
    assert_eq!(error.to_string(), expected);
}

#[test]
fn transform_rerank_response_decodes_the_raw_reply() {
    let raw = RerankProviderResponse {
        status_code: 200,
        headers: Headers::new(),
        body: br#"{"id":"r-1","results":[{"index":0,"relevance_score":0.5}],"usage":{"total_tokens":3}}"#
            .to_vec(),
    };
    let mut logging = RerankLogging::default();
    let request_data = Params::new();
    let optional_params = Params::new();
    let litellm_params = Params::new();
    let request = ResponseTransform {
        model: "qwen3-rerank",
        raw_response: &raw,
        logging: &mut logging,
        api_key: None,
        request_data: &request_data,
        optional_params: &optional_params,
        litellm_params: &litellm_params,
    };
    let response = config("")
        .transform_rerank_response(RerankResponse::default(), request)
        .unwrap();
    assert_eq!(response.id.as_deref(), Some("r-1"));
}

#[test]
fn the_reply_decodes_results_documents_and_usage() {
    let body = br#"{"id":"r-1","results":[{"index":0,"relevance_score":0.75,"document":{"text":"kept"}},{"index":1,"relevance_score":0.25,"document":"plain"},{"index":2,"relevance_score":0.1,"document":7}],"usage":{"total_tokens":12}}"#;
    let response = decode_dashscope_rerank_reply(body, 200).unwrap();
    assert_eq!(response.id.as_deref(), Some("r-1"));
    let results = response.results.expect("results");
    assert_eq!(results.len(), 3);
    assert_eq!(results[0].index, 0);
    assert_eq!(
        results[0]
            .document
            .as_ref()
            .map(|document| document.text.as_str()),
        Some("kept")
    );
    assert_eq!(
        results[1]
            .document
            .as_ref()
            .map(|document| document.text.as_str()),
        Some("plain")
    );
    assert_eq!(results[2].document, None);
    let meta = response.meta.expect("meta");
    let billed = meta.billed_units.flatten().expect("billed units");
    assert_eq!(billed.total_tokens, Some(Some(12)));
    let tokens = meta.tokens.flatten().expect("tokens");
    assert_eq!(tokens.input_tokens, Some(Some(12)));
}

#[test]
fn the_reply_leaves_the_token_fields_absent_without_usage() {
    let body = br#"{"id":"r-1","results":[{"index":0,"relevance_score":0.5}]}"#;
    let response = decode_dashscope_rerank_reply(body, 200).unwrap();
    let rendered = serde_json::to_string(&response).unwrap();
    assert!(!rendered.contains("total_tokens"));
    assert!(!rendered.contains("input_tokens"));
}

#[rstest]
#[case::a_truthy_id(br#"{"id":"given","results":[]}"#, Some("given"))]
#[case::an_empty_id(br#"{"id":"","results":[]}"#, None)]
#[case::a_missing_id(br#"{"results":[]}"#, None)]
#[case::a_null_id(br#"{"id":null,"results":[]}"#, None)]
fn the_reply_keeps_a_truthy_id_and_mints_one_otherwise(
    #[case] body: &[u8],
    #[case] expected: Option<&str>,
) {
    let response = decode_dashscope_rerank_reply(body, 200).unwrap();
    let id = response.id.expect("id");
    match expected {
        Some(expected) => assert_eq!(id, expected),
        None => assert!(Uuid::parse_str(&id).is_ok()),
    }
}

#[test]
fn the_reply_reports_a_code_without_results_from_the_message_field() {
    let body = br#"{"code":"InvalidParameter","message":"query is too long","request_id":"x"}"#;
    let error = decode_dashscope_rerank_reply(body, 400).unwrap_err();
    assert_eq!(error.status_code, 400);
    assert_eq!(error.message, "query is too long");
}

#[test]
fn the_reply_reports_a_code_without_a_message_as_the_body() {
    let body = br#"{"code":"InvalidParameter"}"#;
    let error = decode_dashscope_rerank_reply(body, 400).unwrap_err();
    assert_eq!(error.status_code, 400);
    assert_eq!(error.message, r#"{"code":"InvalidParameter"}"#);
}

#[rstest]
#[case::a_missing_results(br#"{"id":"r-1"}"#)]
#[case::a_null_results(br#"{"id":"r-1","results":null}"#)]
fn the_reply_reports_a_missing_results_field(#[case] body: &[u8]) {
    let error = decode_dashscope_rerank_reply(body, 500).unwrap_err();
    assert_eq!(error.status_code, 500);
    assert!(
        error
            .message
            .starts_with("No results in DashScope rerank response: ")
    );
    assert!(error.message.contains("r-1"));
}

#[test]
fn the_reply_reports_a_non_json_body_with_the_reply_status() {
    let error = decode_dashscope_rerank_reply(b"<html>oops</html>", 503).unwrap_err();
    assert_eq!(error.status_code, 503);
    assert_eq!(error.message, "<html>oops</html>");
}

#[rstest]
#[case::a_result_without_an_index(br#"{"results":[{"relevance_score":0.5}]}"#, "index")]
#[case::an_index_that_is_not_a_number(
    br#"{"results":[{"index":"0","relevance_score":0.5}]}"#,
    "index"
)]
#[case::a_result_without_a_score(br#"{"results":[{"index":0}]}"#, "relevance_score")]
#[case::a_score_that_is_not_a_number(
    br#"{"results":[{"index":0,"relevance_score":"high"}]}"#,
    "relevance_score"
)]
#[case::a_document_without_text(
    br#"{"results":[{"index":0,"relevance_score":0.5,"document":{}}]}"#,
    "document"
)]
#[case::results_that_are_not_a_list(br#"{"results":{"index":0}}"#, "results")]
#[case::usage_that_is_not_an_object(br#"{"results":[],"usage":[]}"#, "usage")]
#[case::a_token_count_that_is_not_an_integer(
    br#"{"results":[],"usage":{"total_tokens":1.5}}"#,
    "usage"
)]
fn the_reply_rejects_a_shape_the_records_cannot_hold(#[case] body: &[u8], #[case] field: &str) {
    let error = decode_dashscope_rerank_reply(body, 200).unwrap_err();
    assert_eq!(error.status_code, 500);
    assert!(error.message.contains(field), "{}", error.message);
}

#[rstest]
fn replays_the_dashscope_rerank_reply_corpus(
    #[files("fuzz/corpus/dashscope_rerank_reply/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("read corpus file");
    let _decoded = decode_dashscope_rerank_reply(&bytes, 200);
    let _failed = decode_dashscope_rerank_reply(&bytes, 500);
}
