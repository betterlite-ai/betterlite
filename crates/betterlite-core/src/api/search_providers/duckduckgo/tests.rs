#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::provider::{
    HttpMethod, ProviderResponse, SearchProvider as _, SearchQuery,
};
use betterlite_openai::search::records::{SearchObject, SearchResponse};
use rstest::rstest;

use super::reply::{REPLY_MAX_BYTES, decode_duckduckgo_search};
use super::request::{DuckDuckGoParams, build_request, search_url};
use super::{DuckDuckGoSearchConfig, DuckDuckGoSearchError};

fn environment(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn options(pairs: &[(&str, JsonValue)]) -> Object {
    Object::from_iter(
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone())),
    )
}

fn text(value: &str) -> JsonValue {
    JsonValue::String(value.to_owned())
}

fn config(variables: &[(&str, &str)]) -> DuckDuckGoSearchConfig {
    DuckDuckGoSearchConfig::load(&environment(variables))
}

fn params(max_results: Option<JsonValue>) -> DuckDuckGoParams {
    DuckDuckGoParams {
        query: "q".to_owned(),
        format: "json".to_owned(),
        max_results,
        pretty: None,
        no_redirect: None,
        no_html: None,
        skip_disambig: None,
    }
}

fn decode(
    body: &str,
    max_results: Option<JsonValue>,
) -> Result<SearchResponse, DuckDuckGoSearchError> {
    decode_duckduckgo_search(body.as_bytes(), &params(max_results))
}

fn single(query: &str) -> SearchQuery {
    SearchQuery::Single(query.to_owned())
}

