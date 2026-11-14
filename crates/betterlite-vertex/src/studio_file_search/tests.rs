#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use betterlite_config::sections::studio_identity::{DEFAULT_API_BASE, Settings};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use jiff::tz::TimeZone;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;

use super::config::{capabilities, validate_environment};
use super::create::create_request;
use super::decode::{Decoded, RawResponse, Shape, decode_studio_file_search};
use super::error::StudioFileSearchError;
use super::request::{
    DEFAULT_MODEL, SearchInput, SearchPlan, SearchQuery, complete_url, search_request,
};
use super::{RequestParams, ResponseHeaders};
use crate::provider_metadata::timestamp::LocalTimeContext;
use crate::studio_identity::Headers;

const STORE: &str = "fileSearchStores/store-1";

fn parsed(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn as_json<T: Serialize>(value: &T) -> Json {
    parsed(&serde_json::to_string(value).unwrap())
}

fn settings(environment_key: Option<&str>, api_base: &str) -> Settings {
    Settings {
        environment_key: environment_key.map(SecretString::from),
        generic_api_key: None,
        api_base: api_base.to_owned(),
    }
}

fn params(api_key: Option<&str>, model: Option<&str>) -> RequestParams {
    RequestParams {
        api_key: api_key.map(SecretString::from),
        model: model.map(str::to_owned),
    }
}

fn exposed(headers: &Headers) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
        .collect()
}

fn search(
    query: SearchQuery,
    filters: Option<&Json>,
    max_num_results: Option<i64>,
    model: Option<&str>,
    api_key: Option<&str>,
) -> Result<SearchPlan, StudioFileSearchError> {
    let settings = Settings::default();
    let params = params(api_key, model);
    search_request(&SearchInput {
        vector_store_id: STORE,
        query: &query,
        filters,
        max_num_results,
        extra_body: None,
        params: &params,
        api_base: None,
        settings: &settings,
    })
}

fn text_query(query: &str) -> SearchQuery {
    SearchQuery::Text(query.to_owned())
}

fn context() -> LocalTimeContext {
    LocalTimeContext {
        zone: TimeZone::UTC,
    }
}

fn decode(
    shape: Shape,
    status: u16,
    body: &str,
    query: &str,
) -> Result<Decoded, StudioFileSearchError> {
    let headers = ResponseHeaders::new();
    let response = RawResponse {
        status,
        headers: &headers,
        body: body.as_bytes(),
    };
    decode_studio_file_search(shape, &response, query, &context())
}

fn field_of(value: &Json, field: &str) -> Json {
    let Json::Object(fields) = value else {
        panic!("the value is not an object");
    };
    fields.get(field).cloned().unwrap_or(Json::Null)
}

fn create_record(result: Decoded) -> Json {
    let Decoded::Create(record) = result else {
        panic!("the reply is not a create record");
    };
    as_json(&*record)
}

fn search_record(result: Decoded) -> Json {
    let Decoded::Search(record) = result else {
        panic!("the reply is not a search record");
    };
    as_json(&*record)
}

#[test]
fn capabilities_publish_the_search_and_create_endpoints() {
    let found = capabilities();
    assert_eq!(
        found.endpoints.read,
        Some(vec![(
            "POST".to_owned(),
            "/models/{model}:generateContent".to_owned()
        )])
    );
    assert_eq!(
        found.endpoints.write,
        Some(vec![("POST".to_owned(), "/fileSearchStores".to_owned())])
    );
    assert_eq!(found.supported_params, ["max_num_results", "filters"]);
    assert_eq!(found.credentials.headers, None);
    assert_eq!(found.credentials.query_params, None);
}

