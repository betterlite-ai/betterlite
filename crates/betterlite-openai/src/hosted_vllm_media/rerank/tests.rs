#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Map, Value};

use super::{
    CohereRerankCall, HostedVllmRerankError, MappedRerankParams, get_complete_url, get_error_class,
    get_supported_cohere_rerank_params, map_cohere_rerank_params, transform_rerank_request,
    validate_environment, validated_truncation_params,
};
use crate::api::rerank_contracts::records::{RerankDocument, TruncationSide};
use crate::openai_base_config::Headers;
use crate::together_ai_rerank::ReturnDocuments;

type Environment = BTreeMap<String, String>;

fn value(text: &str) -> Value {
    serde_json::from_str(text).expect("the case is JSON")
}

fn environment(key: Option<&str>) -> Environment {
    key.map(|value| BTreeMap::from([(String::from(super::API_KEY_ENV), String::from(value))]))
        .unwrap_or_default()
}

fn params(value: Value) -> Map<String, Value> {
    let Value::Object(fields) = value else {
        panic!("test parameters must be an object");
    };
    fields
}

fn documents() -> Vec<RerankDocument> {
    vec![
        RerankDocument::Text(String::from("first")),
        RerankDocument::Object(IndexMap::from([(
            String::from("text"),
            Json::Text(String::from("second")),
        )])),
    ]
}

fn call<'request>(
    documents: &'request [RerankDocument],
    non_default_params: &'request Map<String, Value>,
) -> CohereRerankCall<'request> {
    CohereRerankCall {
        model: "model",
        query: "query",
        documents,
        top_n: Some(2),
        rank_fields: None,
        return_documents: ReturnDocuments::Default,
        max_chunks_per_doc: None,
        max_tokens_per_doc: Some(8),
        instruction: None,
        non_default_params,
    }
}

#[test]
fn supported_params_are_litellms_ten_in_order() {
    assert_eq!(
        get_supported_cohere_rerank_params(),
        vec![
            "query",
            "documents",
            "top_n",
            "rank_fields",
            "return_documents",
            "max_tokens_per_doc",
            "instruction",
            "truncate_prompt_tokens",
            "truncation_side",
            "max_tokens_per_query",
        ]
    );
}

#[test]
fn truncation_params_read_the_four_keys_and_ignore_the_rest() {
    let validated = validated_truncation_params(&params(value(
        r#"{
            "truncate_prompt_tokens": 4,
            "truncation_side": "right",
            "max_tokens_per_query": 6,
            "max_tokens_per_doc": 8,
            "top_n": 2
        }"#,
    )))
    .expect("the truncation keys are well-formed");
    assert_eq!(validated.truncate_prompt_tokens, Some(4));
    assert_eq!(validated.truncation_side, Some(TruncationSide::Right));
    assert_eq!(validated.max_tokens_per_query, Some(6));
    assert_eq!(validated.max_tokens_per_doc, Some(8));
}

