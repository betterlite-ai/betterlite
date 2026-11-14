#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::future::ready;
use std::path::PathBuf;

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_config::sections::vertex_credential_sources::vertex_settings::AdcEnvironment;
use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::api::rerank_contracts::records::RerankDocument;
use betterlite_openai::api::vector_store_records::records::score::Score;
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;
use uuid::Uuid;

use crate::credential_sources::getters::VertexParams;
use crate::gemini_response::decode::MAX_RESPONSE_BYTES;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;
use crate::vertex_rerank::error::VertexRerankError;
use crate::vertex_rerank::request::{RankRequest, map_rerank_params, transform_request};
use crate::vertex_rerank::response::{RankedRecord, decode_vertex_rerank, transform_response};
use crate::vertex_rerank::target::{complete_url, request_headers};

const MODEL: &str = "semantic-ranker-512@latest";

fn params(source: &str) -> Extra {
    serde_json::from_str(source).unwrap()
}

fn request_of(source: &str) -> RankRequest {
    transform_request(MODEL, &params(source), None).unwrap()
}

fn access(token: &str, project: &str) -> AccessToken {
    AccessToken {
        token: SecretString::from(token.to_owned()),
        project: project.to_owned(),
    }
}

const fn decimal(mantissa: i64, scale: u32) -> Score {
    Score(Decimal::new(mantissa, scale))
}

fn request_with(count: usize) -> RankRequest {
    let mut fields = Extra::new();
    fields.insert(String::from("query"), Json::Text(String::from("q")));
    fields.insert(
        String::from("documents"),
        Json::List(vec![Json::Text(String::from("a")); count]),
    );
    transform_request(MODEL, &fields, None).unwrap()
}

#[test]
fn records_are_numbered_from_zero_and_a_string_titles_itself_by_three_words() {
    let request = request_of(r#"{"query":"q","documents":["  one two\tthree four","solo"]}"#);
    assert_eq!(
        serde_json::to_string(&request).unwrap(),
        concat!(
            r#"{"model":"semantic-ranker-512@latest","query":"q","records":["#,
            r#"{"id":"0","title":"one two three","content":"  one two\tthree four"},"#,
            r#"{"id":"1","title":"solo","content":"solo"}],"ignoreRecordDetailsInResponse":false}"#,
        )
    );
}

#[test]
fn python_separator_controls_split_a_title_like_whitespace() {
    let request = request_of("{\"query\":\"q\",\"documents\":[\"a\\u001cb\\u001fc d\"]}");
    assert_eq!(request.records[0].title, "a b c");
}

#[test]
fn a_mapping_prefers_its_text_and_title() {
    let request = request_of(
        r#"{"query":"q","documents":[{"text":"alpha beta gamma delta","title":"Own","extra":1},{"text":"alpha beta gamma delta"}]}"#,
    );
    assert_eq!(request.records[0].title, "Own");
    assert_eq!(request.records[0].content, "alpha beta gamma delta");
    assert_eq!(request.records[1].title, "alpha beta gamma");
}

#[test]
fn a_mapping_without_text_uses_the_python_mapping_text() {
    let request = request_of(
        r#"{"query":"q","documents":[{"body":"it's","n":1,"ok":true,"none":null,"list":[1]},{"title":"T","k":"v"}]}"#,
    );
    let records = &request.records;
    assert_eq!(
        records[0].content,
        r#"{'body': "it's", 'n': 1, 'ok': True, 'none': None, 'list': [1]}"#
    );
    assert_eq!(records[0].title, r#"{'body': "it's", 'n':"#);
    assert_eq!(records[1].content, "{'title': 'T', 'k': 'v'}");
    assert_eq!(records[1].title, "T");
}

#[test]
fn a_null_text_or_title_counts_as_absent() {
    let request = request_of(r#"{"query":"q","documents":[{"text":null,"title":null,"a":1}]}"#);
    assert_eq!(
        request.records[0].content,
        "{'text': None, 'title': None, 'a': 1}"
    );
    assert_eq!(request.records[0].title, "{'text': None, 'title':");
}

#[rstest]
#[case::zero(r#""top_n":0"#, Some(0))]
#[case::three(r#""top_n":3"#, Some(3))]
#[case::null(r#""top_n":null"#, None)]
#[case::absent(r#""other":1"#, None)]
fn top_n_is_emitted_for_every_non_null_value(#[case] field: &str, #[case] expected: Option<i64>) {
    let request = request_of(&format!(r#"{{"query":"q","documents":[],{field}}}"#));
    assert_eq!(request.top_n, expected);
    let wire = serde_json::to_string(&request).unwrap();
    assert_eq!(wire.contains("topN"), expected.is_some());
}

#[rstest]
#[case::absent("", false)]
#[case::null(r#","return_documents":null"#, false)]
#[case::true_value(r#","return_documents":true"#, false)]
#[case::false_value(r#","return_documents":false"#, true)]
fn return_documents_defaults_to_true_and_is_inverted(#[case] field: &str, #[case] ignored: bool) {
    let request = request_of(&format!(r#"{{"query":"q","documents":[]{field}}}"#));
    assert_eq!(request.ignore_record_details, ignored);
}

#[test]
fn an_empty_query_and_an_empty_document_list_are_still_requests() {
    let request = request_of(r#"{"query":"","documents":[]}"#);
    assert_eq!(request.query, "");
    assert!(request.records.is_empty());
}

#[rstest]
#[case::no_query(r#"{"documents":[]}"#, "query is required for Vertex AI rerank")]
#[case::null_query(
    r#"{"query":null,"documents":[]}"#,
    "query is required for Vertex AI rerank"
)]
#[case::no_documents(r#"{"query":"q"}"#, "documents is required for Vertex AI rerank")]
#[case::query_type(r#"{"query":1,"documents":[]}"#, "the query value must be text")]
#[case::documents_type(
    r#"{"query":"q","documents":"x"}"#,
    "the documents value must be a list"
)]
#[case::top_n_type(
    r#"{"query":"q","documents":[],"top_n":1.5}"#,
    "the top_n value must be a whole number"
)]
#[case::return_type(
    r#"{"query":"q","documents":[],"return_documents":1}"#,
    "the return_documents value must be true or false"
)]
#[case::document_type(
    r#"{"query":"q","documents":["a",3]}"#,
    "the document at index 1 must be text or an object"
)]
#[case::text_type(
    r#"{"query":"q","documents":[{"text":3}]}"#,
    "the text of the document at index 0 must be text"
)]
#[case::title_type(
    r#"{"query":"q","documents":["a",{"text":"a","title":[]}]}"#,
    "the title of the document at index 1 must be text"
)]
fn a_request_with_a_missing_or_mistyped_field_names_it(
    #[case] source: &str,
    #[case] message: &str,
) {
    let error = transform_request(MODEL, &params(source), None).unwrap_err();
    assert_eq!(error.to_string(), message);
}