fn topic_reply(topic: &str) -> String {
    let quoted = serde_json::to_string(topic).unwrap();
    format!(r#"{{"RelatedTopics":[{{"Text":{quoted},"FirstURL":"https://a.test"}}]}}"#)
}

#[test]
fn the_friendly_name_and_method_are_native() {
    assert_eq!(DuckDuckGoSearchConfig::ui_friendly_name(), "DuckDuckGo");
    assert_eq!(config(&[]).http_method(), HttpMethod::Get);
}

#[test]
fn validate_environment_keeps_headers_and_sets_json() {
    let headers = config(&[]).validate_environment(environment(&[("X-Trace", "abc")]), None, None);

    assert_eq!(headers.get("X-Trace").map(String::as_str), Some("abc"));
    assert_eq!(
        headers.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(headers.get("Authorization"), None);
}

#[test]
fn duckduckgo_search_settings_default_the_base() {
    let loaded = config(&[]);
    let body = build_request(&single("rust"), &Object::new()).unwrap();

    let url = loaded
        .complete_url(None, &Object::new(), Some(&body))
        .unwrap();

    assert_eq!(url, "https://api.duckduckgo.com/?q=rust&format=json");
}

#[test]
fn duckduckgo_search_settings_prefer_the_caller_base() {
    let loaded = config(&[("DUCKDUCKGO_API_BASE", "https://environment.test")]);
    let body = build_request(&single("rust"), &Object::new()).unwrap();

    let explicit = loaded
        .complete_url(Some("https://caller.test"), &Object::new(), Some(&body))
        .unwrap();
    let empty = loaded
        .complete_url(Some(""), &Object::new(), Some(&body))
        .unwrap();
    let absent = loaded
        .complete_url(None, &Object::new(), Some(&body))
        .unwrap();

    assert_eq!(explicit, "https://caller.test/?q=rust&format=json");
    assert_eq!(empty, "https://environment.test/?q=rust&format=json");
    assert_eq!(absent, "https://environment.test/?q=rust&format=json");
}

#[test]
fn duckduckgo_search_settings_read_the_environment_base() {
    let loaded = config(&[("DUCKDUCKGO_API_BASE", "https://environment.test")]);
    let body = build_request(&single("rust"), &Object::new()).unwrap();

    let url = loaded
        .complete_url(None, &Object::new(), Some(&body))
        .unwrap();

    assert_eq!(url, "https://environment.test/?q=rust&format=json");
}

#[test]
fn duckduckgo_search_settings_treat_an_empty_base_as_unset() {
    let loaded = config(&[("DUCKDUCKGO_API_BASE", "")]);
    let body = build_request(&single("rust"), &Object::new()).unwrap();

    let url = loaded
        .complete_url(None, &Object::new(), Some(&body))
        .unwrap();

    assert_eq!(url, "https://api.duckduckgo.com/?q=rust&format=json");
}

#[test]
fn a_trailing_slash_is_not_trimmed() {
    let body = build_request(&single("rust"), &Object::new()).unwrap();

    assert_eq!(
        search_url(None, "https://instant.test/", Some(&body)),
        "https://instant.test//?q=rust&format=json"
    );
    assert_eq!(
        search_url(None, "https://instant.test/", None),
        "https://instant.test/"
    );
}

#[test]
fn a_query_list_joins_with_spaces() {
    let query = SearchQuery::Multiple(vec!["rust".to_owned(), "language".to_owned()]);

    let body = build_request(&query, &Object::new()).unwrap();

    assert_eq!(body.params.query, "rust language");
}

#[test]
fn format_is_forced_to_json() {
    let body = build_request(&single("q"), &options(&[("format", text("xml"))])).unwrap();

    assert_eq!(body.params.format, "json");
    assert_eq!(
        search_url(None, "https://instant.test", Some(&body)),
        "https://instant.test/?q=q&format=json"
    );
}

#[test]
fn only_duckduckgo_options_are_copied() {
    let extra = options(&[
        ("country", text("us")),
        ("max_tokens_per_page", JsonValue::from(100_i64)),
        ("unrelated", JsonValue::Bool(true)),
        ("pretty", JsonValue::from(1_i64)),
    ]);

    let body = build_request(&single("q"), &extra).unwrap();

    assert_eq!(body.params.pretty, Some(1));
    assert_eq!(
        serde_json::to_value(&body.params).unwrap(),
        serde_json::from_str::<JsonValue>(r#"{"q":"q","format":"json","pretty":1}"#).unwrap()
    );
}

#[test]
fn max_results_is_copied_to_the_limit_key() {
    let body = build_request(
        &single("q"),
        &options(&[("max_results", JsonValue::from(3_i64))]),
    )
    .unwrap();

    assert_eq!(body.params.max_results, Some(JsonValue::from(3_i64)));
    assert_eq!(
        search_url(None, "https://instant.test", Some(&body)),
        "https://instant.test/?q=q&format=json&_max_results=3"
    );
}

#[test]
fn an_array_limit_repeats_like_doseq() {
    let body = build_request(
        &single("q"),
        &options(&[(
            "max_results",
            JsonValue::Array(vec![JsonValue::from(4_i64), text("5"), JsonValue::Null]),
        )]),
    )
    .unwrap();
    let empty = build_request(
        &single("q"),
        &options(&[("max_results", JsonValue::Array(Vec::new()))]),
    )
    .unwrap();

    assert_eq!(
        search_url(None, "https://instant.test", Some(&body)),
        "https://instant.test/?q=q&format=json&_max_results=4&_max_results=5&_max_results=None"
    );
    assert_eq!(
        search_url(None, "https://instant.test", Some(&empty)),
        "https://instant.test/?q=q&format=json"
    );
}

#[rstest]
#[case(JsonValue::Bool(true), "True")]
#[case(JsonValue::Bool(false), "False")]
#[case(JsonValue::from(1.5_f64), "1.5")]
fn scalar_limits_are_written_as_python_does(#[case] value: JsonValue, #[case] written: &str) {
    let body = build_request(&single("q"), &options(&[("max_results", value)])).unwrap();

    assert_eq!(
        search_url(None, "https://instant.test", Some(&body)),
        format!("https://instant.test/?q=q&format=json&_max_results={written}")
    );
}

#[test]
fn null_options_are_left_out() {
    let body = build_request(
        &single("q"),
        &options(&[
            ("max_results", JsonValue::Null),
            ("pretty", JsonValue::Null),
            ("no_redirect", JsonValue::Null),
            ("no_html", JsonValue::Null),
            ("skip_disambig", JsonValue::Null),
        ]),
    )
    .unwrap();

    assert_eq!(body.params.max_results, None);
    assert_eq!(body.params.pretty, None);
    assert_eq!(body.params.no_redirect, None);
    assert_eq!(body.params.no_html, None);
    assert_eq!(body.params.skip_disambig, None);
}

#[rstest]
#[case("pretty", text("1"))]
#[case("no_redirect", text("1"))]
#[case("no_html", text("1"))]
#[case("skip_disambig", text("1"))]
#[case("pretty", JsonValue::from(9_223_372_036_854_775_808_u64))]
#[case("no_redirect", JsonValue::from(9_223_372_036_854_775_808_u64))]
#[case("no_html", JsonValue::from(9_223_372_036_854_775_808_u64))]
#[case("skip_disambig", JsonValue::from(9_223_372_036_854_775_808_u64))]
fn an_unsupported_option_value_names_the_field(#[case] field: &str, #[case] value: JsonValue) {
    let error = build_request(&single("q"), &options(&[(field, value)])).unwrap_err();

    match error {
        DuckDuckGoSearchError::OptionType { field: named } => assert_eq!(named, field),
        other => panic!("unexpected error {other:?}"),
    }
}

#[test]
fn the_duckduckgo_flags_keep_their_order() {
    let body = build_request(
        &single("q"),
        &options(&[
            ("skip_disambig", JsonValue::from(1_i64)),
            ("no_html", JsonValue::from(1_i64)),
            ("no_redirect", JsonValue::from(1_i64)),
            ("pretty", JsonValue::from(1_i64)),
            ("max_results", JsonValue::from(2_i64)),
        ]),
    )
    .unwrap();

    assert_eq!(
        search_url(None, "https://instant.test", Some(&body)),
        "https://instant.test/?q=q&format=json&_max_results=2&pretty=1&no_redirect=1&no_html=1&skip_disambig=1"
    );
}

#[test]
fn the_url_encodes_like_python() {
    let query = SearchQuery::Multiple(vec!["a b".to_owned(), "~*'".to_owned()]);

    let body = build_request(&query, &Object::new()).unwrap();

    assert_eq!(
        search_url(None, "https://instant.test", Some(&body)),
        "https://instant.test/?q=a+b+~%2A%27&format=json"
    );
}

#[test]
fn a_reply_projects_topics_in_order() {
    let body = r#"{"RelatedTopics":[
        {"Text":"A - one","FirstURL":"https://a.test"},
        {"Text":"B - two","FirstURL":"https://b.test"}
    ]}"#;

    let response = decode(body, None).unwrap();

    assert_eq!(response.object, SearchObject::default());
    assert_eq!(response.results.len(), 2);
    assert_eq!(response.results[0].title, "A");
    assert_eq!(response.results[0].snippet, "one");
    assert_eq!(response.results[0].url, "https://a.test");
    assert_eq!(response.results[0].date, None);
    assert_eq!(response.results[0].last_updated, None);
    assert_eq!(response.results[1].title, "B");
    assert_eq!(response.results[1].url, "https://b.test");
}

#[rstest]
#[case(0_i64, 0)]
#[case(1_i64, 1)]
#[case(2_i64, 2)]
#[case(9_i64, 2)]
fn the_limit_stops_before_the_next_topic(#[case] limit: i64, #[case] expected: usize) {
    let body = r#"{"RelatedTopics":[
        {"Text":"A - one","FirstURL":"https://a.test"},
        {"Text":"B - two","FirstURL":"https://b.test"}
    ]}"#;

    let response = decode(body, Some(JsonValue::from(limit))).unwrap();

    assert_eq!(response.results.len(), expected);
}

