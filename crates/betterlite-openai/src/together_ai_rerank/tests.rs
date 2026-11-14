#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    RerankCall, ReturnDocuments, TogetherRerankError, decode_response, request, transform_response,
};
use crate::api::rerank_contracts::records::RerankDocument;

fn fixed_id() -> Uuid {
    Uuid::from_u128(7)
}

fn key() -> SecretString {
    SecretString::from("sk-together")
}

fn base_call<'request>(
    key: &'request SecretString,
    documents: &'request [RerankDocument],
) -> RerankCall<'request> {
    RerankCall {
        model: "together/llama",
        query: "which one",
        documents,
        top_n: None,
        rank_fields: None,
        return_documents: ReturnDocuments::Default,
        max_chunks_per_doc: None,
        api_key: key,
        api_base: "https://api.together.ai/v1",
    }
}

#[test]
fn the_body_lists_only_the_present_keys_in_litellm_order() {
    let key = key();
    let documents = [RerankDocument::Text(String::from("one"))];
    let http = request(&base_call(&key, &documents)).unwrap();
    assert_eq!(
        http.body.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["model", "query", "documents", "return_documents"]
    );
    assert_eq!(http.body.get("return_documents"), Some(&json!(true)));
    assert_eq!(http.url, "https://api.together.ai/v1/rerank");
}

#[test]
fn top_n_and_rank_fields_sit_between_query_and_return_documents() {
    let key = key();
    let documents = [RerankDocument::Text(String::from("one"))];
    let fields = [String::from("title"), String::from("body")];
    let mut call = base_call(&key, &documents);
    call.top_n = Some(3);
    call.rank_fields = Some(&fields);
    let http = request(&call).unwrap();
    assert_eq!(
        http.body.keys().map(String::as_str).collect::<Vec<_>>(),
        vec![
            "model",
            "query",
            "top_n",
            "documents",
            "rank_fields",
            "return_documents"
        ]
    );
    assert_eq!(http.body.get("top_n"), Some(&json!(3)));
    assert_eq!(
        http.body.get("rank_fields"),
        Some(&json!(["title", "body"]))
    );
}

#[rstest]
#[case::default(ReturnDocuments::Default, Some(true))]
#[case::null(ReturnDocuments::Null, None)]
#[case::false_value(ReturnDocuments::Value(false), Some(false))]
#[case::true_value(ReturnDocuments::Value(true), Some(true))]
fn return_documents_follows_the_argument(
    #[case] returned: ReturnDocuments,
    #[case] expected: Option<bool>,
) {
    let key = key();
    let documents = [RerankDocument::Text(String::from("one"))];
    let mut call = base_call(&key, &documents);
    call.return_documents = returned;
    let http = request(&call).unwrap();
    assert_eq!(
        http.body.get("return_documents"),
        expected.map(Value::from).as_ref()
    );
}

#[test]
fn documents_keep_their_string_and_object_shapes() {
    let key = key();
    let documents = [
        RerankDocument::Text(String::from("one")),
        RerankDocument::Object(IndexMap::from([(
            String::from("text"),
            Json::Text(String::from("two")),
        )])),
    ];
    let http = request(&base_call(&key, &documents)).unwrap();
    assert_eq!(
        http.body.get("documents"),
        Some(&json!(["one", {"text": "two"}]))
    );
}

#[test]
fn max_chunks_per_doc_is_refused() {
    let key = key();
    let documents = [RerankDocument::Text(String::from("one"))];
    let mut call = base_call(&key, &documents);
    call.max_chunks_per_doc = Some(2);
    let error = request(&call).err().unwrap();
    assert!(matches!(error, TogetherRerankError::MaxChunksPerDoc));
    assert_eq!(
        error.to_string(),
        "TogetherAI does not support max_chunks_per_doc"
    );
}