#[test]
fn labels_come_from_the_request_metadata_only_when_nonempty() {
    let fields = params(r#"{"query":"q","documents":[]}"#);
    let named: Json =
        serde_json::from_str(r#"{"metadata":{"requester_metadata":{"team":"a","n":1}}}"#).unwrap();
    let labelled = transform_request(MODEL, &fields, Some(&named)).unwrap();
    assert_eq!(
        labelled.user_labels,
        Some(BTreeMap::from([(String::from("team"), String::from("a"))]))
    );
    let empty: Json = serde_json::from_str(r#"{"metadata":{"requester_metadata":{}}}"#).unwrap();
    let plain = transform_request(MODEL, &fields, Some(&empty)).unwrap();
    assert_eq!(plain.user_labels, None);
    assert!(
        !serde_json::to_string(&plain)
            .unwrap()
            .contains("userLabels")
    );
}

#[test]
fn the_mapper_returns_the_four_fields_then_every_non_default_param() {
    let extra = params(r#"{"top_n":9,"model":"m"}"#);
    let documents = [RerankDocument::Text(String::from("d"))];
    let mapped = map_rerank_params(&extra, "q", &documents, Some(2), None);
    let names: Vec<&str> = mapped.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        ["query", "documents", "top_n", "return_documents", "model"]
    );
    assert_eq!(mapped["top_n"], Json::Number(Number::from(9_i64)));
    assert_eq!(mapped["return_documents"], Json::Null);
    assert_eq!(
        mapped["documents"],
        Json::List(vec![Json::Text(String::from("d"))])
    );
}

#[tokio::test]
async fn the_url_is_the_global_ranking_config_of_the_resolved_project() {
    let credentials = ConfiguredCredentials::Text(SecretString::from("{}".to_owned()));
    let vertex = VertexSettings::default();
    let call = VertexParams {
        project: Some("param-project"),
        ai_project: None,
        location: None,
        ai_location: None,
        credentials: Some(&credentials),
        ai_credentials: None,
    };
    let url = complete_url(call, &vertex, |found, project| {
        assert_eq!(found.and_then(ConfiguredCredentials::as_text), Some("{}"));
        assert_eq!(project, Some("param-project"));
        ready(Ok(access("tok", "resolved")))
    })
    .await
    .unwrap();
    assert_eq!(
        url,
        "https://discoveryengine.googleapis.com/v1/projects/resolved/locations/global/rankingConfigs/default_ranking_config:rank"
    );
}

#[rstest]
#[case::configured(Some("configured"), "", Some("configured"))]
#[case::empty_setting(Some(""), "", None)]
#[case::no_setting(None, "", None)]
#[case::token_wins(Some("configured"), "resolved", Some("resolved"))]
#[tokio::test]
async fn an_empty_resolved_project_falls_back_to_the_configured_one(
    #[case] configured: Option<&str>,
    #[case] resolved: &str,
    #[case] expected: Option<&str>,
) {
    let vertex = VertexSettings {
        project: configured.map(str::to_owned),
        location: None,
        credentials: None,
        adc: AdcEnvironment::default(),
    };
    let outcome = complete_url(VertexParams::default(), &vertex, |_, _| {
        ready(Ok(access("tok", resolved)))
    })
    .await;
    match expected {
        Some(project) => assert!(
            outcome
                .unwrap()
                .contains(&format!("/projects/{project}/locations/global/"))
        ),
        None => assert!(matches!(outcome, Err(VertexRerankError::MissingProject))),
    }
}

#[tokio::test]
async fn a_failed_credential_lookup_stops_both_the_url_and_the_headers() {
    let vertex = VertexSettings::default();
    let url = complete_url(VertexParams::default(), &vertex, |_, _| {
        ready(Err(CredentialCacheError::MissingToken))
    })
    .await;
    assert!(matches!(url, Err(VertexRerankError::Credentials(_))));
    let none = BTreeMap::new();
    let headers = request_headers(VertexParams::default(), &vertex, &none, |_, _| {
        ready(Err(CredentialCacheError::MissingToken))
    })
    .await;
    assert!(matches!(headers, Err(VertexRerankError::Credentials(_))));
}

#[tokio::test]
async fn headers_carry_bearer_json_and_the_billing_project() {
    let vertex = VertexSettings::default();
    let none = BTreeMap::new();
    let headers = request_headers(VertexParams::default(), &vertex, &none, |_, _| {
        ready(Ok(access("tok", "proj")))
    })
    .await
    .unwrap();
    let shown: BTreeMap<&str, &str> = headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.expose_secret()))
        .collect();
    assert_eq!(
        shown,
        BTreeMap::from([
            ("Authorization", "Bearer tok"),
            ("Content-Type", "application/json"),
            ("X-Goog-User-Project", "proj"),
        ])
    );
}

#[tokio::test]
async fn every_caller_header_overrides_the_defaults() {
    let vertex = VertexSettings::default();
    let caller = BTreeMap::from([
        (String::from("Authorization"), String::from("Bearer mine")),
        (String::from("Content-Type"), String::from("text/plain")),
        (String::from("X-Goog-User-Project"), String::from("other")),
        (String::from("X-Extra"), String::from("1")),
    ]);
    let headers = request_headers(VertexParams::default(), &vertex, &caller, |_, _| {
        ready(Ok(access("tok", "proj")))
    })
    .await
    .unwrap();
    let shown: BTreeMap<String, String> = headers
        .iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
        .collect();
    assert_eq!(shown, caller);
}

#[test]
fn records_without_a_score_rank_as_exactly_one_and_ids_parse_as_integers() {
    let bytes = br#"{"records":[{"id":"2","score":0.25},{"id":" 7 "},{"id":3,"score":null},{"id":"-1","score":1e-1}]}"#;
    let records = decode_vertex_rerank(bytes).unwrap();
    let indexes: Vec<i64> = records.iter().map(|record| record.index).collect();
    assert_eq!(indexes, [2, 7, 3, -1]);
    assert_eq!(records[0].score, decimal(25, 2));
    assert_eq!(records[1].score, decimal(1, 0));
    assert_eq!(records[2].score, decimal(1, 0));
    assert_eq!(records[3].score, decimal(1, 1));
}

#[test]
fn an_absent_or_null_record_list_is_an_empty_answer() {
    assert!(decode_vertex_rerank(b"{}").unwrap().is_empty());
    assert!(
        decode_vertex_rerank(br#"{"records":null,"x":1}"#)
            .unwrap()
            .is_empty()
    );
}

#[rstest]
#[case::json(br#"{"records":"#, "Failed to parse response: ")]
#[case::trailing(b"{} x", "Failed to parse response: ")]
#[case::list_root(b"[]", "the rerank response must be a JSON object")]
#[case::text_root(b"\"records\"", "the rerank response must be a JSON object")]
#[case::records_type(br#"{"records":{}}"#, "the records of the response must be a list")]
#[case::record_type(
    br#"{"records":[{"id":"1"},[]]}"#,
    "the record at position 1 must be an object"
)]
#[case::no_id(
    br#"{"records":[{"score":1}]}"#,
    "the record at position 0 has no integer id"
)]
#[case::word_id(
    br#"{"records":[{"id":"a"}]}"#,
    "the record at position 0 has no integer id"
)]
#[case::float_id(
    br#"{"records":[{"id":1.5}]}"#,
    "the record at position 0 has no integer id"
)]
#[case::null_id(
    br#"{"records":[{"id":null}]}"#,
    "the record at position 0 has no integer id"
)]
#[case::score_type(
    br#"{"records":[{"id":"1"},{"id":"2","score":"0.5"}]}"#,
    "the score of the record at position 1 must be a number"
)]
#[case::score_range(
    br#"{"records":[{"id":"1","score":1e30}]}"#,
    "the score of the record at position 0 is out of range"
)]
fn a_malformed_answer_names_the_problem(#[case] bytes: &[u8], #[case] message: &str) {
    let error = decode_vertex_rerank(bytes).unwrap_err();
    assert!(error.to_string().starts_with(message), "{error}");
}