#[test]
fn a_limit_is_read_from_a_numeric_string() {
    let body = r#"{"RelatedTopics":[
        {"Text":"A - one","FirstURL":"https://a.test"},
        {"Text":"B - two","FirstURL":"https://b.test"}
    ]}"#;

    let response = decode(body, Some(text("1"))).unwrap();
    let padded = decode(body, Some(text(" 1 "))).unwrap();

    assert_eq!(response.results.len(), 1);
    assert_eq!(padded.results.len(), 1);
}

#[rstest]
#[case(JsonValue::String("abc".to_owned()))]
#[case(JsonValue::String("0_1".to_owned()))]
#[case(JsonValue::Bool(true))]
#[case(JsonValue::from(1.5_f64))]
#[case(JsonValue::Null)]
#[case(JsonValue::Array(Vec::new()))]
#[case(JsonValue::Array(vec![
    JsonValue::from(4_i64),
    JsonValue::String("5".to_owned()),
    JsonValue::Null,
]))]
fn an_invalid_limit_leaves_every_topic(#[case] limit: JsonValue) {
    let body = r#"{"RelatedTopics":[
        {"Text":"A - one","FirstURL":"https://a.test"},
        {"Text":"B - two","FirstURL":"https://b.test"},
        {"Text":"C - three","FirstURL":"https://c.test"}
    ]}"#;

    let response = decode(body, Some(limit)).unwrap();

    let urls: Vec<&str> = response
        .results
        .iter()
        .map(|result| result.url.as_str())
        .collect();
    assert_eq!(urls, ["https://a.test", "https://b.test", "https://c.test"]);
}

