#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::credentials::CredentialError;
use betterlite_openai::search::provider::{
    HttpMethod, OutboundSearch, ProviderResponse, SearchProvider, SearchQuery,
};
use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

use super::error::ParallelAiSearchError;
use super::reply::{ParallelAiReplyError, decode_parallel_ai_reply};
use super::request::{PARALLEL_AI_USAGE_PARAM, SearchBody};
use super::{PARALLEL_AI_API_BASE, ParallelAiSearch};

fn load(environment: &[(&str, &str)]) -> ParallelAiSearch {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ParallelAiSearch::load(&lookup)
}

fn headers(entries: &[(&str, &str)]) -> Headers {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn string_body(options: &str) -> SearchBody {
    load(&[])
        .transform_search_request(
            &SearchQuery::Single("rust".to_owned()),
            &serde_json::from_str::<Object>(options).unwrap(),
        )
        .unwrap()
}

#[test]
fn display_name_rich_input_and_method_match_the_source() {
    assert_eq!(
        <ParallelAiSearch as SearchProvider>::ui_friendly_name(),
        "Parallel AI"
    );
    let search = load(&[]);
    assert_eq!(search.http_method(), HttpMethod::Post);
    assert!(search.supports_rich_search_input());
}

#[rstest]
#[case::default(&[], None, PARALLEL_AI_API_BASE)]
#[case::ai_key(&[("PARALLEL_AI_API_KEY", "ai-key")], Some("ai-key"), PARALLEL_AI_API_BASE)]
#[case::legacy_key(&[("PARALLEL_API_KEY", "legacy-key")], Some("legacy-key"), PARALLEL_AI_API_BASE)]
#[case::ai_key_wins(
    &[("PARALLEL_AI_API_KEY", "ai-key"), ("PARALLEL_API_KEY", "legacy-key")],
    Some("ai-key"),
    PARALLEL_AI_API_BASE
)]
#[case::empty_ai_key_falls_through(
    &[("PARALLEL_AI_API_KEY", ""), ("PARALLEL_API_KEY", "legacy-key")],
    Some("legacy-key"),
    PARALLEL_AI_API_BASE
)]
#[case::base(
    &[("PARALLEL_AI_API_BASE", "https://proxy.test")],
    None,
    "https://proxy.test"
)]
#[case::empty_variables(
    &[
        ("PARALLEL_AI_API_KEY", ""),
        ("PARALLEL_API_KEY", ""),
        ("PARALLEL_AI_API_BASE", ""),
    ],
    None,
    PARALLEL_AI_API_BASE
)]
fn settings_load_from_the_environment(
    #[case] environment: &[(&str, &str)],
    #[case] api_key: Option<&str>,
    #[case] api_base: &str,
) {
    let search = load(environment);
    assert_eq!(
        search.api_key.as_ref().map(ExposeSecret::expose_secret),
        api_key
    );
    assert_eq!(search.api_base, api_base);
}

#[test]
fn headers_carry_the_key_and_content_type() {
    let search = load(&[("PARALLEL_AI_API_KEY", "ai-server")]);
    let prepared = search
        .prepare_headers(headers(&[("X-Trace", "1")]), None, None)
        .unwrap();
    assert_eq!(
        prepared,
        headers(&[
            ("Content-Type", "application/json"),
            ("X-Trace", "1"),
            ("x-api-key", "ai-server"),
        ])
    );
}

#[test]
fn missing_key_names_the_legacy_variable() {
    let error = load(&[])
        .prepare_headers(Headers::new(), None, None)
        .unwrap_err();
    assert!(matches!(error, ParallelAiSearchError::MissingApiKey));
    assert_eq!(
        error.to_string(),
        "PARALLEL_API_KEY is not set. Set `PARALLEL_API_KEY` environment variable."
    );
}

