#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::{
    VectorStoreCreateResponse, VectorStoreSearchResponse,
};
use jiff::tz::TimeZone;
use rstest::rstest;
use serde_json::{Number, json};

use super::create::transform_create_request;
use super::payload::{
    PayloadError, Reply, decode_vertex_rag_corpus, decode_vertex_rag_payload,
    transform_create_response, transform_search_response,
};
use super::search::{SearchQuery, SearchRequest, transform_search_request};
use crate::credential_sources::getters::VertexParams;
use crate::model_urls::error::VertexUrlError;
use crate::provider_metadata::timestamp::LocalTimeContext;
use crate::vertex_rag_store::error::VertexRagStoreError;

fn settings() -> VertexSettings {
    VertexSettings::default()
}

fn params<'text>(project: Option<&'text str>, location: Option<&'text str>) -> VertexParams<'text> {
    VertexParams {
        project,
        ai_project: None,
        location,
        ai_location: None,
        credentials: None,
        ai_credentials: None,
    }
}

fn utc() -> LocalTimeContext {
    LocalTimeContext {
        zone: TimeZone::UTC,
    }
}

fn optional(text: &str) -> BTreeMap<String, Json> {
    match serde_json::from_str::<Json>(text).unwrap() {
        Json::Object(fields) => fields.into_iter().collect(),
        _ => panic!("optional parameters are an object"),
    }
}

fn search_response(body: &str) -> VectorStoreSearchResponse {
    decode_vertex_rag_payload(body.as_bytes(), "query").unwrap()
}

fn corpus_response(body: &str) -> VectorStoreCreateResponse {
    decode_vertex_rag_corpus(body.as_bytes(), &utc()).unwrap()
}

#[test]
fn a_full_corpus_query_joins_the_text_and_logs_it() {
    let settings = settings();
    let mut logging = BTreeMap::new();
    let optional_params = optional("{}");
    let query = SearchQuery::Texts(vec!["what".to_owned(), "is".to_owned(), "it".to_owned()]);
    let call = transform_search_request(SearchRequest {
        vector_store_id: "projects/p/locations/eu/ragCorpora/corpus",
        query: &query,
        optional_params: &optional_params,
        api_base: "https://host/v1/projects/p/locations/eu",
        logging: &mut logging,
        params: params(None, None),
        settings: &settings,
    })
    .unwrap();
    assert_eq!(
        call.url,
        "https://host/v1/projects/p/locations/eu:retrieveContexts"
    );
    let body = serde_json::to_value(&call.body).unwrap();
    assert_eq!(
        body["vertex_rag_store"]["rag_resources"][0]["rag_corpus"],
        json!("projects/p/locations/eu/ragCorpora/corpus")
    );
    assert_eq!(body["query"], json!({"text": "what is it"}));
    assert_eq!(
        logging.get("query"),
        Some(&Json::Text("what is it".to_owned()))
    );
}

#[rstest]
#[case(Some("proj"), Some("us-central1"), None, None)]
#[case(None, None, Some("proj"), Some("us-central1"))]
fn a_bare_corpus_id_expands_under_the_configured_project_and_location(
    #[case] project: Option<&str>,
    #[case] location: Option<&str>,
    #[case] setting_project: Option<&str>,
    #[case] setting_location: Option<&str>,
) {
    let settings = VertexSettings {
        project: setting_project.map(str::to_owned),
        location: setting_location.map(str::to_owned),
        ..VertexSettings::default()
    };
    let mut logging = BTreeMap::new();
    let optional_params = optional("{}");
    let query = SearchQuery::Text("hello".to_owned());
    let call = transform_search_request(SearchRequest {
        vector_store_id: "corpus",
        query: &query,
        optional_params: &optional_params,
        api_base: "https://host",
        logging: &mut logging,
        params: params(project, location),
        settings: &settings,
    })
    .unwrap();
    let body = serde_json::to_value(&call.body).unwrap();
    assert_eq!(
        body["vertex_rag_store"]["rag_resources"][0]["rag_corpus"],
        json!("projects/proj/locations/us-central1/ragCorpora/corpus")
    );
}