#[test]
fn an_out_of_range_limit_leaves_every_topic() {
    let body = r#"{"RelatedTopics":[
        {"Text":"A - one","FirstURL":"https://a.test"},
        {"Text":"B - two","FirstURL":"https://b.test"}
    ]}"#;
    let limit = serde_json::from_str::<JsonValue>("-100000000000000000000").unwrap();

    let response = decode(body, Some(limit)).unwrap();

    assert_eq!(response.results.len(), 2);
}

#[rstest]
#[case(0_i64)]
#[case(-3_i64)]
fn an_abstract_ignores_a_spent_limit(#[case] limit: i64) {
    let body = r#"{"AbstractURL":"https://abstract.test","AbstractText":"An abstract","Heading":"H",
        "RelatedTopics":[{"Text":"A - one","FirstURL":"https://a.test"}]}"#;

    let response = decode(body, Some(JsonValue::from(limit))).unwrap();

    assert_eq!(response.results.len(), 1);
    assert_eq!(response.results[0].title, "H");
    assert_eq!(response.results[0].url, "https://abstract.test");
    assert_eq!(response.results[0].snippet, "An abstract");
}

#[rstest]
#[case(
    r#"{"AbstractURL":"https://a.test","AbstractText":"text","Heading":"H"}"#,
    true,
    "H"
)]
#[case(r#"{"AbstractURL":"https://a.test","AbstractText":"text"}"#, true, "")]
#[case(r#"{"AbstractText":"text","Heading":"H"}"#, false, "")]
#[case(r#"{"AbstractURL":"https://a.test","Heading":"H"}"#, false, "")]
#[case(
    r#"{"AbstractURL":null,"AbstractText":"text","Heading":"H"}"#,
    false,
    ""
)]
#[case(
    r#"{"AbstractURL":"https://a.test","AbstractText":null,"Heading":"H"}"#,
    false,
    ""
)]
#[case(r#"{"AbstractURL":"","AbstractText":"text","Heading":"H"}"#, false, "")]
#[case(
    r#"{"AbstractURL":"https://a.test","AbstractText":"","Heading":"H"}"#,
    false,
    ""
)]
fn an_abstract_needs_both_ends(#[case] body: &str, #[case] present: bool, #[case] title: &str) {
    let response = decode(body, None).unwrap();

    assert_eq!(response.results.len(), usize::from(present));
    if present {
        assert_eq!(response.results[0].title, title);
        assert_eq!(response.results[0].url, "https://a.test");
        assert_eq!(response.results[0].snippet, "text");
    }
}