#[rstest]
#[case::plain("https://api.example/v1", "https://api.example/v1/rerank")]
#[case::one_trailing("https://api.example/v1/", "https://api.example/v1/rerank")]
#[case::many_trailing("https://api.example/v1///", "https://api.example/v1/rerank")]
fn the_url_strips_every_trailing_slash(#[case] base: &str, #[case] expected: &str) {
    let key = key();
    let documents = [RerankDocument::Text(String::from("one"))];
    let mut call = base_call(&key, &documents);
    call.api_base = base;
    assert_eq!(request(&call).unwrap().url, expected);
}

#[test]
fn the_headers_are_lowercase_and_bearer() {
    let key = key();
    let documents = [RerankDocument::Text(String::from("one"))];
    let headers = request(&base_call(&key, &documents)).unwrap().headers;
    assert_eq!(
        headers.get("accept"),
        Some(&String::from("application/json"))
    );
    assert_eq!(
        headers.get("content-type"),
        Some(&String::from("application/json"))
    );
    assert_eq!(
        headers.get("authorization"),
        Some(&String::from("Bearer sk-together"))
    );
    assert_eq!(headers.len(), 3);
}

#[test]
fn a_200_status_passes() {
    assert!(decode_response(200, "{}").is_ok());
}

#[rstest]
#[case::created(201)]
#[case::bad_request(400)]
#[case::server_error(500)]
fn a_non_200_status_fails_with_the_raw_text(#[case] status: u16) {
    let error = decode_response(status, "{not json").unwrap_err();
    assert_eq!(error.to_string(), "{not json");
}

#[test]
fn the_reply_keeps_the_id_results_and_documents() {
    let response = json!({
        "id": "req-1",
        "results": [
            {"index": 0, "relevance_score": 0.93, "document": {"text": "doc one", "extra": 1}},
            {"index": 1, "relevance_score": 0.5, "document": {}},
            {"index": 2, "relevance_score": 0.5},
        ],
    });
    let reply = serde_json::to_value(transform_response(&response, &fixed_id).unwrap()).unwrap();
    assert_eq!(reply["id"], json!("req-1"));
    assert_eq!(
        reply["results"],
        json!([
            {"index": 0, "relevance_score": 0.93, "document": {"text": "doc one"}},
            {"index": 1, "relevance_score": 0.5},
            {"index": 2, "relevance_score": 0.5},
        ])
    );
}

#[rstest]
#[case::missing(json!({"results": []}))]
#[case::null(json!({"id": null, "results": []}))]
#[case::empty(json!({"id": "", "results": []}))]
fn a_missing_or_falsy_id_uses_the_injected_uuid(#[case] response: Value) {
    let reply = serde_json::to_value(transform_response(&response, &fixed_id).unwrap()).unwrap();
    assert_eq!(reply["id"], json!(Uuid::from_u128(7).to_string()));
}

#[test]
fn both_meta_blocks_are_filled_from_the_usage_object() {
    let response = json!({
        "id": "req-1",
        "results": [],
        "usage": {"search_units": 2, "input_tokens": 9, "prompt_tokens": 4},
    });
    let reply = serde_json::to_value(transform_response(&response, &fixed_id).unwrap()).unwrap();
    assert_eq!(
        reply["meta"],
        json!({
            "billed_units": {"search_units": 2},
            "tokens": {"input_tokens": 9},
        })
    );
}

#[rstest]
#[case::absent(json!({"results": []}))]
#[case::null(json!({"results": [], "usage": null}))]
fn an_absent_usage_is_an_empty_object(#[case] response: Value) {
    let reply = serde_json::to_value(transform_response(&response, &fixed_id).unwrap()).unwrap();
    assert_eq!(reply["meta"], json!({"billed_units": {}, "tokens": {}}));
}

#[test]
fn a_null_usage_field_is_kept_as_null() {
    let response = json!({"results": [], "usage": {"total_tokens": null}});
    let reply = serde_json::to_value(transform_response(&response, &fixed_id).unwrap()).unwrap();
    assert_eq!(reply["meta"]["billed_units"], json!({"total_tokens": null}));
}