#[test]
fn environment_preserves_the_caller_content_type() {
    let caller = BTreeMap::from([("Content-Type".to_owned(), "application/custom".to_owned())]);
    let settings = settings(Some("env-key"), DEFAULT_API_BASE);
    let params = params(Some("explicit"), None);
    let headers = validate_environment(&caller, Some(&params), &settings);
    assert_eq!(
        exposed(&headers),
        BTreeMap::from([
            ("Content-Type".to_owned(), "application/custom".to_owned()),
            ("x-goog-api-key".to_owned(), "explicit".to_owned()),
        ])
    );
}

#[test]
fn environment_without_params_never_resolves_a_key() {
    let settings = settings(Some("env-key"), DEFAULT_API_BASE);
    let headers = validate_environment(&BTreeMap::new(), None, &settings);
    assert_eq!(
        exposed(&headers),
        BTreeMap::from([("Content-Type".to_owned(), "application/json".to_owned())])
    );
}

#[test]
fn environment_prefers_the_params_key_then_the_environment_key() {
    let settings = settings(Some("env-key"), DEFAULT_API_BASE);
    let caller = BTreeMap::new();
    let explicit = params(Some("explicit"), None);
    assert_eq!(
        exposed(&validate_environment(&caller, Some(&explicit), &settings))
            .get("x-goog-api-key")
            .map(String::as_str),
        Some("explicit")
    );
    let empty = params(Some(""), None);
    assert_eq!(
        exposed(&validate_environment(&caller, Some(&empty), &settings))
            .get("x-goog-api-key")
            .map(String::as_str),
        Some("env-key")
    );
}

#[test]
fn environment_leaves_the_key_header_absent_when_nothing_resolves() {
    let settings = settings(None, DEFAULT_API_BASE);
    let params = params(Some(""), None);
    let headers = validate_environment(&BTreeMap::new(), Some(&params), &settings);
    assert_eq!(exposed(&headers).get("x-goog-api-key"), None);
    assert_eq!(
        exposed(&headers).get("Content-Type").map(String::as_str),
        Some("application/json")
    );
}

#[test]
fn environment_overwrites_the_caller_key_header() {
    let caller = BTreeMap::from([("x-goog-api-key".to_owned(), "stale".to_owned())]);
    let settings = settings(Some("env-key"), DEFAULT_API_BASE);
    let params = params(None, None);
    let headers = validate_environment(&caller, Some(&params), &settings);
    assert_eq!(
        exposed(&headers).get("x-goog-api-key").map(String::as_str),
        Some("env-key")
    );
}

#[test]
fn url_appends_the_version_without_trimming() {
    let base = complete_url(Some("https://studio.example//"), &Settings::default());
    assert_eq!(base, "https://studio.example///v1beta");
}

#[test]
fn url_uses_the_configured_base_only_when_none() {
    let settings = settings(None, "https://configured.example");
    assert_eq!(
        complete_url(None, &settings),
        "https://configured.example/v1beta"
    );
    assert_eq!(complete_url(Some(""), &settings), "/v1beta");
    assert_eq!(
        complete_url(None, &Settings::default()),
        format!("{DEFAULT_API_BASE}/v1beta")
    );
}

#[test]
fn search_defaults_the_model_to_flash() {
    let plan = search(text_query("q"), None, None, None, Some("key")).unwrap();
    assert_eq!(
        plan.url,
        format!("{DEFAULT_API_BASE}/v1beta/models/{DEFAULT_MODEL}:generateContent")
    );
    let empty = search(text_query("q"), None, None, Some(""), Some("key")).unwrap();
    assert_eq!(empty.url, plan.url);
}

#[test]
fn search_strips_every_gemini_prefix_occurrence() {
    let plan = search(
        text_query("q"),
        None,
        None,
        Some("gemini/gemini/gemini-x"),
        Some("key"),
    )
    .unwrap();
    assert!(
        plan.url.ends_with("/models/gemini-x:generateContent"),
        "{}",
        plan.url
    );
}