#[test]
fn a_direct_topic_wins_over_its_topics_field() {
    let body = r#"{"RelatedTopics":[{"Text":"direct - x","FirstURL":"https://direct.test",
        "Topics":[{"Text":"nested - y","FirstURL":"https://nested.test"}]}]}"#;

    let response = decode(body, None).unwrap();

    assert_eq!(response.results.len(), 1);
    assert_eq!(response.results[0].url, "https://direct.test");
    assert_eq!(response.results[0].title, "direct");
}

#[test]
fn a_nested_level_is_visited_once() {
    let body = r#"{"RelatedTopics":[{"Name":"Group","Topics":[
        {"Text":"inner - x","FirstURL":"https://inner.test",
         "Topics":[{"Text":"deep - y","FirstURL":"https://deep.test"}]}
    ]}]}"#;

    let response = decode(body, None).unwrap();

    assert_eq!(response.results.len(), 1);
    assert_eq!(response.results[0].url, "https://inner.test");
    assert_eq!(response.results[0].title, "inner");
}

#[test]
fn a_nested_member_needs_both_fields() {
    let body = r#"{"RelatedTopics":[{"Name":"Group","Topics":[
        {"Text":"only text"},
        {"FirstURL":"https://url.test"},
        {}
    ]}]}"#;

    let response = decode(body, None).unwrap();

    assert!(response.results.is_empty());
}

#[test]
fn the_limit_reaches_nested_members() {
    let body = r#"{"RelatedTopics":[{"Name":"Group","Topics":[
        {"Text":"inner - one","FirstURL":"https://one.test"},
        {"Text":"inner - two","FirstURL":"https://two.test"}
    ]}]}"#;

    let response = decode(body, Some(JsonValue::from(1_i64))).unwrap();

    assert_eq!(response.results.len(), 1);
    assert_eq!(response.results[0].url, "https://one.test");
}

#[test]
fn a_non_mapping_top_level_member_is_skipped() {
    let body = r#"{"RelatedTopics":[null,"text",5,[],
        {"Text":"A - one","FirstURL":"https://a.test"}]}"#;

    let response = decode(body, None).unwrap();

    assert_eq!(response.results.len(), 1);
}

#[test]
fn an_empty_topic_text_is_kept() {
    let body = r#"{"RelatedTopics":[
        {"Text":"","FirstURL":"https://empty-text.test"},
        {"Text":"B - two","FirstURL":""}
    ]}"#;

    let response = decode(body, None).unwrap();

    assert_eq!(response.results.len(), 2);
    assert_eq!(response.results[0].title, "");
    assert_eq!(response.results[0].snippet, "");
    assert_eq!(response.results[1].url, "");
}

#[rstest]
#[case(r#"{"RelatedTopics":"text"}"#)]
#[case(r#"{"RelatedTopics":5}"#)]
#[case(r#"{"RelatedTopics":{}}"#)]
fn related_topics_must_be_a_list(#[case] body: &str) {
    let error = decode(body, None).unwrap_err();

    match error {
        DuckDuckGoSearchError::ReplyList { field, .. } => assert_eq!(field, "RelatedTopics"),
        other => panic!("unexpected error {other:?}"),
    }
}

