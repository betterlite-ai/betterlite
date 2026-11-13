#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::cohere_rerank_settings::Settings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;

use super::{
    CohereRerankError, DEFAULT_V1_URL, DEFAULT_V2_URL, RerankParams, RerankVersion, complete_url,
    get_error_class, map_rerank_params, request_headers, transform_rerank_request,
    transform_rerank_response,
};
use crate::api::responses_shapes::wire::WireDecodeLimits;

fn limits() -> WireDecodeLimits {
    WireDecodeLimits {
        max_json_bytes: 0x0010_0000,
        max_event_bytes: 0x0004_0000,
        max_items: 10_000,
    }
}

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn settings(
    cohere_api_key: Option<&str>,
    co_api_key: Option<&str>,
    cohere_key: Option<&str>,
) -> Settings {
    Settings {
        cohere_api_key: cohere_api_key.map(secret),
        co_api_key: co_api_key.map(secret),
        cohere_key: cohere_key.map(secret),
    }
}

fn params(entries: Vec<(&str, Json)>) -> RerankParams {
    entries
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn number(value: i64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

fn documents() -> Json {
    Json::List(vec![
        text("first"),
        Json::Object(IndexMap::from([("title".to_owned(), text("t"))])),
    ])
}

fn headers(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn options(query: &str, documents: Vec<&str>) -> RerankParams {
    params(vec![
        ("query", text(query)),
        (
            "documents",
            Json::List(documents.into_iter().map(text).collect()),
        ),
    ])
}

#[rstest]
#[case::v1_default(RerankVersion::V1, None, DEFAULT_V1_URL)]
#[case::v2_default(RerankVersion::V2, None, DEFAULT_V2_URL)]
#[case::empty_base_takes_the_default(RerankVersion::V1, Some(""), DEFAULT_V1_URL)]
#[case::empty_v2_base_takes_the_default(RerankVersion::V2, Some(""), DEFAULT_V2_URL)]
#[case::one_trailing_slash(
    RerankVersion::V1,
    Some("https://api.test/"),
    "https://api.test/v1/rerank"
)]
#[case::many_trailing_slashes(
    RerankVersion::V2,
    Some("https://api.test///"),
    "https://api.test/v2/rerank"
)]
#[case::complete_v1_base(
    RerankVersion::V1,
    Some("https://api.test/v1/rerank"),
    "https://api.test/v1/rerank"
)]
#[case::complete_v2_base_with_slash(
    RerankVersion::V2,
    Some("https://api.test/v2/rerank/"),
    "https://api.test/v2/rerank"
)]
#[case::v1_appends_to_a_v2_base(
    RerankVersion::V1,
    Some("https://api.test/v2/rerank"),
    "https://api.test/v2/rerank/v1/rerank"
)]
#[case::v2_appends_to_a_v1_base(
    RerankVersion::V2,
    Some("https://api.test/v1/rerank"),
    "https://api.test/v1/rerank/v2/rerank"
)]
#[case::root_base(RerankVersion::V1, Some("/"), "/v1/rerank")]
fn complete_url_trims_the_base_and_appends_the_version_suffix(
    #[case] version: RerankVersion,
    #[case] api_base: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(complete_url(version, api_base), expected);
}

#[test]
fn mapping_v1_keeps_query_documents_and_the_chunk_limit() {
    let options = params(vec![
        ("query", text("why")),
        ("documents", Json::List(vec![text("first")])),
        ("top_n", number(2)),
        ("rank_fields", Json::List(vec![text("body")])),
        ("return_documents", Json::Bool(false)),
        ("max_chunks_per_doc", number(3)),
        ("max_tokens_per_doc", number(9)),
        ("instruction", text("rank these")),
    ]);
    let mapped = map_rerank_params(RerankVersion::V1, &options, &RerankParams::new());
    assert_eq!(
        mapped,
        params(vec![
            ("query", text("why")),
            ("documents", Json::List(vec![text("first")])),
            ("top_n", number(2)),
            ("rank_fields", Json::List(vec![text("body")])),
            ("return_documents", Json::Bool(false)),
            ("max_chunks_per_doc", number(3)),
        ])
    );
}