#[rstest]
#[case("corpus", None, Some("us-central1"), VertexUrlError::ProjectRequired)]
#[case("corpus", Some("proj"), None, VertexUrlError::LocationRequired)]
fn a_corpus_path_names_the_missing_setting(
    #[case] store: &str,
    #[case] project: Option<&str>,
    #[case] location: Option<&str>,
    #[case] expected: VertexUrlError,
) {
    let settings = settings();
    let mut logging = BTreeMap::new();
    let optional_params = optional("{}");
    let query = SearchQuery::Text("hello".to_owned());
    let error = transform_search_request(SearchRequest {
        vector_store_id: store,
        query: &query,
        optional_params: &optional_params,
        api_base: "https://host",
        logging: &mut logging,
        params: params(project, location),
        settings: &settings,
    })
    .unwrap_err();
    match error {
        VertexRagStoreError::Url(url) => assert_eq!(url.to_string(), expected.to_string()),
        other => panic!("expected a vertex URL error, got {other:?}"),
    }
}

#[test]
fn retrieval_options_map_onto_the_vertex_names() {
    let settings = settings();
    let mut logging = BTreeMap::new();
    let optional_params =
        optional(r#"{"max_num_results":0,"filters":{"kind":"x"},"ranking_options":{"rank":"y"}}"#);
    let query = SearchQuery::Text("hello".to_owned());
    let call = transform_search_request(SearchRequest {
        vector_store_id: "projects/p/locations/l/ragCorpora/c",
        query: &query,
        optional_params: &optional_params,
        api_base: "https://host",
        logging: &mut logging,
        params: params(None, None),
        settings: &settings,
    })
    .unwrap();
    let body = serde_json::to_value(&call.body).unwrap();
    assert_eq!(
        body["query"]["rag_retrieval_config"],
        json!({"top_k": 0, "filter": {"kind": "x"}, "ranking": {"rank": "y"}})
    );
}

#[rstest]
#[case("{}", r#"{"text":"hello"}"#)]
#[case(r#"{"rewrite_query":true}"#, r#"{"text":"hello"}"#)]
#[case(r#"{"max_num_results":null}"#, r#"{"text":"hello"}"#)]
#[case(
    r#"{"max_num_results":2}"#,
    r#"{"text":"hello","rag_retrieval_config":{"top_k":2}}"#
)]
fn only_a_retrieval_option_takes_the_config_into_the_body(
    #[case] text: &str,
    #[case] expected: &str,
) {
    let settings = settings();
    let mut logging = BTreeMap::new();
    let optional_params = optional(text);
    let query = SearchQuery::Text("hello".to_owned());
    let call = transform_search_request(SearchRequest {
        vector_store_id: "projects/p/locations/l/ragCorpora/c",
        query: &query,
        optional_params: &optional_params,
        api_base: "https://host",
        logging: &mut logging,
        params: params(None, None),
        settings: &settings,
    })
    .unwrap();
    let body = serde_json::to_value(&call.body).unwrap();
    assert_eq!(serde_json::to_string(&body["query"]).unwrap(), expected);
}

#[rstest]
#[case(r#"{"max_num_results":"five"}"#, "max_num_results")]
#[case(r#"{"max_num_results":2.5}"#, "max_num_results")]
#[case(r#"{"filters":"kind"}"#, "filters")]
#[case(r#"{"ranking_options":[1]}"#, "ranking_options")]
fn a_mistyped_retrieval_option_names_its_field(#[case] text: &str, #[case] field: &'static str) {
    let settings = settings();
    let mut logging = BTreeMap::new();
    let optional_params = optional(text);
    let query = SearchQuery::Text("hello".to_owned());
    let error = transform_search_request(SearchRequest {
        vector_store_id: "projects/p/locations/l/ragCorpora/c",
        query: &query,
        optional_params: &optional_params,
        api_base: "https://host",
        logging: &mut logging,
        params: params(None, None),
        settings: &settings,
    })
    .unwrap_err();
    assert!(matches!(error, VertexRagStoreError::ParamType(actual, _) if actual == field));
}