#[test]
fn a_null_related_topics_field_fails() {
    let error = decode(r#"{"RelatedTopics":null}"#, None).unwrap_err();

    match error {
        DuckDuckGoSearchError::ReplyNull { field } => assert_eq!(field, "RelatedTopics"),
        other => panic!("unexpected error {other:?}"),
    }
}

#[test]
fn a_null_topics_field_fails() {
    let body = r#"{"RelatedTopics":[{"Name":"Group","Topics":null}]}"#;

    let error = decode(body, None).unwrap_err();

    match error {
        DuckDuckGoSearchError::ReplyNull { field } => assert_eq!(field, "Topics"),
        other => panic!("unexpected error {other:?}"),
    }
}

#[test]
fn a_nested_entry_must_be_an_object() {
    let body = r#"{"RelatedTopics":[{"Name":"Group","Topics":["text"]}]}"#;

    let error = decode(body, None).unwrap_err();

    assert!(matches!(error, DuckDuckGoSearchError::NestedEntry));
}

#[test]
fn titles_split_at_the_first_separator() {
    let response = decode(&topic_reply("A - B - C"), None).unwrap();

    assert_eq!(response.results[0].title, "A");
    assert_eq!(response.results[0].snippet, "B - C");
}

#[test]
fn long_titles_truncate_at_fifty_code_points() {
    let long = "é".repeat(51);
    let response = decode(&topic_reply(&long), None).unwrap();
    assert_eq!(response.results[0].title.chars().count(), 53);
    assert!(response.results[0].title.ends_with("..."));
    assert_eq!(response.results[0].snippet, long);

    let exact = "é".repeat(50);
    let response = decode(&topic_reply(&exact), None).unwrap();
    assert_eq!(response.results[0].title, exact);
    assert_eq!(response.results[0].snippet, exact);
}

#[test]
fn a_short_title_stays_whole() {
    let response = decode(&topic_reply("plain text"), None).unwrap();

    assert_eq!(response.results[0].title, "plain text");
    assert_eq!(response.results[0].snippet, "plain text");
}

#[test]
fn the_answer_field_is_ignored() {
    let body = r#"{"Answer":"42","AnswerType":"calc",
        "RelatedTopics":[{"Text":"A - one","FirstURL":"https://a.test"}]}"#;

    let response = decode(body, None).unwrap();

    assert_eq!(response.results.len(), 1);
    assert_eq!(response.results[0].snippet, "one");
}

#[rstest]
#[case("[]")]
#[case("\"text\"")]
#[case("")]
#[case("{not json")]
fn a_reply_that_is_not_an_object_fails(#[case] body: &str) {
    let error = decode(body, None).unwrap_err();

    assert!(matches!(
        error,
        DuckDuckGoSearchError::ReplyNotObject | DuckDuckGoSearchError::ReplyJson { .. }
    ));
}

#[test]
fn a_json_array_reply_is_not_an_object() {
    let error = decode("[]", None).unwrap_err();

    assert!(matches!(error, DuckDuckGoSearchError::ReplyNotObject));
}

#[test]
fn a_non_standard_constant_fails_the_decode() {
    let body = r#"{"Answer":NaN,"RelatedTopics":[{"Text":"A - one","FirstURL":"https://a.test"}]}"#;

    let error = decode(body, None).unwrap_err();

    assert!(matches!(error, DuckDuckGoSearchError::ReplyJson { .. }));
}

#[test]
fn a_reply_that_is_too_large_fails() {
    let bytes = vec![b' '; REPLY_MAX_BYTES.saturating_add(1)];

    let error = decode_duckduckgo_search(&bytes, &params(None)).unwrap_err();

    match error {
        DuckDuckGoSearchError::ReplyTooLarge { limit } => assert_eq!(limit, REPLY_MAX_BYTES),
        other => panic!("unexpected error {other:?}"),
    }
}

#[test]
fn a_reply_at_the_size_limit_is_read() {
    let mut bytes = vec![b' '; REPLY_MAX_BYTES];
    bytes[..2].copy_from_slice(b"{}");

    let response = decode_duckduckgo_search(&bytes, &params(None)).unwrap();

    assert!(response.results.is_empty());
}

