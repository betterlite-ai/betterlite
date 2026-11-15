#![cfg(test)]

use std::path::PathBuf;

use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::SecretString;
use uuid::{Uuid, Version};

use super::error::FireworksRerankError;
use super::response::{
    IdSource, MAX_RESPONSE_BYTES, UuidIds, decode_rerank_response, transform_rerank_response,
};
use super::wire::{OptionalRerankParams, transform_rerank_request};
use super::{
    CohereRerankCall, DEFAULT_RERANK_URL, DEFAULT_RETURN_DOCUMENTS, SUPPORTED_COHERE_RERANK_PARAMS,
    get_complete_url, map_cohere_rerank_params, validate_environment,
};
use crate::api::rerank_contracts::records::{RerankDocument, RerankResponse};
use crate::fireworks_ai_common::{AUTHORIZATION_HEADER, CONTENT_TYPE_HEADER, FireworksEnv};
use crate::openai_base_config::Headers;

fn headers() -> Headers {
    Headers::new()
}

fn key(text: &str) -> Option<SecretString> {
    Some(SecretString::from(text))
}

struct FixedIds;

impl IdSource for FixedIds {
    fn new_uuid(&self) -> Uuid {
        Uuid::from_u128(1)
    }
}

fn params(
    query: Option<&str>,
    documents: Option<Vec<RerankDocument>>,
    top_n: Option<i64>,
    return_documents: Option<bool>,
) -> OptionalRerankParams {
    OptionalRerankParams {
        query: query.map(str::to_owned),
        documents,
        top_n,
        return_documents,
    }
}

#[test]
fn the_default_rerank_url_is_the_fireworks_endpoint() {
    assert_eq!(
        get_complete_url(None),
        "https://api.fireworks.ai/inference/v1/rerank"
    );
    assert_eq!(get_complete_url(Some("")), DEFAULT_RERANK_URL);
}

#[rstest]
#[case::strips_trailing_slashes(
    "https://api.fireworks.ai/inference/v1/",
    "https://api.fireworks.ai/inference/v1/rerank"
)]
#[case::v1_base("https://api.fireworks.ai/v1", "https://api.fireworks.ai/v1/rerank")]
#[case::inference_v1_base(
    "https://api.fireworks.ai/inference/v1",
    "https://api.fireworks.ai/inference/v1/rerank"
)]
#[case::already_a_rerank_url("https://proxy.example/rerank/", "https://proxy.example/rerank")]
#[case::other_base("https://proxy.example", "https://proxy.example/inference/v1/rerank")]
fn get_complete_url_appends_the_rerank_path(#[case] api_base: &str, #[case] expected: &str) {
    assert_eq!(get_complete_url(Some(api_base)), expected);
}

#[test]
fn the_supported_params_are_coheres_four() {
    assert_eq!(
        SUPPORTED_COHERE_RERANK_PARAMS,
        ["query", "documents", "top_n", "return_documents"]
    );
}

#[test]
fn map_cohere_rerank_params_keeps_the_two_required_params() {
    let documents = vec![RerankDocument::Text("a".to_owned())];
    let mapped = map_cohere_rerank_params(&CohereRerankCall {
        query: "find",
        documents: &documents,
        top_n: Some(2),
        return_documents: Some(DEFAULT_RETURN_DOCUMENTS),
        rank_fields: None,
        max_chunks_per_doc: None,
        max_tokens_per_doc: None,
        instruction: None,
    });
    assert_eq!(
        mapped,
        params(
            Some("find"),
            Some(documents),
            Some(2),
            Some(DEFAULT_RETURN_DOCUMENTS)
        )
    );
}

#[test]
fn map_cohere_rerank_params_drops_the_unsupported_params() {
    let documents = vec![RerankDocument::Text("a".to_owned())];
    let rank_fields = vec!["title".to_owned()];
    let mapped = map_cohere_rerank_params(&CohereRerankCall {
        query: "find",
        documents: &documents,
        top_n: None,
        return_documents: None,
        rank_fields: Some(&rank_fields),
        max_chunks_per_doc: Some(3),
        max_tokens_per_doc: Some(4),
        instruction: Some("rank"),
    });
    assert_eq!(mapped, params(Some("find"), Some(documents), None, None));
}

fn env_with(text: Option<&str>) -> FireworksEnv {
    FireworksEnv {
        fireworks_api_key: text.map(SecretString::from),
        fireworks_ai_api_key: None,
        fireworksai_api_key: None,
        fireworks_ai_token: None,
    }
}