#[test]
fn nested_contexts_decode_into_one_result_each() {
    let value = serde_json::to_value(search_response(
        r#"{"contexts":{"contexts":[{"text":"chunk","sourceUri":"gs://b/f.pdf","sourceDisplayName":"f.pdf","score":0.5,"pageSpan":{"firstPage":3,"lastPage":4}}]}}"#,
    ))
    .unwrap();
    assert_eq!(value["object"], json!("vector_store.search_results.page"));
    assert_eq!(value["search_query"], json!("query"));
    let result = &value["data"][0];
    assert_eq!(result["score"], json!(0.5));
    assert_eq!(
        result["content"],
        json!([{"text": "chunk", "type": "text"}])
    );
    assert_eq!(result["file_id"], json!("gs://b/f.pdf"));
    assert_eq!(result["filename"], json!("f.pdf"));
    assert_eq!(
        result["attributes"],
        json!({
            "sourceUri": "gs://b/f.pdf",
            "sourceDisplayName": "f.pdf",
            "pageSpan": {"firstPage": 3, "lastPage": 4},
        })
    );
}

#[test]
fn provider_order_and_exact_scores_survive_decoding() {
    let response = search_response(
        r#"{"contexts":{"contexts":[{"text":"first","score":0.30000000000000004},{"text":"second","score":0.1234567890123456789012}]}}"#,
    );
    let value = serde_json::to_value(&response).unwrap();
    assert_eq!(value["data"][0]["content"][0]["text"], json!("first"));
    assert_eq!(value["data"][1]["content"][0]["text"], json!("second"));
    assert_eq!(value["data"][0]["score"], json!(0.30000000000000004));
    let text = serde_json::to_string(&response).unwrap();
    assert!(
        text.contains(r#""score":0.1234567890123456789012"#),
        "{text}"
    );
}

#[rstest]
#[case("{}", "", "Unknown Document")]
#[case(r#"{"sourceDisplayName":"doc"}"#, "doc", "doc")]
#[case(r#"{"sourceUri":""}"#, "", "Unknown Document")]
#[case(
    r#"{"sourceUri":"gs://x","sourceDisplayName":""}"#,
    "gs://x",
    "Unknown Document"
)]
fn a_result_names_its_source_with_the_display_name_fallback(
    #[case] context: &str,
    #[case] file_id: &str,
    #[case] filename: &str,
) {
    let body = format!(r#"{{"contexts":{{"contexts":[{context}]}}}}"#);
    let value = serde_json::to_value(search_response(&body)).unwrap();
    assert_eq!(value["data"][0]["file_id"], json!(file_id));
    assert_eq!(value["data"][0]["filename"], json!(filename));
}

#[test]
fn a_null_source_and_display_name_take_the_default() {
    let value = serde_json::to_value(search_response(
        r#"{"contexts":{"contexts":[{"sourceUri":null,"sourceDisplayName":null}]}}"#,
    ))
    .unwrap();
    assert_eq!(value["data"][0]["file_id"], json!(""));
    assert_eq!(value["data"][0]["filename"], json!("Unknown Document"));
    assert_eq!(value["data"][0]["attributes"], json!({}));
}

#[rstest]
#[case("{}", false)]
#[case(r#"{"pageSpan":{}}"#, false)]
#[case(r#"{"pageSpan":null}"#, false)]
#[case(r#"{"pageSpan":{"firstPage":1}}"#, true)]
fn only_a_truthy_page_span_reaches_the_attributes(#[case] context: &str, #[case] present: bool) {
    let body = format!(r#"{{"contexts":{{"contexts":[{context}]}}}}"#);
    let value = serde_json::to_value(search_response(&body)).unwrap();
    let attributes = value["data"][0]["attributes"].as_object().unwrap();
    assert_eq!(attributes.contains_key("pageSpan"), present);
}

#[rstest]
#[case("{}", "")]
#[case(r#"{"text":null}"#, "")]
#[case(r#"{"text":"chunk"}"#, "chunk")]
fn a_context_text_defaults_to_the_empty_string(#[case] context: &str, #[case] text: &str) {
    let body = format!(r#"{{"contexts":{{"contexts":[{context}]}}}}"#);
    let value = serde_json::to_value(search_response(&body)).unwrap();
    assert_eq!(value["data"][0]["content"][0]["text"], json!(text));
    assert_eq!(value["data"][0]["content"][0]["type"], json!("text"));
}

#[test]
fn a_context_text_of_another_type_names_the_field() {
    let error = decode_vertex_rag_payload(br#"{"contexts":{"contexts":[{"text":5}]}}"#, "query")
        .unwrap_err();
    assert!(matches!(error, PayloadError::Json(error) if error.to_string().contains("text")));
}

#[rstest]
#[case("{}", 0.0)]
#[case(r#"{"score":null}"#, 0.0)]
#[case(r#"{"score":0.5}"#, 0.5)]
fn a_context_score_defaults_to_zero(#[case] context: &str, #[case] score: f64) {
    let body = format!(r#"{{"contexts":{{"contexts":[{context}]}}}}"#);
    let value = serde_json::to_value(search_response(&body)).unwrap();
    assert_eq!(value["data"][0]["score"], json!(score));
}

#[test]
fn a_context_score_of_another_type_names_the_field() {
    let error =
        decode_vertex_rag_payload(br#"{"contexts":{"contexts":[{"score":"high"}]}}"#, "query")
            .unwrap_err();
    assert!(matches!(error, PayloadError::Json(error) if error.to_string().contains("score")));
}

#[test]
fn a_default_score_writes_a_decimal_point() {
    let text =
        serde_json::to_string(&search_response(r#"{"contexts":{"contexts":[{}]}}"#)).unwrap();
    assert!(text.contains(r#""score":0.0"#), "{text}");
}

#[rstest]
#[case("{}")]
#[case(r#"{"contexts":null}"#)]
#[case(r#"{"contexts":{}}"#)]
#[case(r#"{"contexts":{"contexts":[]}}"#)]
fn a_reply_without_contexts_is_an_empty_page(#[case] body: &str) {
    let value = serde_json::to_value(search_response(body)).unwrap();
    assert_eq!(value["data"], json!([]));
}

#[rstest]
#[case(b"[]", "document root")]
#[case(br#"{"contexts":"x"}"#, "contexts")]
#[case(br#"{"contexts":{"contexts":{}}}"#, "contexts.contexts")]
#[case(br#"{"contexts":{"contexts":[5]}}"#, "contexts.contexts[0]")]
fn a_malformed_search_reply_names_the_field(#[case] body: &[u8], #[case] field: &str) {
    let error = decode_vertex_rag_payload(body, "query").unwrap_err();
    assert!(matches!(error, PayloadError::Json(error) if error.to_string().contains(field)));
}

#[test]
fn a_search_reply_that_is_not_json_is_a_decode_error() {
    assert!(matches!(
        decode_vertex_rag_payload(b"{", "query").unwrap_err(),
        PayloadError::Json(_)
    ));
}

#[rstest]
#[case(BTreeMap::new(), "")]
#[case(BTreeMap::from([("query".to_owned(), Json::Text("logged".to_owned()))]), "logged")]
fn a_search_response_reads_the_logged_query(
    #[case] logging: BTreeMap<String, Json>,
    #[case] expected: &str,
) {
    let headers = BTreeMap::new();
    let reply = Reply {
        status: 200,
        headers: &headers,
        body: b"{}",
    };
    let response = transform_search_response(&reply, &logging).unwrap();
    assert_eq!(response.search_query.as_deref(), Some(expected));
}

#[test]
fn a_mistyped_logged_query_names_the_field() {
    let logging = BTreeMap::from([("query".to_owned(), Json::Number(Number::from(5)))]);
    let headers = BTreeMap::new();
    let reply = Reply {
        status: 200,
        headers: &headers,
        body: b"{}",
    };
    let error = transform_search_response(&reply, &logging).unwrap_err();
    assert!(matches!(error, VertexRagStoreError::ParamType("query", _)));
}

#[test]
fn a_decode_failure_keeps_the_reply_status_and_headers() {
    let headers = BTreeMap::from([("x-goog-request-id".to_owned(), "abc".to_owned())]);
    let reply = Reply {
        status: 502,
        headers: &headers,
        body: b"not json",
    };
    let error = transform_search_response(&reply, &BTreeMap::new()).unwrap_err();
    let VertexRagStoreError::Provider(provider) = error else {
        panic!("expected a provider error");
    };
    assert_eq!(provider.status_code, 502);
    assert_eq!(
        provider.headers.get("x-goog-request-id"),
        Some(&Json::Text("abc".to_owned()))
    );
    assert!(provider.message.contains("not valid JSON"));
}

#[test]
fn a_create_request_carries_the_default_corpus_name() {
    let call = transform_create_request(&optional("{}"), "https://host/v1/projects/p/locations/l")
        .unwrap();
    assert_eq!(
        call.url,
        "https://host/v1/projects/p/locations/l/ragCorpora"
    );
    assert_eq!(
        serde_json::to_value(&call.body).unwrap(),
        json!({
            "display_name": "litellm-vector-store",
            "description": "Vector store created via LiteLLM",
        })
    );
}

#[test]
fn a_create_request_takes_the_name_and_labels_from_the_params() {
    let call = transform_create_request(
        &optional(r#"{"name":"corpus","metadata":{"team":"a"}}"#),
        "https://host",
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&call.body).unwrap(),
        json!({
            "display_name": "corpus",
            "description": "Vector store created via LiteLLM",
            "labels": {"team": "a"},
        })
    );
}

#[test]
fn a_null_create_name_and_metadata_are_left_out() {
    let call = transform_create_request(
        &optional(r#"{"name":null,"metadata":null}"#),
        "https://host",
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&call.body).unwrap(),
        json!({"description": "Vector store created via LiteLLM"})
    );
}

#[test]
fn a_create_request_keeps_a_trailing_slash_of_the_complete_url() {
    let call = transform_create_request(&optional("{}"), "https://host/").unwrap();
    assert_eq!(call.url, "https://host//ragCorpora");
}

#[rstest]
#[case(r#"{"name":5}"#, "name")]
#[case(r#"{"metadata":"x"}"#, "metadata")]
#[case(r#"{"metadata":{"team":5}}"#, "metadata")]
fn a_mistyped_create_parameter_names_its_field(#[case] text: &str, #[case] field: &'static str) {
    let error = transform_create_request(&optional(text), "https://host").unwrap_err();
    assert!(matches!(error, VertexRagStoreError::ParamType(actual, _) if actual == field));
}

#[test]
fn a_corpus_reply_becomes_a_completed_vector_store() {
    let value = serde_json::to_value(corpus_response(
        r#"{"name":"projects/p/locations/l/ragCorpora/corpus","display_name":"store","createTime":"2024-01-01T00:00:00Z","labels":{"team":"a"}}"#,
    ))
    .unwrap();
    assert_eq!(value["id"], json!("corpus"));
    assert_eq!(value["object"], json!("vector_store"));
    assert_eq!(value["created_at"], json!(1_704_067_200));
    assert_eq!(value["name"], json!("store"));
    assert_eq!(value["bytes"], json!(0));
    assert_eq!(
        value["file_counts"],
        json!({"in_progress": 0, "completed": 0, "failed": 0, "cancelled": 0, "total": 0})
    );
    assert_eq!(value["status"], json!("completed"));
    assert_eq!(value["metadata"], json!({"team": "a"}));
    let fields = value.as_object().unwrap();
    for field in ["expires_after", "expires_at", "last_active_at"] {
        assert_eq!(fields.get(field), Some(&json!(null)), "{field}");
    }
}

#[rstest]
#[case("{}", 0)]
#[case(r#"{"name":"corpus"}"#, 0)]
#[case(r#"{"name":"a/b/"}"#, 0)]
#[case(r#"{"createTime":1704067200}"#, 1_704_067_200)]
#[case(r#"{"createTime":-5}"#, -5)]
#[case(r#"{"createTime":1704067200.0}"#, 0)]
#[case(r#"{"createTime":"2024-01-01T00:00:00Z"}"#, 1_704_067_200)]
#[case(r#"{"createTime":"2024-01-01T00:00:00+02:00"}"#, 1_704_060_000)]
#[case(r#"{"createTime":"not-a-time"}"#, 0)]
#[case(r#"{"createTime":""}"#, 0)]
#[case(r#"{"createTime":null}"#, 0)]
#[case(r#"{"createTime":true}"#, 1)]
#[case(r#"{"createTime":false}"#, 0)]
#[case(r#"{"createTime":[1]}"#, 0)]
#[case(r#"{"createTime":{"a":1}}"#, 0)]
fn a_create_time_takes_an_integer_an_iso_string_or_zero(#[case] body: &str, #[case] expected: i64) {
    let response = corpus_response(body);
    assert_eq!(response.created_at, Some(expected));
}

#[rstest]
#[case("{}", "{}")]
#[case(r#"{"labels":null}"#, "{}")]
#[case(r#"{"labels":"x"}"#, "{}")]
#[case(r#"{"labels":[1]}"#, "{}")]
#[case(r#"{"labels":{"a":"b"}}"#, r#"{"a":"b"}"#)]
fn only_a_labels_object_becomes_the_metadata(#[case] body: &str, #[case] expected: &str) {
    let value = serde_json::to_value(corpus_response(body)).unwrap();
    let expected: Json = serde_json::from_str(expected).unwrap();
    assert_eq!(value["metadata"], serde_json::to_value(expected).unwrap());
}

#[test]
fn a_label_value_that_is_not_a_string_is_a_decode_error() {
    let error = decode_vertex_rag_corpus(br#"{"labels":{"a":1}}"#, &utc()).unwrap_err();
    assert!(matches!(error, PayloadError::Field("labels", _)));
}

#[rstest]
#[case("{}", Json::Text(String::new()))]
#[case(r#"{"displayName":"camel"}"#, Json::Text(String::new()))]
#[case(r#"{"display_name":"snake"}"#, Json::Text("snake".to_owned()))]
#[case(r#"{"display_name":null}"#, Json::Text(String::new()))]
fn the_created_name_reads_only_the_snake_case_display_name(
    #[case] body: &str,
    #[case] expected: Json,
) {
    let value = serde_json::to_value(corpus_response(body)).unwrap();
    assert_eq!(value["name"], serde_json::to_value(expected).unwrap());
}

#[rstest]
#[case(r#"{"name":"corpus"}"#, "corpus")]
#[case(r#"{"name":"projects/p/locations/l/ragCorpora/corpus"}"#, "corpus")]
#[case(r#"{"name":"a/b/"}"#, "")]
#[case("{}", "")]
fn the_corpus_id_is_the_last_resource_name_segment(#[case] body: &str, #[case] expected: &str) {
    assert_eq!(corpus_response(body).id.as_deref(), Some(expected));
}

#[rstest]
#[case(b"[]", "document root")]
#[case(br#"{"name":5}"#, "name")]
#[case(br#"{"display_name":5}"#, "display_name")]
fn a_malformed_corpus_reply_names_the_field(#[case] body: &[u8], #[case] field: &str) {
    let error = decode_vertex_rag_corpus(body, &utc()).unwrap_err();
    assert!(matches!(error, PayloadError::Json(error) if error.to_string().contains(field)));
}

#[rstest]
fn replays_the_vertex_rag_payload_corpus(
    #[files("fuzz/corpus/vertex_rag_payload/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    let local = utc();
    let _search = decode_vertex_rag_payload(&bytes, "query");
    let _corpus = decode_vertex_rag_corpus(&bytes, &local);
}

#[test]
fn a_corpus_decode_failure_keeps_the_reply_status_and_headers() {
    let headers = BTreeMap::from([("retry-after".to_owned(), "3".to_owned())]);
    let reply = Reply {
        status: 429,
        headers: &headers,
        body: b"[]",
    };
    let error = transform_create_response(&reply, &utc()).unwrap_err();
    let VertexRagStoreError::Provider(provider) = error else {
        panic!("expected a provider error");
    };
    assert_eq!(provider.status_code, 429);
    assert_eq!(
        provider.headers.get("retry-after"),
        Some(&Json::Text("3".to_owned()))
    );
}
