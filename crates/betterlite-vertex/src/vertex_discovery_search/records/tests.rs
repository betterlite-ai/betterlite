#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use betterlite_models::catalog::BundledCatalog;
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreSearchResponse;
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::{Number, json};

use super::cost::calculate_vector_store_cost;
use super::create::{CreateStep, transform_create_store};
use super::payload::{PayloadError, decode_vertex_search_payload, transform_search_response};
use super::search::{SearchQuery, SearchRequest, transform_search_request};
use crate::gemini_response::decode::MAX_RESPONSE_BYTES;
use crate::vertex_discovery_search::error::VertexDiscoverySearchError;
use crate::vertex_discovery_search::options::VertexSearchOptions;
use crate::vertex_rag_store::records::payload::Reply;

fn object(text: &str) -> BTreeMap<String, Json> {
    match serde_json::from_str::<Json>(text).unwrap() {
        Json::Object(fields) => fields.into_iter().collect(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("a fixture object expected")
        }
    }
}

fn expected_json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn request<'fixture>(
    query: &'fixture SearchQuery,
    optional_params: &'fixture BTreeMap<String, Json>,
    extra_body: Option<&'fixture BTreeMap<String, Json>>,
    litellm_params: &'fixture BTreeMap<String, Json>,
) -> SearchRequest<'fixture> {
    SearchRequest {
        query,
        optional_params,
        extra_body,
        api_base: "https://host/v1",
        litellm_params,
    }
}

fn request_body(
    optional_params: &str,
    extra_body: Option<&str>,
    litellm_params: &str,
) -> (Json, Json) {
    let optional = object(optional_params);
    let extra = extra_body.map(object);
    let litellm = object(litellm_params);
    let query = SearchQuery::Text("hello".to_owned());
    let mut logging = BTreeMap::new();
    let call = transform_search_request(
        &request(&query, &optional, extra.as_ref(), &litellm),
        &mut logging,
    )
    .unwrap();
    (
        call.body,
        logging.get("query").cloned().unwrap_or(Json::Null),
    )
}

fn page(body: &str) -> VectorStoreSearchResponse {
    decode_vertex_search_payload(body.as_bytes(), "query").unwrap()
}