#[test]
fn truncation_params_reject_a_side_litellm_does_not_declare() {
    assert!(matches!(
        validated_truncation_params(&params(value(r#"{"truncation_side": "up"}"#))),
        Err(HostedVllmRerankError::Truncation { .. })
    ));
}

#[test]
fn mapped_params_carry_the_defaults_litellm_writes() {
    let documents = documents();
    let non_default = params(value("{}"));
    let mapped = map_cohere_rerank_params(&call(&documents, &non_default))
        .expect("the parameters are well-formed");
    assert_eq!(mapped.query, "query");
    assert_eq!(mapped.documents, documents);
    assert_eq!(mapped.top_n, Some(2));
    assert_eq!(mapped.rank_fields, None);
    assert_eq!(mapped.return_documents, Some(true));
    assert_eq!(mapped.max_tokens_per_doc, Some(8));
    assert_eq!(mapped.truncation_side, None);
    let encoded = serde_json::to_value(&mapped).expect("the mapped params serialize");
    assert!(encoded.get("instruction").is_none());
    assert_eq!(encoded.get("rank_fields"), Some(&Value::Null));
}

#[rstest]
#[case(ReturnDocuments::Default, Some(true))]
#[case(ReturnDocuments::Null, None)]
#[case(ReturnDocuments::Value(false), Some(false))]
fn mapped_params_render_return_documents_each_way(
    #[case] given: ReturnDocuments,
    #[case] expected: Option<bool>,
) {
    let documents = documents();
    let non_default = params(value("{}"));
    let mut request = call(&documents, &non_default);
    request.return_documents = given;
    assert_eq!(
        map_cohere_rerank_params(&request)
            .expect("the parameters are well-formed")
            .return_documents,
        expected
    );
}

#[test]
fn mapped_params_forward_the_instruction_and_truncation() {
    let documents = documents();
    let non_default = params(value(
        r#"{
            "truncate_prompt_tokens": 4,
            "truncation_side": "left",
            "max_tokens_per_query": 6
        }"#,
    ));
    let mut request = call(&documents, &non_default);
    request.instruction = Some("rank by relevance");
    let mapped = map_cohere_rerank_params(&request).expect("the parameters are well-formed");
    assert_eq!(mapped.instruction.as_deref(), Some("rank by relevance"));
    assert_eq!(mapped.truncate_prompt_tokens, Some(4));
    assert_eq!(mapped.truncation_side, Some(TruncationSide::Left));
    assert_eq!(mapped.max_tokens_per_query, Some(6));
}

#[test]
fn mapped_params_take_max_tokens_per_doc_from_the_argument_not_the_params() {
    let documents = documents();
    let non_default = params(value(r#"{"max_tokens_per_doc": 99}"#));
    let mapped = map_cohere_rerank_params(&call(&documents, &non_default))
        .expect("the parameters are well-formed");
    assert_eq!(mapped.max_tokens_per_doc, Some(8));
}

#[test]
fn mapped_params_refuse_max_chunks_per_doc_before_validating_truncation() {
    let documents = documents();
    let non_default = params(value(r#"{"truncation_side": "up"}"#));
    let mut request = call(&documents, &non_default);
    request.max_chunks_per_doc = Some(3);
    assert!(matches!(
        map_cohere_rerank_params(&request),
        Err(HostedVllmRerankError::MaxChunksPerDoc)
    ));
}

#[test]
fn mapped_params_refuse_a_bad_truncation_value() {
    let documents = documents();
    let non_default = params(value(r#"{"truncation_side": "up"}"#));
    assert!(matches!(
        map_cohere_rerank_params(&call(&documents, &non_default)),
        Err(HostedVllmRerankError::Truncation { .. })
    ));
}

#[rstest]
#[case("http://host", "http://host/rerank")]
#[case("http://host/", "http://host/rerank")]
#[case("http://host/rerank", "http://host/rerank")]
#[case("http://host/rerank/", "http://host/rerank")]
#[case("http://host/v1/rerank", "http://host/rerank")]
#[case("http://host/v1/rerank/", "http://host/rerank")]
fn complete_url_rewrites_the_v1_suffix(#[case] api_base: &str, #[case] expected: &str) {
    assert_eq!(
        get_complete_url(Some(api_base)).expect("a base is set"),
        expected
    );
}

#[rstest]
#[case(None)]
#[case(Some(""))]
fn complete_url_requires_a_base(#[case] api_base: Option<&str>) {
    assert!(matches!(
        get_complete_url(api_base),
        Err(HostedVllmRerankError::MissingApiBase)
    ));
}

#[test]
fn headers_carry_the_explicit_key_and_the_json_content_type() {
    let key = SecretString::from(String::from("key"));
    assert_eq!(
        validate_environment(&Headers::new(), Some(&key), &environment(None)),
        Headers::from([
            (String::from("Authorization"), String::from("Bearer key")),
            (String::from("accept"), String::from("application/json")),
            (
                String::from("content-type"),
                String::from("application/json")
            ),
        ])
    );
}

#[test]
fn headers_read_the_environment_key_and_keep_an_empty_explicit_one() {
    assert_eq!(
        validate_environment(&Headers::new(), None, &environment(Some("env-key")))
            .get("Authorization"),
        Some(&String::from("Bearer env-key"))
    );
    let empty = SecretString::from(String::new());
    assert_eq!(
        validate_environment(&Headers::new(), Some(&empty), &environment(Some("env-key")))
            .get("Authorization"),
        Some(&String::from("Bearer "))
    );
}

#[test]
fn headers_use_the_placeholder_without_any_key() {
    assert_eq!(
        validate_environment(&Headers::new(), None, &environment(None)).get("Authorization"),
        Some(&String::from("Bearer fake-api-key"))
    );
}

#[test]
fn headers_use_the_placeholder_for_an_empty_environment_key() {
    assert_eq!(
        validate_environment(&Headers::new(), None, &environment(Some(""))).get("Authorization"),
        Some(&String::from("Bearer fake-api-key"))
    );
}

#[test]
fn headers_let_the_caller_override_authorization_and_add_their_own() {
    let supplied = Headers::from([
        (String::from("Authorization"), String::from("Bearer caller")),
        (String::from("x-extra"), String::from("1")),
    ]);
    assert_eq!(
        validate_environment(&supplied, None, &environment(None)),
        Headers::from([
            (String::from("Authorization"), String::from("Bearer caller")),
            (String::from("accept"), String::from("application/json")),
            (
                String::from("content-type"),
                String::from("application/json")
            ),
            (String::from("x-extra"), String::from("1")),
        ])
    );
}

#[test]
fn headers_keep_a_differently_cased_authorization_as_a_second_entry() {
    let supplied = Headers::from([(String::from("authorization"), String::from("Bearer caller"))]);
    assert_eq!(
        validate_environment(&supplied, None, &environment(None)),
        Headers::from([
            (
                String::from("Authorization"),
                String::from("Bearer fake-api-key")
            ),
            (String::from("accept"), String::from("application/json")),
            (
                String::from("content-type"),
                String::from("application/json")
            ),
            (String::from("authorization"), String::from("Bearer caller")),
        ])
    );
}

#[test]
fn request_body_drops_the_null_keys_and_keeps_the_lists() {
    let documents = documents();
    let non_default = params(value(r#"{"truncate_prompt_tokens": 4}"#));
    let mapped = map_cohere_rerank_params(&call(&documents, &non_default))
        .expect("the parameters are well-formed");
    let body = transform_rerank_request("model", &mapped).expect("the request encodes");
    assert_eq!(
        Value::Object(body),
        value(
            r#"{
                "model": "model",
                "query": "query",
                "top_n": 2,
                "documents": ["first", {"text": "second"}],
                "return_documents": true,
                "max_tokens_per_doc": 8,
                "truncate_prompt_tokens": 4
            }"#,
        )
    );
}

#[test]
fn request_body_accepts_a_mapped_dict_with_only_the_required_keys() {
    let mapped: MappedRerankParams = serde_json::from_value(value(
        r#"{
            "query": "query",
            "documents": ["only"],
            "return_documents": false
        }"#,
    ))
    .expect("the mapped dict decodes");
    let body = transform_rerank_request("model", &mapped).expect("the request encodes");
    assert_eq!(
        Value::Object(body),
        value(
            r#"{
                "model": "model",
                "query": "query",
                "documents": ["only"],
                "return_documents": false
            }"#,
        )
    );
}

#[rstest]
#[case(r#"{"documents": ["only"]}"#)]
#[case(r#"{"query": "query"}"#)]
fn a_mapped_dict_without_query_or_documents_does_not_decode(#[case] text: &str) {
    let error = serde_json::from_value::<MappedRerankParams>(value(text))
        .expect_err("a mapped dict needs query and documents");
    assert!(error.to_string().contains("missing field"));
}

#[test]
fn error_class_carries_the_status_message_and_headers() {
    let headers = Headers::from([(String::from("x-extra"), String::from("1"))]);
    let error = get_error_class("bad request", 422, &headers);
    assert_eq!(error.status_code, 422);
    assert_eq!(error.message, "bad request");
    assert_eq!(
        error.headers,
        Some(vec![(String::from("x-extra"), String::from("1"))])
    );
    assert!(!error.status_code_is_synthesized);
}