#[test]
fn caller_key_skips_the_trust_check() {
    let search = load(&[("PARALLEL_AI_API_KEY", "ai-server")]);
    let prepared = search
        .prepare_headers(
            Headers::new(),
            Some(&SecretString::from("ai-caller")),
            Some("https://evil.test/v1/search"),
        )
        .unwrap();
    assert_eq!(
        prepared.get("x-api-key").map(String::as_str),
        Some("ai-caller")
    );
}

#[test]
fn empty_caller_key_falls_back_to_the_server_key() {
    let search = load(&[("PARALLEL_API_KEY", "legacy-server")]);
    let prepared = search
        .prepare_headers(Headers::new(), Some(&SecretString::from("")), None)
        .unwrap();
    assert_eq!(
        prepared.get("x-api-key").map(String::as_str),
        Some("legacy-server")
    );
}

#[test]
fn server_key_refuses_an_untrusted_caller_base() {
    let search = load(&[("PARALLEL_AI_API_KEY", "ai-server")]);
    let error = search
        .prepare_headers(Headers::new(), None, Some("https://evil.test/v1/search"))
        .unwrap_err();
    assert!(matches!(
        error,
        ParallelAiSearchError::Credential(CredentialError::UntrustedBase { .. })
    ));
    assert!(!error.to_string().contains("ai-server"));
}

#[rstest]
#[case::default_host("https://api.parallel.ai/v1/search", &[], true)]
#[case::operator_override(
    "https://proxy.internal:9000/parallel",
    &[("PARALLEL_AI_API_BASE", "https://proxy.internal:9000")],
    true
)]
#[case::other_host("https://api.parallel.ai.evil.test/v1/search", &[], false)]
#[case::other_port("https://api.parallel.ai:8443/v1/search", &[], false)]
fn server_key_trust_follows_the_configured_hosts(
    #[case] api_base: &str,
    #[case] environment: &[(&str, &str)],
    #[case] trusted: bool,
) {
    let mut settings = vec![("PARALLEL_AI_API_KEY", "ai-server")];
    settings.extend_from_slice(environment);
    let prepared = load(&settings).prepare_headers(Headers::new(), None, Some(api_base));
    assert_eq!(prepared.is_ok(), trusted, "{prepared:?}");
}

#[rstest]
#[case::default_base(None, &[], "https://api.parallel.ai/v1/search")]
#[case::caller_base(Some("https://proxy.test"), &[], "https://proxy.test/v1/search")]
#[case::trailing_slash(Some("https://proxy.test/"), &[], "https://proxy.test/v1/search")]
#[case::v1_suffix(Some("https://proxy.test/v1"), &[], "https://proxy.test/v1/search")]
#[case::v1_slash_suffix(Some("https://proxy.test/v1/"), &[], "https://proxy.test/v1/search")]
#[case::search_suffix(
    Some("https://proxy.test/v1/search"),
    &[],
    "https://proxy.test/v1/search"
)]
#[case::search_slash_suffix(
    Some("https://proxy.test/v1/search/"),
    &[],
    "https://proxy.test/v1/search"
)]
#[case::env_base(
    None,
    &[("PARALLEL_AI_API_BASE", "https://env.test")],
    "https://env.test/v1/search"
)]
#[case::lookalike_suffix(
    Some("https://proxy.test/v1/search/extra"),
    &[],
    "https://proxy.test/v1/search/extra/v1/search"
)]
#[case::empty_caller_base_is_used_as_given(Some(""), &[], "/v1/search")]
fn endpoint_trims_trailing_slashes_and_appends_v1_search(
    #[case] api_base: Option<&str>,
    #[case] environment: &[(&str, &str)],
    #[case] expected: &str,
) {
    assert_eq!(load(environment).endpoint(api_base), expected);
}