#[test]
fn a_search_request_posts_to_the_search_suffix_with_a_page_of_ten() {
    let litellm = object(r#"{"vector_store_id":"store"}"#);
    let optional = object("{}");
    let query = SearchQuery::Text("hello".to_owned());
    let mut logging = BTreeMap::new();
    let call = transform_search_request(&request(&query, &optional, None, &litellm), &mut logging)
        .unwrap();
    assert_eq!(call.url, "https://host/v1:search");
    assert_eq!(
        call.body,
        expected_json(r#"{"query": "hello", "pageSize": 10}"#)
    );
    assert_eq!(logging.get("query"), Some(&Json::Text("hello".to_owned())));
}

#[test]
fn a_query_sequence_is_joined_with_spaces() {
    let litellm = object("{}");
    let optional = object("{}");
    let query = SearchQuery::Texts(vec!["what".to_owned(), "is".to_owned()]);
    let mut logging = BTreeMap::new();
    let call = transform_search_request(&request(&query, &optional, None, &litellm), &mut logging)
        .unwrap();
    assert_eq!(
        call.body,
        expected_json(r#"{"query": "what is", "pageSize": 10}"#)
    );
    assert_eq!(
        logging.get("query"),
        Some(&Json::Text("what is".to_owned()))
    );
}

#[rstest]
#[case("{}", Some("default_collection"))]
#[case(r#"{"vertex_collection_id":null}"#, Some("default_collection"))]
#[case(
    r#"{"vertex_collection_id":"team_collection"}"#,
    Some("team_collection")
)]
fn the_search_options_default_the_collection_when_absent_or_null(
    #[case] params: &str,
    #[case] collection: Option<&str>,
) {
    let options = VertexSearchOptions::from_params(&object(params)).unwrap();
    assert_eq!(options.vertex_collection_id.as_deref(), collection);
}

#[rstest]
#[case("{}", 10)]
#[case(r#"{"max_num_results":null}"#, 10)]
#[case(r#"{"max_num_results":0}"#, 0)]
#[case(r#"{"max_num_results":25}"#, 25)]
fn a_max_num_results_replaces_the_page_size(#[case] optional: &str, #[case] page_size: i64) {
    let (body, _logged) = request_body(optional, None, "{}");
    let body = serde_json::to_value(body).unwrap();
    assert_eq!(body["pageSize"], json!(page_size));
}

#[rstest]
#[case(r#"{"max_num_results":2.5}"#)]
#[case(r#"{"max_num_results":"ten"}"#)]
#[case(r#"{"max_num_results":true}"#)]
fn a_non_integer_max_num_results_names_the_field(#[case] optional: &str) {
    let optional = object(optional);
    let litellm = object("{}");
    let query = SearchQuery::Text("hello".to_owned());
    let mut logging = BTreeMap::new();
    let error = transform_search_request(&request(&query, &optional, None, &litellm), &mut logging)
        .unwrap_err();
    assert!(matches!(
        error,
        VertexDiscoverySearchError::ParamType("max_num_results", _)
    ));
}

#[test]
fn allowed_extra_body_fields_override_the_query_and_page_size() {
    let (body, logged) = request_body(
        "{}",
        Some(r#"{"query":"native","pageSize":3,"filter":"owner=me"}"#),
        "{}",
    );
    assert_eq!(
        body,
        expected_json(r#"{"query": "native", "pageSize": 3, "filter": "owner=me"}"#)
    );
    assert_eq!(logged, Json::Text("native".to_owned()));
}

#[test]
fn null_extra_body_values_are_dropped_before_validation() {
    let (body, logged) = request_body(
        "{}",
        Some(r#"{"branch":null,"servingConfig":null,"dataStoreSpecs":null,"pageSize":null}"#),
        "{}",
    );
    assert_eq!(body, expected_json(r#"{"query": "hello", "pageSize": 10}"#));
    assert_eq!(logged, Json::Text("hello".to_owned()));
}

#[test]
fn target_selecting_fields_are_rejected_before_unsupported_ones() {
    let extra = object(r#"{"servingConfig":"apps/other","unsupported":1,"branch":"0"}"#);
    let optional = object("{}");
    let litellm = object("{}");
    let query = SearchQuery::Text("hello".to_owned());
    let mut logging = BTreeMap::new();
    let error = transform_search_request(
        &request(&query, &optional, Some(&extra), &litellm),
        &mut logging,
    )
    .unwrap_err();
    assert_eq!(error.status_code(), Some(400));
    let VertexDiscoverySearchError::TargetSelecting { fields } = error else {
        panic!("the target-selecting fields come first");
    };
    assert_eq!(fields, ["branch", "servingConfig"]);
}

#[test]
fn an_empty_target_selecting_value_is_still_rejected() {
    let extra = object(r#"{"entity":""}"#);
    let optional = object("{}");
    let litellm = object("{}");
    let query = SearchQuery::Text("hello".to_owned());
    let mut logging = BTreeMap::new();
    let error = transform_search_request(
        &request(&query, &optional, Some(&extra), &litellm),
        &mut logging,
    )
    .unwrap_err();
    let VertexDiscoverySearchError::TargetSelecting { fields } = error else {
        panic!("an empty entity still selects a target");
    };
    assert_eq!(fields, ["entity"]);
}

#[test]
fn a_datastore_rejection_lists_sorted_unsupported_fields() {
    let extra = object(r#"{"zzz":1,"dataStoreSpecs":[],"aaa":2}"#);
    let optional = object("{}");
    let litellm = object("{}");
    let query = SearchQuery::Text("hello".to_owned());
    let mut logging = BTreeMap::new();
    let error = transform_search_request(
        &request(&query, &optional, Some(&extra), &litellm),
        &mut logging,
    )
    .unwrap_err();
    assert_eq!(error.status_code(), Some(400));
    let VertexDiscoverySearchError::UnsupportedFields {
        mode,
        fields,
        supported,
    } = error
    else {
        panic!("only the unsupported fields are left");
    };
    assert_eq!(mode, "data store");
    assert_eq!(fields, ["aaa", "dataStoreSpecs", "zzz"]);
    assert!(!supported.iter().any(|field| field == "dataStoreSpecs"));
    let mut sorted = supported.clone();
    sorted.sort_unstable();
    assert_eq!(supported, sorted);
}

#[test]
fn an_engine_rejection_allows_the_store_fields_and_names_its_mode() {
    let extra = object(
        r#"{"zzz":1,"dataStoreSpecs":[{"dataStore":"projects/p"}],"numResultsPerDataStore":2}"#,
    );
    let optional = object("{}");
    let litellm = object(r#"{"vertex_engine_id":"app"}"#);
    let query = SearchQuery::Text("hello".to_owned());
    let mut logging = BTreeMap::new();
    let error = transform_search_request(
        &request(&query, &optional, Some(&extra), &litellm),
        &mut logging,
    )
    .unwrap_err();
    let VertexDiscoverySearchError::UnsupportedFields {
        mode,
        fields,
        supported,
    } = error
    else {
        panic!("only the unsupported fields are left");
    };
    assert_eq!(mode, "engine/app");
    assert_eq!(fields, ["zzz"]);
    assert!(supported.iter().any(|field| field == "dataStoreSpecs"));
}

#[test]
fn a_datastore_rejects_the_two_engine_only_fields() {
    let extra = object(r#"{"dataStoreSpecs":[],"numResultsPerDataStore":2}"#);
    let optional = object("{}");
    let litellm = object("{}");
    let query = SearchQuery::Text("hello".to_owned());
    let mut logging = BTreeMap::new();
    let error = transform_search_request(
        &request(&query, &optional, Some(&extra), &litellm),
        &mut logging,
    )
    .unwrap_err();
    let VertexDiscoverySearchError::UnsupportedFields { fields, .. } = error else {
        panic!("a data store cannot fan out across stores");
    };
    assert_eq!(fields, ["dataStoreSpecs", "numResultsPerDataStore"]);
}

#[test]
fn engine_mode_sends_the_multi_store_fields() {
    let (body, _logged) = request_body(
        "{}",
        Some(r#"{"dataStoreSpecs":[{"dataStore":"projects/p"}],"numResultsPerDataStore":2}"#),
        r#"{"vertex_engine_id":"app"}"#,
    );
    let body = serde_json::to_value(body).unwrap();
    assert_eq!(body["dataStoreSpecs"][0]["dataStore"], json!("projects/p"));
    assert_eq!(body["numResultsPerDataStore"], json!(2));
}

#[test]
fn a_mistyped_target_option_names_the_field() {
    let optional = object("{}");
    let litellm = object(r#"{"vertex_engine_id":5}"#);
    let query = SearchQuery::Text("hello".to_owned());
    let mut logging = BTreeMap::new();
    let error = transform_search_request(&request(&query, &optional, None, &litellm), &mut logging)
        .unwrap_err();
    assert!(matches!(error, VertexDiscoverySearchError::Options(_)));
    assert!(error.to_string().contains("vertex_engine_id"));
}

#[test]
fn a_chunk_hit_reports_its_content_and_resource_name() {
    let value = serde_json::to_value(page(
        r#"{"results":[{"chunk":{"id":"c1","name":"projects/p/locations/l/dataStores/s/branches/0/documents/d1/chunks/c1","content":"chunk text","documentMetadata":{"uri":"gs://b/p.pdf","title":"Guide","structData":{"author":"a"}},"pageSpan":{"pageStart":1,"pageEnd":2}}}]}"#,
    ))
    .unwrap();
    let hit = &value["data"][0];
    assert_eq!(hit["score"], json!(1));
    assert_eq!(hit["content"][0]["text"], json!("chunk text"));
    assert_eq!(hit["content"][0]["type"], json!("text"));
    assert_eq!(hit["file_id"], json!("gs://b/p.pdf"));
    assert_eq!(hit["filename"], json!("Guide"));
    assert_eq!(
        hit["attributes"],
        json!({
            "document_id": "d1",
            "chunk_id": "c1",
            "link": "gs://b/p.pdf",
            "title": "Guide",
            "structData": {"author": "a"},
            "pageSpan": {"pageStart": 1, "pageEnd": 2},
        })
    );
}

#[test]
fn an_empty_chunk_hit_wins_over_the_document() {
    let value = serde_json::to_value(page(
        r#"{"results":[{"chunk":{},"document":{"id":"doc1","derivedStructData":{"title":"Doc"}}}]}"#,
    ))
    .unwrap();
    let hit = &value["data"][0];
    assert_eq!(hit["content"][0]["text"], json!(""));
    assert_eq!(hit["file_id"], json!(""));
    assert_eq!(hit["filename"], json!("Unknown Document"));
    assert_eq!(hit["attributes"], json!({"document_id": ""}));
}

#[rstest]
#[case(
    "projects/p/locations/l/dataStores/s/branches/0/documents/d1/chunks/c1",
    "d1"
)]
#[case("chunks/c1", "")]
#[case("a/documents/b/c", "b")]
#[case("x/documents//y", "")]
#[case("", "")]
fn a_chunk_document_id_is_the_segment_after_documents(
    #[case] name: &str,
    #[case] document_id: &str,
) {
    let body = format!(r#"{{"results":[{{"chunk":{{"name":"{name}"}}}}]}}"#);
    let value = serde_json::to_value(page(&body)).unwrap();
    assert_eq!(
        value["data"][0]["attributes"]["document_id"],
        json!(document_id)
    );
}

#[rstest]
#[case(
    r#"{"chunk":{"name":"documents/d1","documentMetadata":{"uri":"gs://b/p.pdf","title":"Guide"}}}"#,
    "gs://b/p.pdf",
    "Guide"
)]
#[case(
    r#"{"chunk":{"name":"documents/d1","documentMetadata":{}}}"#,
    "",
    "Unknown Document"
)]
#[case(
    r#"{"chunk":{"name":"documents/d1","documentMetadata":{"uri":"","title":""}}}"#,
    "",
    "Unknown Document"
)]
#[case(
    r#"{"id":"doc1","document":{"derivedStructData":{"link":"https://x/doc","title":"Doc"}}}"#,
    "https://x/doc",
    "Doc"
)]
#[case(
    r#"{"id":"doc1","document":{"derivedStructData":{}}}"#,
    "doc1",
    "Unknown Document"
)]
#[case(r#"{"document":{}}"#, "", "Unknown Document")]
fn a_hit_takes_its_file_from_the_chunk_metadata_or_the_document_link(
    #[case] hit: &str,
    #[case] file_id: &str,
    #[case] filename: &str,
) {
    let body = format!(r#"{{"results":[{hit}]}}"#);
    let value = serde_json::to_value(page(&body)).unwrap();
    assert_eq!(value["data"][0]["file_id"], json!(file_id));
    assert_eq!(value["data"][0]["filename"], json!(filename));
}

#[rstest]
#[case(
    r#"{"extractive_segments":[{"content":"seg"}],"extractive_answers":[{"content":"ans"}],"snippets":[{"snippet":"snip"}],"title":"T"}"#,
    "seg"
)]
#[case(
    r#"{"extractive_segments":[{"content":""}],"extractive_answers":[{"content":"ans"}]}"#,
    "ans"
)]
#[case(
    r#"{"extractive_answers":[{"content":"a"},{"content":""},{"content":"b"}]}"#,
    "a\n\nb"
)]
#[case(r#"{"snippets":[{"snippet":"a"},{"htmlSnippet":"b"}]}"#, "a b")]
#[case(r#"{"snippets":[{"htmlSnippet":"H"}]}"#, "H")]
#[case(
    r#"{"snippets":[{"snippet":"","htmlSnippet":"H"}],"title":"Fallback"}"#,
    "Fallback"
)]
#[case(r#"{"snippets":[]}"#, "")]
#[case(r#"{"title":"T"}"#, "T")]
fn document_text_follows_the_segment_answer_snippet_title_order(
    #[case] derived: &str,
    #[case] text: &str,
) {
    let body = format!(r#"{{"results":[{{"document":{{"derivedStructData":{derived}}}}}]}}"#);
    let value = serde_json::to_value(page(&body)).unwrap();
    assert_eq!(value["data"][0]["content"][0]["text"], json!(text));
}

#[test]
fn a_document_keeps_its_id_and_drops_the_empty_attributes() {
    let value = serde_json::to_value(page(
        r#"{"results":[{"id":"doc1","document":{"structData":{"k":"v"},"derivedStructData":{"link":"https://x/doc","title":"Doc","displayLink":"x.com","formattedUrl":"https://x/fmt"}}}]}"#,
    ))
    .unwrap();
    assert_eq!(
        value["data"][0]["attributes"],
        json!({
            "document_id": "doc1",
            "link": "https://x/doc",
            "title": "Doc",
            "displayLink": "x.com",
            "formattedUrl": "https://x/fmt",
            "structData": {"k": "v"},
        })
    );
    let empty = serde_json::to_value(page(
        r#"{"results":[{"document":{"derivedStructData":{"link":"","title":""}}}]}"#,
    ))
    .unwrap();
    assert_eq!(empty["data"][0]["attributes"], json!({"document_id": ""}));
}

#[test]
fn a_null_chunk_takes_the_document_path() {
    let value = serde_json::to_value(page(
        r#"{"results":[{"chunk":null,"document":{"derivedStructData":{"title":"Doc"}}}]}"#,
    ))
    .unwrap();
    assert_eq!(value["data"][0]["filename"], json!("Doc"));
    assert_eq!(value["data"][0]["content"][0]["text"], json!("Doc"));
}

#[test]
fn positional_document_scores_are_exact_reciprocals() {
    let value = serde_json::to_value(page(r#"{"results":[{},{},{}]}"#)).unwrap();
    let scores = (0..3)
        .map(|index| value["data"][index]["score"].to_string())
        .collect::<Vec<_>>();
    assert_eq!(scores, ["1", "0.5", "0.3333333333333333333333333333"]);
}

#[rstest]
#[case(r#"{"chunk":{"relevanceScore":0}}"#, "0")]
#[case(r#"{"chunk":{"relevanceScore":0.25}}"#, "0.25")]
#[case(r#"{"chunk":{}}"#, "0.5")]
fn a_chunk_relevance_score_overrides_the_positional_score(
    #[case] second_hit: &str,
    #[case] score: &str,
) {
    let body = format!(r#"{{"results":[{{"chunk":{{"relevanceScore":1}}}},{second_hit}]}}"#);
    let value = serde_json::to_value(page(&body)).unwrap();
    assert_eq!(value["data"][1]["score"].to_string(), score);
}

#[test]
fn a_chunk_relevance_score_that_is_not_a_number_names_the_field() {
    let error = decode_vertex_search_payload(
        br#"{"results":[{"chunk":{"relevanceScore":"high"}}]}"#,
        "query",
    )
    .unwrap_err();
    assert!(
        matches!(error, PayloadError::Json(decode) if decode.to_string().contains("relevanceScore"))
    );
}

#[rstest]
#[case("{}")]
#[case(r#"{"results":null}"#)]
#[case(r#"{"results":[]}"#)]
#[case(r#"{"attributionToken":"t","results":[]}"#)]
fn a_reply_without_results_is_an_empty_page(#[case] body: &str) {
    let value = serde_json::to_value(page(body)).unwrap();
    assert_eq!(value["data"], json!([]));
    assert_eq!(value["object"], json!("vector_store.search_results.page"));
    assert_eq!(value["search_query"], json!("query"));
}

#[rstest]
#[case(b"[]")]
#[case(br#" "x""#)]
#[case(b"null")]
#[case(b"5")]
fn a_non_object_search_root_is_a_decode_error(#[case] body: &[u8]) {
    assert!(matches!(
        decode_vertex_search_payload(body, "query").unwrap_err(),
        PayloadError::Root
    ));
}

#[rstest]
#[case(br#"{"results":5}"#, "results")]
#[case(br#"{"results":[5]}"#, "results[0]")]
#[case(br#"{"results":[{"chunk":"x"}]}"#, "chunk")]
#[case(
    br#"{"results":[{"document":{"derivedStructData":{"title":5}}}]}"#,
    "title"
)]
#[case(
    br#"{"results":[{"chunk":{"documentMetadata":{"title":5}}}]}"#,
    "documentMetadata.title"
)]
fn a_malformed_search_reply_names_the_field(#[case] body: &[u8], #[case] field: &str) {
    let error = decode_vertex_search_payload(body, "query").unwrap_err();
    assert!(matches!(error, PayloadError::Json(decode) if decode.to_string().contains(field)));
}