#[test]
fn search_keeps_a_foreign_model_name_untouched() {
    let plan = search(
        text_query("q"),
        None,
        None,
        Some("openai/gemini-pro"),
        Some("key"),
    )
    .unwrap();
    assert!(
        plan.url
            .ends_with("/models/openai/gemini-pro:generateContent"),
        "{}",
        plan.url
    );
}

#[test]
fn search_builds_contents_and_the_native_tool_spelling() {
    let plan = search(text_query("alpha beta"), None, None, None, Some("key")).unwrap();
    assert_eq!(
        plan.body,
        parsed(
            r#"{"contents":[{"parts":[{"text":"alpha beta"}]}],"tools":[{"file_search":{"file_search_store_names":["fileSearchStores/store-1"]}}]}"#
        )
    );
}

#[test]
fn search_ignores_extra_body() {
    let extra = parsed(r#"{"retrievalConfig":{"numberOfResults":9}}"#);
    let settings = Settings::default();
    let params = params(Some("key"), None);
    let query = text_query("alpha beta");
    let plan = search_request(&SearchInput {
        vector_store_id: STORE,
        query: &query,
        filters: None,
        max_num_results: None,
        extra_body: Some(&extra),
        params: &params,
        api_base: None,
        settings: &settings,
    })
    .unwrap();
    assert_eq!(
        plan.body,
        parsed(
            r#"{"contents":[{"parts":[{"text":"alpha beta"}]}],"tools":[{"file_search":{"file_search_store_names":["fileSearchStores/store-1"]}}]}"#
        )
    );
}

#[test]
fn search_joins_query_lists_with_spaces() {
    let query = SearchQuery::Sequence(vec!["alpha".to_owned(), "beta".to_owned()]);
    let plan = search(query, None, None, None, Some("key")).unwrap();
    assert_eq!(plan.query, "alpha beta");
    assert_eq!(
        plan.body,
        parsed(
            r#"{"contents":[{"parts":[{"text":"alpha beta"}]}],"tools":[{"file_search":{"file_search_store_names":["fileSearchStores/store-1"]}}]}"#
        )
    );
}

#[test]
fn search_records_the_query_and_store() {
    let plan = search(text_query("seed query"), None, None, None, Some("key")).unwrap();
    assert_eq!(plan.query, "seed query");
    assert_eq!(plan.vector_store_id, STORE);
}

#[test]
fn search_requires_a_key_without_placing_it_in_the_url() {
    let error = search(text_query("q"), None, None, None, None).unwrap_err();
    assert!(matches!(error, StudioFileSearchError::MissingKey));
    let plan = search(text_query("q"), None, None, None, Some("secret-key")).unwrap();
    assert!(!plan.url.contains("secret-key"));
    let body = serde_json::to_string(&plan.body).unwrap();
    assert!(!body.contains("secret-key"));
}

#[test]
fn search_reads_the_environment_key() {
    let settings = settings(Some("env-key"), DEFAULT_API_BASE);
    let params = params(None, None);
    let query = text_query("q");
    let plan = search_request(&SearchInput {
        vector_store_id: STORE,
        query: &query,
        filters: None,
        max_num_results: None,
        extra_body: None,
        params: &params,
        api_base: None,
        settings: &settings,
    })
    .unwrap();
    assert!(
        plan.url
            .ends_with(&format!("/models/{DEFAULT_MODEL}:generateContent"))
    );
}

#[test]
fn search_keys_stay_isolated_between_contexts() {
    let first = settings(Some("first-key"), DEFAULT_API_BASE);
    let second = settings(None, DEFAULT_API_BASE);
    let explicit = params(Some("first-explicit"), None);
    let query = text_query("q");
    let first_plan = search_request(&SearchInput {
        vector_store_id: STORE,
        query: &query,
        filters: None,
        max_num_results: None,
        extra_body: None,
        params: &explicit,
        api_base: None,
        settings: &first,
    })
    .unwrap();
    assert!(
        first_plan
            .url
            .ends_with(&format!("/models/{DEFAULT_MODEL}:generateContent"))
    );
    let anonymous = params(None, None);
    let error = search_request(&SearchInput {
        vector_store_id: STORE,
        query: &query,
        filters: None,
        max_num_results: None,
        params: &anonymous,
        api_base: None,
        settings: &second,
    })
    .unwrap_err();
    assert!(matches!(error, StudioFileSearchError::MissingKey));
    let other = params(Some("second-explicit"), None);
    let second_plan = search_request(&SearchInput {
        vector_store_id: STORE,
        query: &query,
        filters: None,
        max_num_results: None,
        extra_body: None,
        params: &other,
        api_base: None,
        settings: &second,
    })
    .unwrap();
    assert!(
        second_plan
            .url
            .ends_with(&format!("/models/{DEFAULT_MODEL}:generateContent"))
    );
    let headers = validate_environment(&BTreeMap::new(), None, &second);
    assert_eq!(exposed(&headers).get("x-goog-api-key"), None);
}

fn metadata_filter_of(plan: &SearchPlan) -> Json {
    let Json::Object(body) = &plan.body else {
        panic!("the body is not an object");
    };
    let Some(Json::List(tools)) = body.get("tools") else {
        panic!("the body has no tools");
    };
    let Some(Json::Object(tool)) = tools.first() else {
        panic!("the tool is not an object");
    };
    let Some(Json::Object(search)) = tool.get("file_search") else {
        panic!("the tool has no file_search");
    };
    search.get("metadata_filter").cloned().unwrap_or(Json::Null)
}

#[rstest]
#[case::absent(None)]
#[case::null(Some(Json::Null))]
#[case::empty_object(Some(Json::Object(IndexMap::new())))]
#[case::empty_list(Some(Json::List(Vec::new())))]
#[case::zero(Some(Json::Number(serde_json::Number::from(0_u8))))]
#[case::false_flag(Some(Json::Bool(false)))]
#[case::empty_text(Some(Json::Text(String::new())))]
fn search_omits_metadata_filter_for_falsy_filters(#[case] filters: Option<Json>) {
    let plan = search(text_query("q"), filters.as_ref(), None, None, Some("key")).unwrap();
    assert_eq!(metadata_filter_of(&plan), Json::Null);
}

#[test]
fn search_quotes_string_filter_values_without_escaping() {
    let filters =
        parsed(r#"{"genre":"sci\"fi","year":2024,"score":1.5,"verified":true,"missing":null}"#);
    let plan = search(text_query("q"), Some(&filters), None, None, Some("key")).unwrap();
    assert_eq!(
        metadata_filter_of(&plan),
        Json::Text(
            "genre = \"sci\"fi\" AND year = 2024 AND score = 1.5 AND verified = True AND missing = None"
                .to_owned()
        )
    );
}

#[test]
fn search_spells_container_filter_values_as_standard_json() {
    let filters = parsed(r#"{"tags":["a","b"],"nested":{"one":1}}"#);
    let plan = search(text_query("q"), Some(&filters), None, None, Some("key")).unwrap();
    assert_eq!(
        metadata_filter_of(&plan),
        Json::Text("tags = [\"a\",\"b\"] AND nested = {\"one\":1}".to_owned())
    );
}

#[test]
fn search_passes_a_non_dict_filter_through() {
    let list = parsed(r#"["a","b"]"#);
    let plan = search(text_query("q"), Some(&list), None, None, Some("key")).unwrap();
    assert_eq!(
        metadata_filter_of(&plan),
        Json::List(vec![Json::Text("a".to_owned()), Json::Text("b".to_owned())])
    );
    let text = Json::Text("genre = drama".to_owned());
    let plan = search(text_query("q"), Some(&text), None, None, Some("key")).unwrap();
    assert_eq!(
        metadata_filter_of(&plan),
        Json::Text("genre = drama".to_owned())
    );
}

fn generation_config_of(plan: &SearchPlan) -> Option<&Json> {
    let Json::Object(body) = &plan.body else {
        return None;
    };
    body.get("generationConfig")
}

#[rstest]
#[case::absent(None)]
#[case::zero(Some(0))]
fn search_omits_generation_config_for_a_falsy_limit(#[case] limit: Option<i64>) {
    let plan = search(text_query("q"), None, limit, None, Some("key")).unwrap();
    assert_eq!(generation_config_of(&plan), None);
}

#[rstest]
#[case::one(1)]
#[case::hundred(100)]
#[case::negative(-2)]
fn search_sets_candidate_count_one_for_a_truthy_limit(#[case] limit: i64) {
    let plan = search(text_query("q"), None, Some(limit), None, Some("key")).unwrap();
    assert_eq!(
        generation_config_of(&plan),
        Some(&parsed(r#"{"candidateCount":1}"#))
    );
}

#[test]
fn create_forwards_only_a_truthy_name() {
    let named = create_request(Some("docs"), "https://h/v1beta");
    assert_eq!(named.url, "https://h/v1beta/fileSearchStores");
    assert_eq!(named.body, parsed(r#"{"displayName":"docs"}"#));
    assert_eq!(
        create_request(Some(""), "https://h/v1beta").body,
        parsed("{}")
    );
    assert_eq!(create_request(None, "https://h/v1beta").body, parsed("{}"));
}

#[test]
fn decode_create_keeps_the_complete_store_id() {
    let record = create_record(
        decode(
            Shape::Create,
            200,
            r#"{"name":"fileSearchStores/abc","displayName":"Docs","createTime":"2024-01-02T03:04:05Z"}"#,
            "",
        )
        .unwrap(),
    );
    assert_eq!(
        record,
        parsed(
            r#"{"id":"fileSearchStores/abc","object":"vector_store","created_at":1704164645,"name":"Docs","bytes":0,"file_counts":{"in_progress":0,"completed":0,"failed":0,"cancelled":0,"total":0},"status":"completed","expires_after":null,"expires_at":null,"last_active_at":null,"metadata":null}"#
        )
    );
}

#[test]
fn decode_create_converts_the_time_to_unix_seconds() {
    let zulu = create_record(
        decode(
            Shape::Create,
            200,
            r#"{"createTime":"2024-01-02T03:04:05Z"}"#,
            "",
        )
        .unwrap(),
    );
    assert_eq!(field_of(&zulu, "created_at"), parsed("1704164645"));
    let offset = create_record(
        decode(
            Shape::Create,
            200,
            r#"{"createTime":"2024-01-02T03:04:05.1+05:30"}"#,
            "",
        )
        .unwrap(),
    );
    assert_eq!(field_of(&offset, "created_at"), parsed("1704144845"));
}

#[rstest]
#[case::absent("{}")]
#[case::empty(r#"{"createTime":""}"#)]
#[case::invalid(r#"{"createTime":"not a time"}"#)]
fn decode_create_returns_zero_for_a_missing_or_invalid_time(#[case] body: &str) {
    let record = create_record(decode(Shape::Create, 200, body, "").unwrap());
    assert_eq!(field_of(&record, "created_at"), parsed("0"));
}

#[test]
fn decode_create_writes_explicit_nulls_for_expiry_and_metadata() {
    let record = create_record(decode(Shape::Create, 200, "{}", "").unwrap());
    assert_eq!(
        record,
        parsed(
            r#"{"id":"","object":"vector_store","created_at":0,"name":"","bytes":0,"file_counts":{"in_progress":0,"completed":0,"failed":0,"cancelled":0,"total":0},"status":"completed","expires_after":null,"expires_at":null,"last_active_at":null,"metadata":null}"#
        )
    );
}

#[rstest]
#[case::array("[]")]
#[case::text("\"store\"")]
fn decode_create_rejects_a_non_object_reply(#[case] body: &str) {
    let error = decode(Shape::Create, 400, body, "").unwrap_err();
    assert!(matches!(
        error,
        StudioFileSearchError::CreateReply { status: 400, .. }
    ));
}

#[test]
fn decode_create_rejects_a_wrong_typed_field() {
    let error = decode(Shape::Create, 200, r#"{"name":7}"#, "").unwrap_err();
    assert!(matches!(
        error,
        StudioFileSearchError::CreateReply { status: 200, .. }
    ));
}

#[test]
fn decode_create_wraps_invalid_json() {
    let error = decode(Shape::Create, 422, "{", "").unwrap_err();
    match error {
        StudioFileSearchError::CreateReply { status, reason, .. } => {
            assert_eq!(status, 422);
            assert!(reason.contains("line 1"), "{reason}");
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn decode_search_appends_retrieved_contexts_across_candidates() {
    let body = r#"{"candidates":[
        {"groundingMetadata":{"groundingChunks":[{"retrievedContext":{"text":"first","uri":"files/a","title":"a.txt"}}]}},
        {"groundingMetadata":{"groundingChunks":[{"retrievedContext":{"text":"second","uri":"files/b","title":"b.txt"}}]}}
    ]}"#;
    let record = search_record(decode(Shape::Search, 200, body, "seed query").unwrap());
    assert_eq!(
        record,
        parsed(
            r#"{"object":"vector_store.search_results.page","search_query":"seed query","data":[
                {"score":null,"content":[{"text":"first","type":"text"}],"file_id":"files/a","filename":"a.txt","attributes":{"uri":"files/a","title":"a.txt"}},
                {"score":null,"content":[{"text":"second","type":"text"}],"file_id":"files/b","filename":"b.txt","attributes":{"uri":"files/b","title":"b.txt"}}
            ]}"#
        )
    );
}

#[test]
fn decode_search_keeps_a_truthy_context_with_empty_text() {
    let body = r#"{"candidates":[{"groundingMetadata":{"groundingChunks":[
        {"retrievedContext":{"text":""}}
    ]}}]}"#;
    let record = search_record(decode(Shape::Search, 200, body, "").unwrap());
    assert_eq!(
        record,
        parsed(
            r#"{"object":"vector_store.search_results.page","search_query":"","data":[
                {"score":null,"content":[{"text":"","type":"text"}],"file_id":null,"filename":null,"attributes":{"uri":"","title":""}}
            ]}"#
        )
    );
}

#[rstest]
#[case::uri_only(
    r#"{"candidates":[{"groundingMetadata":{"groundingChunks":[{"retrievedContext":{"uri":"files/only"}}]}}]}"#,
    r#"{"object":"vector_store.search_results.page","search_query":"","data":[{"score":null,"content":[{"text":"","type":"text"}],"file_id":"files/only","filename":null,"attributes":{"uri":"files/only","title":""}}]}"#
)]
#[case::title_only(
    r#"{"candidates":[{"groundingMetadata":{"groundingChunks":[{"retrievedContext":{"title":"only.txt"}}]}}]}"#,
    r#"{"object":"vector_store.search_results.page","search_query":"","data":[{"score":null,"content":[{"text":"","type":"text"}],"file_id":null,"filename":"only.txt","attributes":{"uri":"","title":"only.txt"}}]}"#
)]
fn decode_search_keeps_a_context_with_only_one_identifier(
    #[case] body: &str,
    #[case] expected: &str,
) {
    let record = search_record(decode(Shape::Search, 200, body, "").unwrap());
    assert_eq!(record, parsed(expected));
}