#[test]
fn an_answer_over_the_byte_limit_is_refused_before_it_is_parsed() {
    let mut bytes = vec![b' '; MAX_RESPONSE_BYTES];
    assert!(matches!(
        decode_vertex_rerank(&bytes),
        Err(VertexRerankError::Json(_))
    ));
    bytes.push(b' ');
    assert!(matches!(
        decode_vertex_rerank(&bytes),
        Err(VertexRerankError::TooLarge { size }) if size == bytes.len()
    ));
}

#[test]
fn an_answer_nested_past_the_parser_depth_is_an_error_not_a_crash() {
    let depth = 4096;
    let text = format!(
        r#"{{"records":{}{}}}"#,
        "[".repeat(depth),
        "]".repeat(depth)
    );
    assert!(matches!(
        decode_vertex_rerank(text.as_bytes()),
        Err(VertexRerankError::Json(_))
    ));
}

#[test]
fn results_sort_by_descending_score_and_ties_keep_the_provider_order() {
    let bytes = br#"{"records":[{"id":"4"},{"id":"1","score":0.5},{"id":"2","score":1.0},{"id":"3","score":0.5},{"id":"0","score":2},{"id":"5","score":0.9}]}"#;
    let records = decode_vertex_rerank(bytes).unwrap();
    let response = transform_response(records, &request_with(6), Uuid::nil());
    let results = response.results.unwrap();
    let order: Vec<i64> = results.iter().map(|result| result.index).collect();
    assert_eq!(order, [0, 4, 2, 5, 1, 3]);
    assert!(results.iter().all(|result| result.document.is_none()));
    assert_eq!(results[3].relevance_score, decimal(9, 1));
}