#[test]
fn mapping_v2_keeps_the_token_limit_instead() {
    let mut options = options("why", vec!["first"]);
    options.insert("max_chunks_per_doc".to_owned(), number(3));
    options.insert("max_tokens_per_doc".to_owned(), number(9));
    options.insert("instruction".to_owned(), text("rank these"));
    let mapped = map_rerank_params(RerankVersion::V2, &options, &RerankParams::new());
    assert_eq!(mapped.get("max_tokens_per_doc"), Some(&number(9)));
    assert!(!mapped.contains_key("max_chunks_per_doc"));
    assert!(!mapped.contains_key("instruction"));
    assert_eq!(mapped.get("return_documents"), Some(&Json::Bool(true)));
}

#[rstest]
#[case::absent(None, true)]
#[case::null(Some(Json::Null), true)]
#[case::explicit_true(Some(Json::Bool(true)), true)]
#[case::explicit_false(Some(Json::Bool(false)), false)]
fn mapping_defaults_return_documents_to_true(#[case] given: Option<Json>, #[case] expected: bool) {
    let mut options = options("why", Vec::new());
    if let Some(value) = given {
        options.insert("return_documents".to_owned(), value);
    }
    let mapped = map_rerank_params(RerankVersion::V1, &options, &RerankParams::new());
    assert_eq!(mapped.get("return_documents"), Some(&Json::Bool(expected)));
}

#[test]
fn mapping_retains_nullable_keys_and_the_projection_removes_them() {
    let options = options("why", vec!["first"]);
    let mapped = map_rerank_params(RerankVersion::V1, &options, &RerankParams::new());
    assert_eq!(mapped.get("top_n"), Some(&Json::Null));
    assert_eq!(mapped.get("rank_fields"), Some(&Json::Null));
    assert_eq!(mapped.get("max_chunks_per_doc"), Some(&Json::Null));

    let body = transform_rerank_request(RerankVersion::V1, "rerank-v3", &mapped)
        .expect("the mapped params project");
    assert_eq!(
        body,
        params(vec![
            ("model", text("rerank-v3")),
            ("query", text("why")),
            ("documents", Json::List(vec![text("first")])),
            ("return_documents", Json::Bool(true)),
        ])
    );
    assert!(!body.values().any(|value| matches!(value, Json::Null)));
}

#[test]
fn the_projection_keeps_every_declared_non_null_field() {
    let options = params(vec![
        ("query", text("why")),
        (
            "documents",
            Json::List(vec![
                text("first"),
                Json::Object(IndexMap::from([("title".to_owned(), text("t"))])),
            ]),
        ),
        ("top_n", number(0)),
        ("rank_fields", Json::List(vec![text("body")])),
        ("return_documents", Json::Bool(false)),
        ("max_chunks_per_doc", number(0)),
    ]);
    let mapped = map_rerank_params(RerankVersion::V1, &options, &RerankParams::new());
    let body = transform_rerank_request(RerankVersion::V1, "rerank-v3", &mapped)
        .expect("the mapped params project");
    assert_eq!(body.get("top_n"), Some(&number(0)));
    assert_eq!(body.get("max_chunks_per_doc"), Some(&number(0)));
    assert_eq!(body.get("return_documents"), Some(&Json::Bool(false)));
    assert_eq!(
        body.get("documents"),
        Some(&Json::List(vec![
            text("first"),
            Json::Object(IndexMap::from([("title".to_owned(), text("t"))])),
        ]))
    );
}

#[test]
fn the_projection_requires_query_by_key_presence() {
    let mapped = params(vec![("documents", documents())]);
    let error = transform_rerank_request(RerankVersion::V1, "rerank-v3", &mapped)
        .expect_err("a request without query is refused");
    assert!(matches!(
        error,
        CohereRerankError::Required { name: "query" }
    ));
}

#[test]
fn the_projection_requires_documents_by_key_presence() {
    let mapped = params(vec![("query", text("why"))]);
    let error = transform_rerank_request(RerankVersion::V1, "rerank-v3", &mapped)
        .expect_err("a request without documents is refused");
    assert!(matches!(
        error,
        CohereRerankError::Required { name: "documents" }
    ));
}