#[rstest]
#[case(r#"{"AbstractURL":5}"#, "AbstractURL")]
#[case(r#"{"AbstractURL":5,"AbstractText":"text"}"#, "AbstractURL")]
#[case(r#"{"AbstractURL":"https://a.test","AbstractText":5}"#, "AbstractText")]
#[case(
    r#"{"AbstractURL":"https://a.test","AbstractText":"text","Heading":5}"#,
    "Heading"
)]
fn a_wrong_typed_abstract_fails(#[case] body: &str, #[case] field: &str) {
    let error = decode(body, None).unwrap_err();

    match error {
        DuckDuckGoSearchError::ReplyText { field: named, .. } => assert_eq!(named, field),
        other => panic!("unexpected error {other:?}"),
    }
}

#[test]
fn a_null_heading_fails() {
    let body = r#"{"AbstractURL":"https://a.test","AbstractText":"text","Heading":null}"#;

    let error = decode(body, None).unwrap_err();

    match error {
        DuckDuckGoSearchError::ReplyNull { field } => assert_eq!(field, "Heading"),
        other => panic!("unexpected error {other:?}"),
    }
}

#[rstest]
#[case(
    r#"{"Text":null,"FirstURL":"https://a.test"}"#,
    "the DuckDuckGo search reply field `Text` is null"
)]
#[case(
    r#"{"Text":"A - one","FirstURL":null}"#,
    "the DuckDuckGo search reply field `FirstURL` is null"
)]
fn a_null_topic_field_fails(#[case] topic: &str, #[case] message: &str) {
    let body = format!(r#"{{"RelatedTopics":[{topic}]}}"#);

    assert_eq!(decode(&body, None).unwrap_err().to_string(), message);
}

#[rstest]
#[case(r#"{"Text":5,"FirstURL":"https://a.test"}"#, "Text")]
#[case(r#"{"Text":"A - one","FirstURL":5}"#, "FirstURL")]
fn a_wrong_typed_topic_field_fails(#[case] topic: &str, #[case] field: &str) {
    let body = format!(r#"{{"RelatedTopics":[{topic}]}}"#);

    let error = decode(&body, None).unwrap_err();

    match error {
        DuckDuckGoSearchError::ReplyText { field: named, .. } => assert_eq!(named, field),
        other => panic!("unexpected error {other:?}"),
    }
}

#[test]
fn the_config_decodes_with_the_query_it_built() {
    let loaded = config(&[]);
    let request_options = options(&[("max_results", JsonValue::from(1_i64))]);
    let headers = Headers::new();
    loaded
        .transform_search_request(&single("q"), &request_options)
        .unwrap();

    let body = r#"{"RelatedTopics":[
        {"Text":"A - one","FirstURL":"https://a.test"},
        {"Text":"B - two","FirstURL":"https://b.test"}
    ]}"#;
    let response = loaded
        .transform_search_response(&ProviderResponse {
            status_code: 200,
            headers: &headers,
            text: body,
        })
        .unwrap();

    assert_eq!(response.results.len(), 1);
}

#[test]
fn the_config_without_a_request_has_no_limit() {
    let loaded = config(&[]);
    let headers = Headers::new();
    let body = r#"{"RelatedTopics":[
        {"Text":"A - one","FirstURL":"https://a.test"},
        {"Text":"B - two","FirstURL":"https://b.test"}
    ]}"#;

    let response = loaded
        .transform_search_response(&ProviderResponse {
            status_code: 200,
            headers: &headers,
            text: body,
        })
        .unwrap();

    assert_eq!(response.results.len(), 2);
}

#[rstest]
fn replays_the_duckduckgo_corpus(#[files("fuzz/corpus/duckduckgo_reply/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();

    drop(decode_duckduckgo_search(&bytes, &params(None)));
}