#[test]
fn the_response_id_embeds_the_injected_uuid() {
    let id = Uuid::from_u128(0x1234);
    let response = transform_response(Vec::new(), &request_with(0), id);
    assert_eq!(
        response.id.as_deref(),
        Some("vertex_ai_rerank_00000000-0000-0000-0000-000000001234")
    );
    assert_eq!(response.results, Some(Vec::new()));
}

#[rstest]
#[case(0, 0)]
#[case(1, 1)]
#[case(99, 1)]
#[case(100, 1)]
#[case(101, 2)]
#[case(200, 2)]
#[case(201, 3)]
fn search_units_are_the_submitted_records_over_one_hundred_rounded_up(
    #[case] submitted: usize,
    #[case] units: i64,
) {
    let returned = vec![RankedRecord {
        index: 0,
        score: decimal(1, 0),
    }];
    let response = transform_response(returned, &request_with(submitted), Uuid::nil());
    let meta = response.meta.unwrap();
    let billed = meta.billed_units.unwrap().unwrap();
    assert_eq!(billed.search_units, Some(Some(units)));
    assert_eq!(billed.total_tokens, None);
    assert!(meta.api_version.is_none() && meta.tokens.is_none());
}

#[rstest]
fn replays_the_vertex_rerank_corpus(#[files("fuzz/corpus/vertex_rerank/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    if let Ok(records) = decode_vertex_rerank(&bytes) {
        drop(transform_response(records, &request_with(1), Uuid::nil()));
    }
}