#[test]
fn trait_hooks_shape_headers_and_url() {
    let search = load(&[("PARALLEL_AI_API_KEY", "ai-env")]);
    let validated = search.validate_environment(headers(&[("X-Trace", "1")]), None, None);
    assert_eq!(
        validated.get("x-api-key").map(String::as_str),
        Some("ai-env")
    );
    assert_eq!(
        search
            .complete_url(Some("https://proxy.test/v1"), &Object::new(), None)
            .unwrap(),
        "https://proxy.test/v1/search"
    );
    let keyless = load(&[]).validate_environment(headers(&[("X-Trace", "1")]), None, None);
    assert_eq!(keyless, headers(&[("X-Trace", "1")]));
    let caller_key =
        search.validate_environment(Headers::new(), Some(&SecretString::from("ai-caller")), None);
    assert_eq!(
        caller_key.get("x-api-key").map(String::as_str),
        Some("ai-caller")
    );
}

fn outbound<'call>(
    api_base: &'call str,
    api_key: Option<&'call SecretString>,
    params: &'call Object,
    body: &'call SearchBody,
) -> OutboundSearch<'call, SearchBody> {
    OutboundSearch {
        api_base,
        api_key,
        optional_params: params,
        body,
    }
}

#[test]
fn sign_request_returns_the_key_header() {
    let search = load(&[("PARALLEL_AI_API_KEY", "ai-env")]);
    let params = Object::new();
    let body = string_body("{}");
    let request = outbound("https://api.parallel.ai/v1/search", None, &params, &body);
    let (signed, signed_body) = search.sign_request(Headers::new(), &request).unwrap();
    assert_eq!(signed.get("x-api-key").map(String::as_str), Some("ai-env"));
    assert!(signed_body.is_none());
}

#[test]
fn sign_request_reports_a_missing_key() {
    let search = load(&[]);
    let params = Object::new();
    let body = string_body("{}");
    let request = outbound("https://api.parallel.ai/v1/search", None, &params, &body);
    assert!(matches!(
        search.sign_request(Headers::new(), &request),
        Err(ParallelAiSearchError::MissingApiKey)
    ));
}

#[test]
fn sign_request_rejects_an_untrusted_base() {
    let search = load(&[("PARALLEL_AI_API_KEY", "ai-server")]);
    let params = Object::new();
    let body = string_body("{}");
    let request = outbound("https://evil.test/v1/search", None, &params, &body);
    assert!(matches!(
        search.sign_request(Headers::new(), &request),
        Err(ParallelAiSearchError::Credential(
            CredentialError::UntrustedBase { .. }
        ))
    ));
}

#[test]
fn body_maps_a_string_query_to_queries_and_objective() {
    assert_eq!(
        serde_json::to_string(&string_body("{}")).unwrap(),
        r#"{"search_queries":["rust"],"objective":"rust","mode":"basic"}"#
    );
}

#[test]
fn body_maps_a_list_query_without_objective() {
    let body = load(&[])
        .transform_search_request(
            &SearchQuery::Multiple(vec!["rust".to_owned(), "search".to_owned()]),
            &serde_json::from_str::<Object>("{}").unwrap(),
        )
        .unwrap();
    assert_eq!(
        serde_json::to_string(&body).unwrap(),
        r#"{"search_queries":["rust","search"],"mode":"basic"}"#
    );
}