#[rstest]
#[case::empty_object(
    r#"{"candidates":[{"groundingMetadata":{"groundingChunks":[{"retrievedContext":{}}]}}]}"#
)]
#[case::null_context(
    r#"{"candidates":[{"groundingMetadata":{"groundingChunks":[{"retrievedContext":null},{}]}}]}"#
)]
fn decode_search_skips_a_falsy_retrieved_context(#[case] body: &str) {
    let record = search_record(decode(Shape::Search, 200, body, "").unwrap());
    assert_eq!(field_of(&record, "data"), parsed("[]"));
}

#[test]
fn decode_search_uses_the_first_confidence_score() {
    let body = r#"{"candidates":[{"groundingMetadata":{"groundingSupports":[{"segment":{"text":"answer"},"groundingChunkIndices":[0,2],"confidenceScores":[0.25,0.75]}]}}]}"#;
    let record = search_record(decode(Shape::Search, 200, body, "").unwrap());
    assert_eq!(
        record,
        parsed(
            r#"{"object":"vector_store.search_results.page","search_query":"","data":[
                {"score":0.25,"content":[{"text":"answer","type":"text"}],"attributes":{"grounding_chunk_indices":[0,2]}}
            ]}"#
        )
    );
}

#[test]
fn decode_search_omits_file_metadata_for_supports() {
    let body = r#"{"candidates":[{"groundingMetadata":{"groundingSupports":[{"segment":{"text":"answer"}}]}}]}"#;
    let record = search_record(decode(Shape::Search, 200, body, "").unwrap());
    assert_eq!(
        record,
        parsed(
            r#"{"object":"vector_store.search_results.page","search_query":"","data":[
                {"score":null,"content":[{"text":"answer","type":"text"}],"attributes":{"grounding_chunk_indices":[]}}
            ]}"#
        )
    );
}