#[test]
fn a_non_object_usage_is_refused() {
    let error = transform_response(&json!({"results": [], "usage": [1]}), &fixed_id).unwrap_err();
    assert!(matches!(error, TogetherRerankError::InvalidUsage { .. }));
}

#[test]
fn a_non_integer_usage_field_is_refused() {
    let response = json!({"results": [], "usage": {"search_units": "two"}});
    let error = transform_response(&response, &fixed_id).unwrap_err();
    assert!(matches!(error, TogetherRerankError::UsageField { .. }));
}

#[rstest]
#[case::missing(json!({"id": "req-1"}))]
#[case::null(json!({"id": "req-1", "results": null}))]
#[case::not_a_list(json!({"id": "req-1", "results": "none"}))]
fn missing_results_fail_with_the_response_text(#[case] response: Value) {
    let error = transform_response(&response, &fixed_id).unwrap_err();
    assert!(matches!(error, TogetherRerankError::NoResults { .. }));
    assert!(
        error
            .to_string()
            .starts_with("No results found in the response=")
    );
    assert!(error.to_string().contains("req-1"));
}

#[rstest]
#[case::no_index(json!([{"relevance_score": 0.5}]))]
#[case::no_score(json!([{"index": 0}]))]
#[case::string_index(json!([{"index": "0", "relevance_score": 0.5}]))]
#[case::string_score(json!([{"index": 0, "relevance_score": "0.5"}]))]
#[case::not_an_object(json!([5]))]
fn a_result_without_its_required_fields_fails(#[case] results: Value) {
    let response = json!({"id": "req-1", "results": results});
    let error = transform_response(&response, &fixed_id).unwrap_err();
    assert!(matches!(
        error,
        TogetherRerankError::MissingResultFields { .. }
    ));
    assert!(
        error
            .to_string()
            .starts_with("Missing required fields in the result=")
    );
}

#[test]
fn an_unrepresentable_score_is_refused() {
    let response = serde_json::from_str::<Value>(
        r#"{"id": "req-1", "results": [{"index": 0, "relevance_score": 1e400}]}"#,
    )
    .unwrap();
    let error = transform_response(&response, &fixed_id).unwrap_err();
    assert!(matches!(error, TogetherRerankError::Score { .. }));
}

#[rstest]
#[case::absent(json!({"index": 0, "relevance_score": 0.5}), None)]
#[case::null(json!({"index": 0, "relevance_score": 0.5, "document": null}), None)]
#[case::empty(json!({"index": 0, "relevance_score": 0.5, "document": {}}), None)]
#[case::text(json!({"index": 0, "relevance_score": 0.5, "document": {"text": "doc"}}), Some("doc"))]
#[case::no_text(json!({"index": 0, "relevance_score": 0.5, "document": {"other": 1}}), Some(""))]
fn a_document_is_kept_only_when_it_has_fields(
    #[case] result: Value,
    #[case] expected: Option<&str>,
) {
    let response = json!({"id": "req-1", "results": [result]});
    let reply = serde_json::to_value(transform_response(&response, &fixed_id).unwrap()).unwrap();
    assert_eq!(
        reply["results"][0].get("document"),
        expected.map(|text| json!({"text": text})).as_ref()
    );
}

#[test]
fn a_non_object_document_is_refused() {
    let response = json!({
        "id": "req-1",
        "results": [{"index": 0, "relevance_score": 0.5, "document": "doc"}],
    });
    let error = transform_response(&response, &fixed_id).unwrap_err();
    assert!(matches!(error, TogetherRerankError::InvalidDocument { .. }));
}

#[test]
fn an_empty_results_list_is_a_valid_reply() {
    let reply =
        serde_json::to_value(transform_response(&json!({"results": []}), &fixed_id).unwrap())
            .unwrap();
    assert_eq!(reply["results"], json!([]));
}