#[rstest]
#[case::null_query("query", Json::Null, "query")]
#[case::numeric_query("query", number(1), "query")]
#[case::scalar_documents("documents", text("first"), "documents")]
#[case::numeric_document("documents", Json::List(vec![number(1)]), "documents")]
#[case::scalar_rank_fields("rank_fields", text("body"), "rank_fields")]
#[case::numeric_rank_field("rank_fields", Json::List(vec![number(1)]), "rank_fields")]
#[case::scalar_return_documents("return_documents", text("yes"), "return_documents")]
#[case::scalar_top_n("top_n", text("2"), "top_n")]
fn the_projection_refuses_a_field_of_the_wrong_type(
    #[case] field: &str,
    #[case] value: Json,
    #[case] expected: &'static str,
) {
    let mut mapped = params(vec![("query", text("why")), ("documents", documents())]);
    mapped.insert(field.to_owned(), value);
    let error = transform_rerank_request(RerankVersion::V1, "rerank-v3", &mapped)
        .expect_err("the wrong type is refused");
    assert!(matches!(
        error,
        CohereRerankError::Field { name, .. } if name == expected
    ));
}

#[test]
fn each_version_reads_only_its_own_limit_from_the_params() {
    let shared = params(vec![
        ("query", text("why")),
        ("documents", documents()),
        ("max_chunks_per_doc", number(4)),
        ("max_tokens_per_doc", number(7)),
    ]);
    let v1 =
        transform_rerank_request(RerankVersion::V1, "m", &shared).expect("the v1 params project");
    assert_eq!(v1.get("max_chunks_per_doc"), Some(&number(4)));
    assert!(!v1.contains_key("max_tokens_per_doc"));
    let v2 =
        transform_rerank_request(RerankVersion::V2, "m", &shared).expect("the v2 params project");
    assert_eq!(v2.get("max_tokens_per_doc"), Some(&number(7)));
    assert!(!v2.contains_key("max_chunks_per_doc"));
}

#[test]
fn the_projection_ignores_instruction_and_unknown_params() {
    let mapped = params(vec![
        ("query", text("why")),
        ("documents", documents()),
        ("instruction", text("rank these")),
        ("extension", text("kept out")),
    ]);
    let body = transform_rerank_request(RerankVersion::V1, "m", &mapped)
        .expect("the unknown params are dropped");
    assert!(!body.contains_key("instruction"));
    assert!(!body.contains_key("extension"));
}

#[rstest]
#[case::long_alias_wins(None, "long", "short", "module", "long")]
#[case::short_alias_next(None, "", "short", "module", "short")]
#[case::module_key_last(None, "", "", "module", "module")]
#[case::empty_module_key_last(None, "", "", "", "")]
#[case::explicit_wins(Some("explicit"), "long", "short", "module", "explicit")]
#[case::explicit_empty_key(Some(""), "long", "short", "module", "")]
fn environment_resolves_the_key_chain(
    #[case] explicit: Option<&str>,
    #[case] long: &str,
    #[case] short: &str,
    #[case] module: &str,
    #[case] expected: &str,
) {
    let supplied = explicit.map(secret);
    let resolved = request_headers(
        &BTreeMap::new(),
        supplied.as_ref(),
        &settings(Some(long), Some(short), Some(module)),
    )
    .expect("the key chain resolves");
    let authorization = format!("Bearer {expected}");
    assert_eq!(resolved.get("Authorization"), Some(&authorization));
}

#[test]
fn environment_without_any_key_fails() {
    let error = request_headers(&BTreeMap::new(), None, &settings(None, None, None))
        .expect_err("a missing key is refused");
    assert!(matches!(error, CohereRerankError::MissingApiKey));
}

#[test]
fn environment_writes_the_cohere_defaults() {
    let resolved = request_headers(
        &BTreeMap::new(),
        Some(&secret("k")),
        &settings(None, None, None),
    )
    .expect("the defaults build");
    assert_eq!(
        resolved,
        headers(&[
            ("Authorization", "Bearer k"),
            ("accept", "application/json"),
            ("content-type", "application/json"),
        ])
    );
}

#[test]
fn caller_headers_replace_the_cohere_defaults_including_authorization() {
    let caller = headers(&[
        ("Authorization", "Bearer caller"),
        ("accept", "text/plain"),
        ("content-type", "text/plain"),
        ("x-trace", "abc"),
    ]);
    let resolved = request_headers(&caller, Some(&secret("k")), &settings(None, None, None))
        .expect("the caller headers merge");
    assert_eq!(
        resolved.get("Authorization").map(String::as_str),
        Some("Bearer caller")
    );
    assert_eq!(
        resolved.get("accept").map(String::as_str),
        Some("text/plain")
    );
    assert_eq!(
        resolved.get("content-type").map(String::as_str),
        Some("text/plain")
    );
    assert_eq!(resolved.get("x-trace").map(String::as_str), Some("abc"));
}