#[test]
fn decode_search_dedupes_support_text_across_candidates() {
    let body = r#"{"candidates":[
        {"groundingMetadata":{
            "groundingChunks":[{"retrievedContext":{"text":"shared"}}],
            "groundingSupports":[{"segment":{"text":"other"}}]
        }},
        {"groundingMetadata":{
            "groundingSupports":[{"segment":{"text":"shared"}},{"segment":{"text":"other"}}]
        }}
    ]}"#;
    let record = search_record(decode(Shape::Search, 200, body, "").unwrap());
    assert_eq!(
        record,
        parsed(
            r#"{"object":"vector_store.search_results.page","search_query":"","data":[
                {"score":null,"content":[{"text":"shared","type":"text"}],"file_id":null,"filename":null,"attributes":{"uri":"","title":""}},
                {"score":null,"content":[{"text":"other","type":"text"}],"attributes":{"grounding_chunk_indices":[]}}
            ]}"#
        )
    );
}

#[test]
fn decode_search_skips_supports_without_text() {
    let body = r#"{"candidates":[{"groundingMetadata":{"groundingSupports":[
        {"segment":{"text":""}},
        {"segment":{}},
        {}
    ]}}]}"#;
    let record = search_record(decode(Shape::Search, 200, body, "").unwrap());
    assert_eq!(field_of(&record, "data"), parsed("[]"));
}