#[test]
fn a_search_reply_that_is_not_json_is_a_decode_error() {
    assert!(matches!(
        decode_vertex_search_payload(b"{", "query").unwrap_err(),
        PayloadError::Json(_)
    ));
}

#[test]
fn the_search_payload_is_bounded_by_the_response_limit() {
    let mut exact = vec![b' '; MAX_RESPONSE_BYTES];
    exact[0] = b'{';
    assert!(matches!(
        decode_vertex_search_payload(&exact, "query").unwrap_err(),
        PayloadError::Json(_)
    ));
    let mut over = exact;
    over.push(b' ');
    assert!(matches!(
        decode_vertex_search_payload(&over, "query").unwrap_err(),
        PayloadError::TooLarge { size, limit }
            if size == MAX_RESPONSE_BYTES + 1 && limit == MAX_RESPONSE_BYTES
    ));
}

#[rstest]
#[case(BTreeMap::new(), "")]
#[case(
    BTreeMap::from([("query".to_owned(), Json::Text("logged".to_owned()))]),
    "logged"
)]
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
    let logging = BTreeMap::from([("query".to_owned(), Json::Number(Number::from(5_i64)))]);
    let headers = BTreeMap::new();
    let reply = Reply {
        status: 200,
        headers: &headers,
        body: b"{}",
    };
    let error = transform_search_response(&reply, &logging).unwrap_err();
    assert!(matches!(
        error,
        VertexDiscoverySearchError::ParamType("query", _)
    ));
}