#[test]
fn rerank_validate_environment_names_both_key_variables_when_missing() {
    let error = validate_environment(&headers(), None, &FireworksEnv::default()).unwrap_err();
    assert!(matches!(error, FireworksRerankError::MissingApiKey));
    let message = error.to_string();
    assert!(message.contains("FIREWORKS_API_KEY"));
    assert!(message.contains("FIREWORKS_AI_API_KEY"));
}

#[test]
fn rerank_validate_environment_sets_the_default_headers() {
    let built = validate_environment(
        &Headers::new(),
        key("direct").as_ref(),
        &FireworksEnv::default(),
    )
    .unwrap();
    assert_eq!(
        built.get(AUTHORIZATION_HEADER).map(String::as_str),
        Some("Bearer direct")
    );
    assert_eq!(
        built.get(CONTENT_TYPE_HEADER).map(String::as_str),
        Some("application/json")
    );
    assert_eq!(built.len(), 2);
}

#[test]
fn rerank_validate_environment_lets_an_exact_authorization_override() {
    let given = Headers::from([("Authorization".to_owned(), "Bearer caller".to_owned())]);
    let built =
        validate_environment(&given, key("direct").as_ref(), &FireworksEnv::default()).unwrap();
    assert_eq!(
        built.get(AUTHORIZATION_HEADER).map(String::as_str),
        Some("Bearer caller")
    );
}

#[test]
fn rerank_validate_environment_keeps_a_lowercase_authorization_apart() {
    let given = Headers::from([("authorization".to_owned(), "Bearer caller".to_owned())]);
    let built =
        validate_environment(&given, key("direct").as_ref(), &FireworksEnv::default()).unwrap();
    assert_eq!(
        built.get(AUTHORIZATION_HEADER).map(String::as_str),
        Some("Bearer direct")
    );
    assert_eq!(
        built.get("authorization").map(String::as_str),
        Some("Bearer caller")
    );
}

#[test]
fn rerank_validate_environment_lets_other_caller_headers_override() {
    let given = Headers::from([("Content-Type".to_owned(), "text/plain".to_owned())]);
    let built =
        validate_environment(&given, key("direct").as_ref(), &FireworksEnv::default()).unwrap();
    assert_eq!(
        built.get(CONTENT_TYPE_HEADER).map(String::as_str),
        Some("text/plain")
    );
}

fn documents() -> Vec<RerankDocument> {
    vec![RerankDocument::Text("a".to_owned())]
}

#[test]
fn transform_rerank_request_needs_query_and_documents() {
    let missing_query =
        transform_rerank_request("qwen", &params(None, Some(documents()), None, None)).unwrap_err();
    assert!(matches!(missing_query, FireworksRerankError::MissingQuery));
    let missing_documents =
        transform_rerank_request("qwen", &params(Some("find"), None, None, None)).unwrap_err();
    assert!(matches!(
        missing_documents,
        FireworksRerankError::MissingDocuments
    ));
}

#[rstest]
#[case::adds_the_prefix("fireworks_ai/qwen3-reranker-8b", "fireworks/qwen3-reranker-8b")]
#[case::keeps_an_existing_prefix(
    "fireworks_ai/fireworks/qwen3-reranker-8b",
    "fireworks/qwen3-reranker-8b"
)]
#[case::plain_name("qwen3-reranker-8b", "fireworks/qwen3-reranker-8b")]
#[case::replaces_every_provider_prefix("fireworks_ai/fireworks_ai/qwen", "fireworks/qwen")]
fn transform_rerank_request_rewrites_the_model_name(#[case] model: &str, #[case] expected: &str) {
    let body =
        transform_rerank_request(model, &params(Some("find"), Some(documents()), None, None))
            .unwrap();
    assert_eq!(body.model, expected);
}

#[test]
fn transform_rerank_request_sends_the_optional_params_only_when_present() {
    let body = transform_rerank_request(
        "qwen",
        &params(Some("find"), Some(documents()), Some(3), Some(false)),
    )
    .unwrap();
    assert_eq!(body.query, "find");
    assert_eq!(body.documents, documents());
    assert_eq!(body.top_n, Some(3));
    assert_eq!(body.return_documents, Some(false));
    let plain =
        transform_rerank_request("qwen", &params(Some("find"), Some(documents()), None, None))
            .unwrap();
    assert_eq!(plain.top_n, None);
    assert_eq!(plain.return_documents, None);
    let encoded = serde_json::to_value(&plain).unwrap();
    assert_eq!(
        encoded,
        serde_json::json!({
            "model": "fireworks/qwen",
            "query": "find",
            "documents": ["a"],
        })
    );
}