#[test]
fn decode_search_handles_an_empty_object() {
    let record = search_record(decode(Shape::Search, 200, "{}", "").unwrap());
    assert_eq!(
        record,
        parsed(r#"{"object":"vector_store.search_results.page","search_query":"","data":[]}"#)
    );
}

#[test]
fn decode_search_skips_a_null_grounding_metadata() {
    let body = r#"{"candidates":[{"groundingMetadata":null},{"groundingMetadata":{}}]}"#;
    let record = search_record(decode(Shape::Search, 200, body, "").unwrap());
    assert_eq!(field_of(&record, "data"), parsed("[]"));
}

#[rstest]
#[case::candidates_object(r#"{"candidates":{}}"#)]
#[case::candidate_text(r#"{"candidates":["x"]}"#)]
#[case::chunks_object(r#"{"candidates":[{"groundingMetadata":{"groundingChunks":{}}}]}"#)]
#[case::context_text(
    r#"{"candidates":[{"groundingMetadata":{"groundingChunks":[{"retrievedContext":"x"}]}}]}"#
)]
#[case::supports_text(r#"{"candidates":[{"groundingMetadata":{"groundingSupports":["x"]}}]}"#)]
#[case::segment_text(
    r#"{"candidates":[{"groundingMetadata":{"groundingSupports":[{"segment":"x"}]}}]}"#
)]
fn decode_search_rejects_a_malformed_shape(#[case] body: &str) {
    let error = decode(Shape::Search, 200, body, "").unwrap_err();
    assert!(matches!(
        error,
        StudioFileSearchError::SearchReply { status: 200, .. }
    ));
}