#[rstest]
#[case::mode(r#"{"mode":"fast"}"#, "fast")]
#[case::empty_mode(r#"{"mode":""}"#, "basic")]
#[case::null_mode(r#"{"mode":null}"#, "basic")]
#[case::processor_base(r#"{"processor":"base"}"#, "basic")]
#[case::processor_pro(r#"{"processor":"pro"}"#, "advanced")]
#[case::processor_kept(r#"{"processor":"turbo"}"#, "turbo")]
#[case::empty_processor(r#"{"processor":""}"#, "basic")]
#[case::null_processor(r#"{"processor":null}"#, "basic")]
#[case::mode_beats_processor(r#"{"mode":"fast","processor":"pro"}"#, "fast")]
#[case::empty_mode_beats_processor(r#"{"mode":"","processor":"pro"}"#, "basic")]
fn body_maps_mode_and_processor(#[case] options: &str, #[case] mode: &str) {
    assert_eq!(
        serde_json::to_string(&string_body(options)).unwrap(),
        format!(r#"{{"search_queries":["rust"],"objective":"rust","mode":"{mode}"}}"#)
    );
}

#[test]
fn body_collects_advanced_settings_in_source_order() {
    let body = string_body(
        r#"{"max_results":5,"country":"us","location":"de","max_chars_per_result":120,"fetch_policy":{"max_age_seconds":60},"search_domain_filter":["news.test"],"include_domains":["mine.test"],"exclude_domains":["spam.test"],"after_date":"2024-01-01","advanced_settings":{"max_results":9,"session":"s1"}}"#,
    );
    assert_eq!(
        serde_json::to_string(&body).unwrap(),
        r#"{"search_queries":["rust"],"objective":"rust","mode":"basic","advanced_settings":{"excerpt_settings":{"max_chars_per_result":120},"fetch_policy":{"max_age_seconds":60},"location":"de","max_results":9,"session":"s1","source_policy":{"after_date":"2024-01-01","exclude_domains":["spam.test"],"include_domains":["mine.test"]}}}"#
    );
}

#[test]
fn body_callers_advanced_settings_replace_derived_keys() {
    let body = string_body(
        r#"{"search_domain_filter":["news.test"],"advanced_settings":{"source_policy":{"exclude_domains":["z.test"]}}}"#,
    );
    assert_eq!(
        serde_json::to_string(&body).unwrap(),
        r#"{"search_queries":["rust"],"objective":"rust","mode":"basic","advanced_settings":{"source_policy":{"exclude_domains":["z.test"]}}}"#
    );
}

#[test]
fn body_keeps_null_advanced_values() {
    let body = string_body(r#"{"max_results":null,"search_domain_filter":null}"#);
    assert_eq!(
        serde_json::to_string(&body).unwrap(),
        r#"{"search_queries":["rust"],"objective":"rust","mode":"basic","advanced_settings":{"max_results":null,"source_policy":{"include_domains":null}}}"#
    );
}

#[test]
fn body_location_wins_over_country() {
    let body = string_body(r#"{"country":"us","location":"de"}"#);
    assert_eq!(
        serde_json::to_string(&body).unwrap(),
        r#"{"search_queries":["rust"],"objective":"rust","mode":"basic","advanced_settings":{"location":"de"}}"#
    );
}

#[test]
fn body_include_domains_wins_over_the_filter() {
    let body = string_body(r#"{"search_domain_filter":["a.test"],"include_domains":["b.test"]}"#);
    assert_eq!(
        serde_json::to_string(&body).unwrap(),
        r#"{"search_queries":["rust"],"objective":"rust","mode":"basic","advanced_settings":{"source_policy":{"include_domains":["b.test"]}}}"#
    );
}

#[test]
fn body_drops_max_tokens_per_page_and_the_usage_key() {
    let body = string_body(
        r#"{"max_tokens_per_page":100,"_parallel_ai_usage":[{"count":1}],"session_id":"s1"}"#,
    );
    assert_eq!(
        serde_json::to_string(&body).unwrap(),
        r#"{"search_queries":["rust"],"objective":"rust","mode":"basic","session_id":"s1"}"#
    );
}

#[test]
fn body_merges_remaining_options_last() {
    let body = string_body(r#"{"search_queries":["mine"],"objective":"mine","client_model":"m"}"#);
    assert_eq!(
        serde_json::to_string(&body).unwrap(),
        r#"{"search_queries":["mine"],"objective":"mine","mode":"basic","client_model":"m"}"#
    );
}

#[rstest]
#[case::null_advanced_settings(r#"{"advanced_settings":null}"#)]
#[case::missing_advanced_settings("{}")]
fn body_accepts_a_missing_advanced_settings(#[case] options: &str) {
    assert_eq!(
        serde_json::to_string(&string_body(options)).unwrap(),
        r#"{"search_queries":["rust"],"objective":"rust","mode":"basic"}"#
    );
}

#[rstest]
#[case::number(r#"{"advanced_settings":5}"#)]
#[case::list(r#"{"advanced_settings":[]}"#)]
#[case::text(r#"{"advanced_settings":"x"}"#)]
fn body_rejects_a_non_object_advanced_settings(#[case] options: &str) {
    let error = load(&[])
        .transform_search_request(
            &SearchQuery::Single("rust".to_owned()),
            &serde_json::from_str::<Object>(options).unwrap(),
        )
        .unwrap_err();
    assert!(matches!(
        error,
        ParallelAiSearchError::OptionType {
            field: "advanced_settings"
        }
    ));
}

#[test]
fn reply_projects_results_extras_and_usage() {
    let response = decode_parallel_ai_reply(
        r#"{"search_id":"s1","session_id":"ss1","results":[{"url":"https://a.test","title":"A","publish_date":"2024-05-01","excerpts":["one","two"]},{"url":"https://b.test","title":"B"}],"usage":[{"name":"sku_search","count":2}],"warnings":[{"type":"notice","message":"m"}]}"#
            .as_bytes(),
    )
    .unwrap();
    assert_eq!(response.object.0, "search");
    assert_eq!(response.results.len(), 2);
    assert_eq!(response.results[0].title, "A");
    assert_eq!(response.results[1].title, "B");
    assert_eq!(response.results[0].url, "https://a.test");
    assert_eq!(response.results[0].snippet, "one ... two");
    assert_eq!(response.results[0].date.as_deref(), Some("2024-05-01"));
    assert_eq!(
        response.results[0].extra,
        serde_json::from_str::<Object>(r#"{"excerpts":["one","two"]}"#).unwrap()
    );
    assert_eq!(response.results[1].snippet, "");
    assert_eq!(response.results[1].date, None);
    assert_eq!(
        response.results[1].extra,
        serde_json::from_str::<Object>(r#"{"excerpts":[]}"#).unwrap()
    );
    assert_eq!(
        response.extra,
        serde_json::from_str::<Object>(
            r#"{"search_id":"s1","session_id":"ss1","parallel_usage":[{"name":"sku_search","count":2}],"warnings":[{"type":"notice","message":"m"}]}"#
        )
        .unwrap()
    );
    assert_eq!(
        response.hidden_params,
        serde_json::from_str::<Object>(
            r#"{"_parallel_ai_usage":[{"name":"sku_search","count":2}]}"#
        )
        .unwrap()
    );
}

#[rstest]
#[case::empty_reply("{}")]
#[case::empty_results(r#"{"results":[]}"#)]
fn reply_defaults_missing_members_to_empty(#[case] body: &str) {
    let response = decode_parallel_ai_reply(body.as_bytes()).unwrap();
    assert!(response.results.is_empty());
    assert!(response.extra.is_empty());
}

#[test]
fn reply_stores_a_null_usage_key() {
    let response = decode_parallel_ai_reply(br#"{"usage":null,"results":[]}"#).unwrap();
    assert_eq!(
        response.hidden_params,
        serde_json::from_str::<Object>(r#"{"_parallel_ai_usage":null}"#).unwrap()
    );
    assert!(!response.extra.contains_key("parallel_usage"));
}

#[test]
fn reply_keeps_empty_warnings_and_a_null_usage_absent() {
    let response = decode_parallel_ai_reply(br#"{"warnings":[],"results":[]}"#).unwrap();
    assert_eq!(
        response.extra,
        serde_json::from_str::<Object>(r#"{"warnings":[]}"#).unwrap()
    );
}

#[test]
fn reply_treats_explicit_nulls_like_missing_members() {
    let response = decode_parallel_ai_reply(
        r#"{"search_id":null,"session_id":null,"usage":null,"warnings":null,"results":[{"title":null,"url":null,"publish_date":null,"excerpts":null}]}"#
            .as_bytes(),
    )
    .unwrap();
    assert!(response.extra.is_empty());
    assert_eq!(
        response.hidden_params,
        serde_json::from_str::<Object>(r#"{"_parallel_ai_usage":null}"#).unwrap()
    );
    assert_eq!(response.results[0].title, "");
    assert_eq!(response.results[0].url, "");
    assert_eq!(response.results[0].snippet, "");
    assert_eq!(response.results[0].date, None);
    assert_eq!(
        response.results[0].extra,
        serde_json::from_str::<Object>(r#"{"excerpts":[]}"#).unwrap()
    );
}

#[rstest]
#[case::empty_excerpts(r#"{"excerpts":[]}"#)]
#[case::null_excerpts(r#"{"excerpts":null}"#)]
#[case::missing_excerpts("{}")]
fn reply_projects_empty_excerpts(#[case] result: &str) {
    let response =
        decode_parallel_ai_reply(format!(r#"{{"results":[{result}]}}"#).as_bytes()).unwrap();
    assert_eq!(response.results[0].snippet, "");
    assert_eq!(
        response.results[0].extra,
        serde_json::from_str::<Object>(r#"{"excerpts":[]}"#).unwrap()
    );
}

#[rstest]
#[case::top_level_array("[]")]
#[case::top_level_text(r#""x""#)]
#[case::results_is_an_object(r#"{"results":{}}"#)]
#[case::results_is_text(r#"{"results":"x"}"#)]
#[case::null_results(r#"{"results":null}"#)]
#[case::null_result(r#"{"results":[null]}"#)]
#[case::list_result(r#"{"results":[[]]}"#)]
#[case::numeric_title(r#"{"results":[{"title":7}]}"#)]
#[case::numeric_url(r#"{"results":[{"url":7}]}"#)]
#[case::numeric_date(r#"{"results":[{"publish_date":7}]}"#)]
#[case::numeric_excerpt(r#"{"results":[{"excerpts":[7]}]}"#)]
#[case::text_excerpts(r#"{"results":[{"excerpts":"one"}]}"#)]
#[case::numeric_usage(r#"{"usage":7}"#)]
#[case::usage_is_an_object(r#"{"usage":{"count":1}}"#)]
#[case::usage_entry_is_text(r#"{"usage":[1]}"#)]
#[case::numeric_warnings(r#"{"warnings":7}"#)]
#[case::numeric_search_id(r#"{"search_id":7}"#)]
#[case::invalid_json("not json")]
#[case::trailing_bytes(r#"{"results":[]}x"#)]
fn reply_rejects_a_shape_the_source_would_fail(#[case] body: &str) {
    assert!(decode_parallel_ai_reply(body.as_bytes()).is_err(), "{body}");
}

#[test]
fn transform_search_response_reports_a_schema_failure() {
    let response = ProviderResponse {
        status_code: 200,
        headers: &Headers::new(),
        text: "not json",
    };
    match load(&[]).transform_search_response(&response).unwrap_err() {
        ParallelAiSearchError::Reply(ParallelAiReplyError::Schema { .. }) => {}
        other @ (ParallelAiSearchError::Credential(_)
        | ParallelAiSearchError::MissingApiKey
        | ParallelAiSearchError::OptionType { .. }) => {
            panic!("expected a schema error, got {other}")
        }
    }
}

#[rstest]
fn replays_the_parallel_ai_corpus(#[files("fuzz/corpus/parallel_ai_reply/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    if let Ok(parsed) = decode_parallel_ai_reply(&bytes) {
        assert_eq!(parsed.object.0, "search");
        assert!(parsed.hidden_params.contains_key(PARALLEL_AI_USAGE_PARAM));
    }
}