#[test]
fn a_decode_failure_keeps_the_reply_status_and_headers() {
    let headers = BTreeMap::from([("x-goog-request-id".to_owned(), "abc".to_owned())]);
    let reply = Reply {
        status: 502,
        headers: &headers,
        body: b"[]",
    };
    let error = transform_search_response(&reply, &BTreeMap::new()).unwrap_err();
    let VertexDiscoverySearchError::Provider(provider) = error else {
        panic!("expected a provider error");
    };
    assert_eq!(provider.status_code, 502);
    assert_eq!(
        provider.headers.get("x-goog-request-id"),
        Some(&Json::Text("abc".to_owned()))
    );
    assert!(provider.message.contains("root is not a JSON object"));
}

#[test]
fn the_query_cost_comes_from_the_bundled_catalog() {
    let catalog = BundledCatalog::default().get();
    let prices = catalog
        .entries
        .get("vertex_ai/search_api")
        .map(|entry| &entry.prices);
    assert_eq!(
        calculate_vector_store_cost(prices),
        (Decimal::new(15, 4), Decimal::ZERO)
    );
}

#[test]
fn a_missing_catalog_entry_costs_nothing() {
    assert_eq!(
        calculate_vector_store_cost(None),
        (Decimal::ZERO, Decimal::ZERO)
    );
}

#[test]
fn creating_a_store_is_not_supported() {
    for step in [CreateStep::Request, CreateStep::Response] {
        let error = transform_create_store(step).err().unwrap();
        assert!(matches!(
            error,
            VertexDiscoverySearchError::CreateUnsupported
        ));
        assert_eq!(error.status_code(), Some(500));
    }
}

#[rstest]
fn replays_the_vertex_search_payload_corpus(
    #[files("fuzz/corpus/vertex_search_payload/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    let _page = decode_vertex_search_payload(&bytes, "query");
}