#[test]
fn decode_search_rejects_a_wrong_typed_text_field() {
    let body = r#"{"candidates":[{"groundingMetadata":{"groundingChunks":[{"retrievedContext":{"text":7}}]}}]}"#;
    let error = decode(Shape::Search, 200, body, "").unwrap_err();
    assert!(matches!(
        error,
        StudioFileSearchError::SearchReply { status: 200, .. }
    ));
}

#[test]
fn decode_search_rejects_a_non_number_confidence_score() {
    let body = r#"{"candidates":[{"groundingMetadata":{"groundingSupports":[{"segment":{"text":"a"},"confidenceScores":["high"]}]}}]}"#;
    let error = decode(Shape::Search, 200, body, "").unwrap_err();
    assert!(matches!(
        error,
        StudioFileSearchError::SearchReply { status: 200, .. }
    ));
}

#[test]
fn decode_search_wraps_invalid_json() {
    let error = decode(Shape::Search, 500, "not json", "").unwrap_err();
    match error {
        StudioFileSearchError::SearchReply { status, reason, .. } => {
            assert_eq!(status, 500);
            assert!(reason.contains("line 1"), "{reason}");
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn decode_reports_the_reply_status_and_headers() {
    let headers = ResponseHeaders::from([("x-request-id".to_owned(), "abc".to_owned())]);
    let response = RawResponse {
        status: 429,
        headers: &headers,
        body: b"{",
    };
    let error = decode_studio_file_search(Shape::Search, &response, "", &context()).unwrap_err();
    match error {
        StudioFileSearchError::SearchReply {
            status,
            headers: carried,
            ..
        } => {
            assert_eq!(status, 429);
            assert_eq!(carried, headers);
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn decode_error_display_names_the_operation() {
    let create = decode(Shape::Create, 200, "[]", "").unwrap_err();
    assert!(
        create
            .to_string()
            .starts_with("Failed to parse Gemini create response: ")
    );
    let search = decode(Shape::Search, 200, "[]", "").unwrap_err();
    assert!(
        search
            .to_string()
            .starts_with("Failed to parse Gemini response: ")
    );
    assert_eq!(
        StudioFileSearchError::MissingKey.to_string(),
        "GEMINI_API_KEY or GOOGLE_API_KEY is required"
    );
}

#[rstest]
fn replays_the_studio_file_search_corpus(
    #[files("fuzz/corpus/studio_file_search/*")] path: PathBuf,
) {
    let bytes = fs::read(&path).unwrap();
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("utf-8 name");
    let headers = ResponseHeaders::new();
    let response = RawResponse {
        status: 200,
        headers: &headers,
        body: &bytes,
    };
    let search = decode_studio_file_search(Shape::Search, &response, "", &context());
    let create = decode_studio_file_search(Shape::Create, &response, "", &context());
    if name.starts_with("valid_search") {
        assert!(search.is_ok(), "{name}");
    }
    if name.starts_with("invalid_search") {
        assert!(search.is_err(), "{name}");
    }
    if name.starts_with("valid_create") {
        assert!(create.is_ok(), "{name}");
    }
    if name.starts_with("invalid_create") {
        assert!(create.is_err(), "{name}");
    }
    if name.starts_with("invalid_both") {
        assert!(search.is_err(), "{name}");
        assert!(create.is_err(), "{name}");
    }
}