#[test]
fn a_lowercase_authorization_header_does_not_replace_the_default() {
    let caller = headers(&[("authorization", "Bearer caller")]);
    let resolved = request_headers(&caller, Some(&secret("k")), &settings(None, None, None))
        .expect("both header names survive");
    assert_eq!(
        resolved.get("Authorization").map(String::as_str),
        Some("Bearer k")
    );
    assert_eq!(
        resolved.get("authorization").map(String::as_str),
        Some("Bearer caller")
    );
}

#[test]
fn the_response_decoder_does_not_check_the_status() {
    let body = br#"{"results":[{"index":0,"relevance_score":0.5}]}"#;
    let response = transform_rerank_response(500, body, limits())
        .expect("the reply decodes despite the status");
    let results = response.results.expect("the results decode");
    assert_eq!(results.len(), 1);
    assert_eq!(results.first().map(|result| result.index), Some(0));
}

#[test]
fn a_response_that_is_not_json_becomes_the_cohere_error_with_its_status() {
    let error = transform_rerank_response(418, b"not json", limits())
        .expect_err("a non-JSON reply is refused");
    assert!(matches!(
        &error,
        CohereRerankError::ResponseJson {
            status: 418,
            message
        } if message == "not json"
    ));
}

#[test]
fn a_response_that_is_json_but_not_a_rerank_record_keeps_the_contract_error() {
    let error = transform_rerank_response(200, br#"{"results":"later"}"#, limits())
        .expect_err("a wrong shape is refused");
    assert!(matches!(error, CohereRerankError::Decode(_)));
}

#[test]
fn a_response_with_a_score_beyond_the_decimal_range_keeps_the_contract_status() {
    let error = transform_rerank_response(
        200,
        br#"{"results":[{"index":0,"relevance_score":1e400}]}"#,
        limits(),
    )
    .expect_err("the score is out of range");
    let CohereRerankError::Decode(source) = error else {
        panic!("the score error keeps the contract error");
    };
    assert_eq!(source.details().status, Some(502));
}

#[test]
fn the_error_class_drops_the_headers_and_keeps_status_and_message() {
    let dropped = headers(&[("retry-after", "2")]);
    let error = get_error_class("slow down".to_owned(), 429, &dropped);
    assert_eq!(error.status_code, 429);
    assert_eq!(error.message, "slow down");
    assert!(error.headers.is_none());
    assert!(!error.status_code_is_synthesized);
}

#[test]
fn the_plan_composes_the_v1_url_headers_and_body() {
    let mut options = options("why", vec!["first"]);
    options.insert("max_tokens_per_doc".to_owned(), number(7));
    options.insert("max_chunks_per_doc".to_owned(), number(4));
    let caller = headers(&[("x-trace", "abc")]);
    let mapped = map_rerank_params(RerankVersion::V1, &options, &RerankParams::new());
    let resolved = request_headers(&caller, Some(&secret("k")), &settings(None, None, None))
        .expect("the headers build");
    let body =
        transform_rerank_request(RerankVersion::V1, "rerank-v3", &mapped).expect("the body builds");
    assert_eq!(complete_url(RerankVersion::V1, None), DEFAULT_V1_URL);
    assert_eq!(
        resolved.get("Authorization").map(String::as_str),
        Some("Bearer k")
    );
    assert_eq!(resolved.get("x-trace").map(String::as_str), Some("abc"));
    assert_eq!(body.get("max_chunks_per_doc"), Some(&number(4)));
    assert!(!body.contains_key("max_tokens_per_doc"));
}

#[test]
fn the_plan_composes_the_v2_url_from_a_configured_base() {
    let mut options = options("why", Vec::new());
    options.insert("max_chunks_per_doc".to_owned(), number(4));
    let mapped = map_rerank_params(RerankVersion::V2, &options, &RerankParams::new());
    let resolved = request_headers(
        &BTreeMap::new(),
        None,
        &settings(None, Some("env-key"), None),
    )
    .expect("the environment key builds");
    let body =
        transform_rerank_request(RerankVersion::V2, "rerank-v4", &mapped).expect("the body builds");
    assert_eq!(
        complete_url(RerankVersion::V2, Some("https://gateway.test/cohere/")),
        "https://gateway.test/cohere/v2/rerank"
    );
    assert_eq!(
        resolved.get("Authorization").map(String::as_str),
        Some("Bearer env-key")
    );
    assert!(!body.contains_key("max_chunks_per_doc"));
    assert_eq!(body.get("return_documents"), Some(&Json::Bool(true)));
}