fn body(text: &str) -> Vec<u8> {
    text.as_bytes().to_vec()
}

fn transform(status: u16, payload: &str) -> Result<RerankResponse, FireworksRerankError> {
    transform_rerank_response(status, &headers(), payload.as_bytes(), &FixedIds)
}

#[test]
fn transform_rerank_response_projects_data_results_and_usage() {
    let response = transform(
        200,
        r#"{"id":"body-id","data":[{"index":0,"relevance_score":0.95,"document":"first"},{"index":1,"relevance_score":0.75}],"usage":{"total_tokens":100,"prompt_tokens":50,"completion_tokens":50}}"#,
    )
    .unwrap();
    assert_eq!(response.id.as_deref(), Some("body-id"));
    let results = response.results.unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].index, 0);
    assert_eq!(results[0].relevance_score.0, Decimal::new(95, 2));
    assert_eq!(
        results[0].document,
        Some(
            crate::api::rerank_contracts::records::RerankResponseDocument {
                text: "first".to_owned()
            }
        )
    );
    assert_eq!(results[1].document, None);
    let meta = response.meta.unwrap();
    let billed = meta.billed_units.unwrap().unwrap();
    assert_eq!(billed.search_units, Some(Some(100)));
    assert_eq!(billed.total_tokens, None);
    let tokens = meta.tokens.unwrap().unwrap();
    assert_eq!(tokens.input_tokens, Some(Some(50)));
    assert_eq!(tokens.output_tokens, Some(Some(50)));
    assert_eq!(meta.api_version, None);
}

#[test]
fn transform_rerank_response_stamps_a_fresh_id_only_without_a_body_id() {
    let stamped = transform(200, r#"{"data":[{"index":0,"relevance_score":1}]}"#).unwrap();
    assert_eq!(stamped.id, Some(Uuid::from_u128(1).to_string()));
    let empty = transform(200, r#"{"id":"","data":[{"index":0,"relevance_score":1}]}"#).unwrap();
    assert_eq!(empty.id, Some(Uuid::from_u128(1).to_string()));
}

#[test]
fn transform_rerank_response_defaults_usage_to_zero() {
    let response = transform(200, r#"{"data":[{"index":0,"relevance_score":1}]}"#).unwrap();
    let meta = response.meta.unwrap();
    let billed = meta.billed_units.unwrap().unwrap();
    assert_eq!(billed.search_units, Some(Some(0)));
    let tokens = meta.tokens.unwrap().unwrap();
    assert_eq!(tokens.input_tokens, Some(Some(0)));
    assert_eq!(tokens.output_tokens, Some(Some(0)));
}

#[test]
fn transform_rerank_response_falls_back_to_results_when_data_is_empty() {
    let response = transform(
        200,
        r#"{"data":[],"results":[{"index":2,"relevance_score":0.5}]}"#,
    )
    .unwrap();
    let results = response.results.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].index, 2);
}

#[rstest]
#[case::empty_object("{}")]
#[case::empty_data(r#"{"data":[]}"#)]
fn transform_rerank_response_errors_when_neither_data_nor_results_exist(#[case] payload: &str) {
    let error = transform(200, payload).unwrap_err();
    assert!(
        matches!(error, FireworksRerankError::NoResults),
        "{payload}"
    );
}

#[test]
fn transform_rerank_response_returns_no_results_when_both_lists_are_empty() {
    let response = transform(200, r#"{"data":[],"results":[]}"#).unwrap();
    assert_eq!(response.results, Some(Vec::new()));
}

#[test]
fn transform_rerank_response_requires_index_and_relevance_score() {
    let missing_index = transform(200, r#"{"data":[{"relevance_score":1}]}"#).unwrap_err();
    assert!(matches!(
        missing_index,
        FireworksRerankError::MissingResultField { field: "index" }
    ));
    let missing_score = transform(200, r#"{"data":[{"index":0}]}"#).unwrap_err();
    assert!(matches!(
        missing_score,
        FireworksRerankError::MissingResultField {
            field: "relevance_score"
        }
    ));
}

#[test]
fn transform_rerank_response_needs_an_integer_index() {
    let fractional = transform(200, r#"{"data":[{"index":1.5,"relevance_score":1}]}"#).unwrap_err();
    assert!(matches!(fractional, FireworksRerankError::IndexShape));
    let text = transform(200, r#"{"data":[{"index":"0","relevance_score":1}]}"#).unwrap_err();
    let FireworksRerankError::Fireworks(error) = text else {
        panic!("expected a FireworksError for a string index");
    };
    assert_eq!(error.status_code, 200);
    assert!(error.message.starts_with("Failed to parse response"));
}

#[rstest]
#[case::non_string_id(r#"{"id":5,"data":[{"index":0,"relevance_score":1}]}"#)]
#[case::non_number_score(r#"{"data":[{"index":0,"relevance_score":"0.5"}]}"#)]
fn transform_rerank_response_rejects_a_non_string_id_and_a_non_number_score(#[case] payload: &str) {
    let error = transform(200, payload).unwrap_err();
    let FireworksRerankError::Fireworks(error) = error else {
        panic!("expected a FireworksError for {payload}");
    };
    assert_eq!(error.status_code, 200);
    assert!(error.message.starts_with("Failed to parse response"));
}

#[rstest]
#[case::text_document(r#""text""#, Some("text"))]
#[case::object_document(r#"{"text":"text"}"#, Some("text"))]
#[case::empty_text(r#""""#, None)]
#[case::empty_object_text(r#"{"text":""}"#, None)]
#[case::object_without_text(r#"{"page":1}"#, None)]
#[case::number_object_text(r#"{"text":7}"#, None)]
#[case::number_document("7", None)]
#[case::null_document("null", None)]
fn transform_rerank_response_adds_a_document_only_for_text(
    #[case] document: &str,
    #[case] expected: Option<&str>,
) {
    let payload =
        format!(r#"{{"data":[{{"index":0,"relevance_score":1,"document":{document}}}]}}"#);
    let response = transform(200, &payload).unwrap();
    assert_eq!(
        response.results.unwrap()[0]
            .document
            .as_ref()
            .map(|document| document.text.as_str()),
        expected
    );
}

#[test]
fn transform_rerank_response_rejects_a_body_that_is_not_an_object() {
    let error = transform(200, "[1,2]").unwrap_err();
    let FireworksRerankError::Fireworks(error) = error else {
        panic!("expected a FireworksError");
    };
    assert_eq!(error.status_code, 200);
    assert!(error.message.starts_with("Failed to parse response"));
}

#[test]
fn transform_rerank_response_reports_a_parse_failure_with_the_http_status() {
    let given = Headers::from([("x-request-id".to_owned(), "request-1".to_owned())]);
    let error = transform_rerank_response(502, &given, br#"{"data": [}"#, &FixedIds).unwrap_err();
    let FireworksRerankError::Fireworks(error) = error else {
        panic!("expected a FireworksError");
    };
    assert_eq!(error.status_code, 502);
    assert_eq!(error.headers, Some(given));
    assert!(error.message.starts_with("Failed to parse response"));
}

#[test]
fn decode_rerank_response_rejects_a_body_over_one_mib() {
    let bytes = vec![b' '; MAX_RESPONSE_BYTES + 1];
    let error = decode_rerank_response(&bytes).unwrap_err();
    assert!(matches!(
        error,
        FireworksRerankError::ResponseTooLarge { limit, .. } if limit == MAX_RESPONSE_BYTES
    ));
}

#[test]
fn decode_rerank_response_accepts_a_body_of_exactly_the_limit() {
    let mut bytes = vec![b' '; MAX_RESPONSE_BYTES];
    bytes[0] = b'{';
    bytes[MAX_RESPONSE_BYTES - 1] = b'}';
    assert!(decode_rerank_response(&bytes).is_ok());
}

#[rstest]
fn replays_the_rerank_response_corpus(
    #[files("fuzz/corpus/fireworks_ai_rerank_response/*")] path: PathBuf,
) {
    let _decoded = decode_rerank_response(&std::fs::read(path).expect("corpus file"));
}

#[test]
fn the_response_limit_is_one_mib() {
    assert_eq!(MAX_RESPONSE_BYTES, 1024 * 1024);
}

#[test]
fn uuid_ids_produce_a_v4_uuid() {
    assert_eq!(UuidIds.new_uuid().get_version(), Some(Version::Random));
}
