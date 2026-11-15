#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::io::Error as IoError;
use std::path::PathBuf;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use super::enhance::{PerplexityReplyError, apply};
use super::{
    CapabilityLookup, SUPPORTED_PARAMS, provider_info, supported_openai_params,
    with_perplexity_fields,
};

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn key_text(key: Option<&SecretString>) -> Option<String> {
    let Some(secret) = key else {
        return None;
    };
    Some(secret.expose_secret().to_owned())
}

fn fixture(text: &str) -> Value {
    serde_json::from_str(text).expect("the JSON fixture")
}

fn enhanced_reply(reply_text: &str, raw: &str) -> Value {
    let parsed: Value = serde_json::from_str(reply_text).expect("the reply fixture");
    with_perplexity_fields(parsed, raw.as_bytes())
}

#[test]
fn provider_info_prefers_the_explicit_base() {
    let environment = environment(&[("PERPLEXITY_API_BASE", "https://env.example")]);
    let (base, _key) = provider_info(Some("https://explicit.example"), None, &environment);
    assert_eq!(base, "https://explicit.example");
}

#[test]
fn provider_info_reads_the_base_environment() {
    let environment = environment(&[("PERPLEXITY_API_BASE", "https://env.example")]);
    let (base, _key) = provider_info(None, None, &environment);
    assert_eq!(base, "https://env.example");
}

#[test]
fn provider_info_defaults_the_base() {
    let environment = environment(&[]);
    let (base, _key) = provider_info(None, None, &environment);
    assert_eq!(base, "https://api.perplexity.ai");
}

#[test]
fn provider_info_skips_an_empty_explicit_base() {
    let environment = environment(&[("PERPLEXITY_API_BASE", "https://env.example")]);
    let (base, _key) = provider_info(Some(""), None, &environment);
    assert_eq!(base, "https://env.example");
}

#[test]
fn provider_info_treats_an_empty_base_environment_as_unset() {
    let environment = environment(&[("PERPLEXITY_API_BASE", "")]);
    let (base, _key) = provider_info(None, None, &environment);
    assert_eq!(base, "https://api.perplexity.ai");
}

#[test]
fn provider_info_skips_an_empty_explicit_key() {
    let environment = environment(&[("PERPLEXITYAI_API_KEY", "env-key")]);
    let (_, key) = provider_info(None, Some(&secret("")), &environment);
    assert_eq!(key_text(key.as_ref()).as_deref(), Some("env-key"));
}

#[test]
fn provider_info_prefers_the_explicit_key() {
    let environment = environment(&[("PERPLEXITYAI_API_KEY", "env-key")]);
    let (_, key) = provider_info(None, Some(&secret("explicit-key")), &environment);
    assert_eq!(key_text(key.as_ref()).as_deref(), Some("explicit-key"));
}

#[test]
fn provider_info_reads_perplexityai_api_key_first() {
    let environment = environment(&[
        ("PERPLEXITYAI_API_KEY", "first-key"),
        ("PERPLEXITY_API_KEY", "second-key"),
    ]);
    let (_, key) = provider_info(None, None, &environment);
    assert_eq!(key_text(key.as_ref()).as_deref(), Some("first-key"));
}

#[test]
fn provider_info_falls_back_to_perplexity_api_key() {
    let environment = environment(&[("PERPLEXITY_API_KEY", "second-key")]);
    let (_, key) = provider_info(None, None, &environment);
    assert_eq!(key_text(key.as_ref()).as_deref(), Some("second-key"));
}

#[test]
fn provider_info_treats_an_empty_key_environment_as_unset() {
    let environment = environment(&[
        ("PERPLEXITYAI_API_KEY", ""),
        ("PERPLEXITY_API_KEY", "second-key"),
    ]);
    let (_, key) = provider_info(None, None, &environment);
    assert_eq!(key_text(key.as_ref()).as_deref(), Some("second-key"));
}

#[test]
fn provider_info_has_no_key_without_one() {
    let environment = environment(&[]);
    let (_, key) = provider_info(None, None, &environment);
    assert!(key.is_none());
}

struct Lookup {
    reasoning: Option<bool>,
    web_search: Option<bool>,
}

impl CapabilityLookup for Lookup {
    type Error = IoError;

    fn supports_reasoning(&self, _model: &str) -> Result<bool, Self::Error> {
        self.reasoning
            .ok_or_else(|| IoError::other("the reasoning lookup failed"))
    }

    fn supports_web_search(&self, _model: &str) -> Result<bool, Self::Error> {
        self.web_search
            .ok_or_else(|| IoError::other("the web search lookup failed"))
    }
}

#[rstest]
#[case::both_supported(Some(true), Some(true), &["reasoning_effort", "web_search_options"])]
#[case::reasoning_only(Some(true), Some(false), &["reasoning_effort"])]
#[case::web_search_only(Some(false), Some(true), &["web_search_options"])]
#[case::neither(Some(false), Some(false), &[])]
#[case::failed_reasoning_still_checks_web_search(None, Some(true), &["web_search_options"])]
#[case::failed_web_search_still_checks_reasoning(Some(true), None, &["reasoning_effort"])]
#[case::both_lookups_failed(None, None, &[])]
fn supported_openai_params_appends_capabilities(
    #[case] reasoning: Option<bool>,
    #[case] web_search: Option<bool>,
    #[case] appended: &[&'static str],
) {
    let lookup = Lookup {
        reasoning,
        web_search,
    };
    let params = supported_openai_params(&lookup, "sonar-pro");
    let mut expected = SUPPORTED_PARAMS.to_vec();
    expected.extend_from_slice(appended);
    assert_eq!(params, expected);
}

#[rstest]
#[case("tools")]
#[case("tool_choice")]
#[case("function_call")]
#[case("functions")]
fn supported_openai_params_never_lists_tools(#[case] unsupported: &'static str) {
    let lookup = Lookup {
        reasoning: Some(true),
        web_search: Some(true),
    };
    let params = supported_openai_params(&lookup, "sonar-pro");
    assert!(
        !params.contains(&unsupported),
        "{unsupported} was advertised"
    );
}

#[rstest]
#[case::four_characters(fixture(r#"["abcd"]"#), Some(1))]
#[case::five_characters(fixture(r#"["abcde"]"#), Some(1))]
#[case::eight_characters(fixture(r#"["abcdefgh"]"#), Some(2))]
#[case::two_entries(fixture(r#"["abcd", "abcdefgh"]"#), Some(3))]
#[case::unicode_counts_scalars(fixture(r#"["\u00e9\u00e9\u00e9\u00e9"]"#), Some(1))]
#[case::empty_list(fixture("[]"), None)]
#[case::empty_entry(fixture(r#"[""]"#), None)]
#[case::absent_citations(Value::Null, None)]
#[case::null_entry(fixture("[null]"), None)]
#[case::zero_entry(fixture("[0]"), None)]
#[case::zero_float_entry(fixture("[0.0]"), None)]
#[case::false_entry(fixture("[false]"), None)]
#[case::empty_object_entry(fixture("[{}]"), None)]
#[case::number_entry(fixture("[12345678]"), Some(2))]
#[case::true_entry(fixture("[true]"), Some(1))]
#[case::list_entry(fixture("[[12345678]]"), Some(2))]
#[case::object_entry(fixture(r#"[{"a": 1}]"#), Some(1))]
#[case::empty_array_entry(fixture("[[]]"), None)]
#[case::three_characters(fixture(r#"["abc"]"#), Some(1))]
fn citation_tokens_are_quarter_characters(#[case] citations: Value, #[case] expected: Option<i64>) {
    let mut raw = fixture(r#"{"usage": {}}"#);
    if !citations.is_null() {
        raw["citations"] = citations;
    }
    let enhanced = enhanced_reply(
        r#"{"object":"chat.completion","choices":[]}"#,
        &raw.to_string(),
    );
    assert_eq!(enhanced["usage"]["citation_tokens"], json!(expected));
}

#[test]
fn usage_is_created_when_the_reply_has_none() {
    let enhanced = enhanced_reply(r#"{"object":"chat.completion","choices":[]}"#, "{}");
    assert_eq!(
        enhanced["usage"],
        fixture(r#"{"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0}"#)
    );
}

#[rstest]
#[case::usage_key_wins(
    fixture(r#"{"usage": {"num_search_queries": 2}, "num_search_queries": 9}"#),
    Some(2)
)]
#[case::usage_null_falls_to_root(
    fixture(r#"{"usage": {"num_search_queries": null}, "num_search_queries": 3}"#),
    Some(3)
)]
#[case::present_zero_stops_the_search(
    fixture(
        r#"{"usage": {"num_search_queries": 0}, "num_search_queries": 9, "search_queries": 8}"#
    ),
    Some(0)
)]
#[case::usage_search_queries(
    fixture(r#"{"usage": {"search_queries": 4}, "search_queries": 7}"#),
    Some(4)
)]
#[case::root_search_queries(fixture(r#"{"usage": {}, "search_queries": 5}"#), Some(5))]
#[case::root_null_falls_to_usage_search_queries(
    fixture(r#"{"usage": {"search_queries": 6}, "num_search_queries": null}"#),
    Some(6)
)]
#[case::no_count_anywhere(fixture(r#"{"usage": {}}"#), None)]
fn search_count_follows_the_key_order(#[case] raw: Value, #[case] expected: Option<i64>) {
    let enhanced = enhanced_reply(
        r#"{"object":"chat.completion","choices":[]}"#,
        &raw.to_string(),
    );
    let details = &enhanced["usage"]["prompt_tokens_details"];
    let count = expected.filter(|count| *count > 0);
    assert_eq!(details["web_search_requests"], json!(count));
    if count.is_none() {
        assert_eq!(details, &Value::Null);
    }
}

#[test]
fn annotations_become_url_citations_with_character_offsets() {
    let raw = fixture(
        r#"{
            "citations": ["https://a.example", "https://b.example"],
            "search_results": [
                {"url": "https://a.example", "title": "A"},
                {"url": "https://b.example", "title": "B"}
            ]
        }"#,
    );
    let enhanced = enhanced_reply(
        r#"{"choices":[{"message":{"role":"assistant","content":"See [1] and [2][1]."}}]}"#,
        &raw.to_string(),
    );
    assert_eq!(
        enhanced["choices"][0]["message"]["annotations"],
        fixture(
            r#"[
                {"type": "url_citation", "url_citation": {"url": "https://a.example", "title": "A", "start_index": 4, "end_index": 7}},
                {"type": "url_citation", "url_citation": {"url": "https://b.example", "title": "B", "start_index": 12, "end_index": 15}},
                {"type": "url_citation", "url_citation": {"url": "https://a.example", "title": "A", "start_index": 15, "end_index": 18}}
            ]"#,
        )
    );
}

#[test]
fn annotation_offsets_count_scalars_not_bytes() {
    let raw = fixture(r#"{"citations": ["https://a.example"]}"#);
    let enhanced = enhanced_reply(
        r#"{"choices":[{"message":{"role":"assistant","content":"\ud83d\ude42[1]"}}]}"#,
        &raw.to_string(),
    );
    let citation = &enhanced["choices"][0]["message"]["annotations"][0]["url_citation"];
    assert_eq!(citation["start_index"], json!(1_i64));
    assert_eq!(citation["end_index"], json!(4_i64));
}

#[test]
fn the_last_matching_search_result_sets_the_title() {
    let raw = fixture(
        r#"{
            "citations": ["https://a.example"],
            "search_results": [
                {"url": "https://a.example", "title": "old"},
                {"url": "https://a.example", "title": "new"}
            ]
        }"#,
    );
    let enhanced = enhanced_reply(
        r#"{"choices":[{"message":{"role":"assistant","content":"[1]"}}]}"#,
        &raw.to_string(),
    );
    assert_eq!(
        enhanced["choices"][0]["message"]["annotations"][0]["url_citation"]["title"],
        json!("new")
    );
}

#[test]
fn a_missing_search_result_title_is_empty() {
    let raw = fixture(r#"{"citations": ["https://a.example"]}"#);
    let enhanced = enhanced_reply(
        r#"{"choices":[{"message":{"role":"assistant","content":"[1]"}}]}"#,
        &raw.to_string(),
    );
    assert_eq!(
        enhanced["choices"][0]["message"]["annotations"][0]["url_citation"]["title"],
        json!("")
    );
}

#[test]
fn a_non_string_citation_uses_up_its_number() {
    let raw = fixture(r#"{"citations": [7, "https://b.example"]}"#);
    let enhanced = enhanced_reply(
        r#"{"choices":[{"message":{"role":"assistant","content":"[1][2]"}}]}"#,
        &raw.to_string(),
    );
    let annotations = &enhanced["choices"][0]["message"]["annotations"];
    assert_eq!(annotations.as_array().map(Vec::len), Some(1));
    assert_eq!(
        annotations[0]["url_citation"]["url"],
        json!("https://b.example")
    );
}

#[test]
fn leading_zero_markers_name_the_same_citation() {
    let raw = fixture(r#"{"citations": ["https://a.example"]}"#);
    let enhanced = enhanced_reply(
        r#"{"choices":[{"message":{"role":"assistant","content":"[01]"}}]}"#,
        &raw.to_string(),
    );
    assert_eq!(
        enhanced["choices"][0]["message"]["annotations"][0]["url_citation"]["url"],
        json!("https://a.example")
    );
}

#[test]
fn markers_in_non_ascii_digits_are_not_citations() {
    let raw = fixture(r#"{"citations": ["https://a.example"]}"#);
    let enhanced = enhanced_reply(
        r#"{"choices":[{"message":{"role":"assistant","content":"[\u0661]"}}]}"#,
        &raw.to_string(),
    );
    assert_eq!(
        enhanced["choices"][0]["message"]["annotations"],
        Value::Null
    );
}

#[test]
fn markers_past_the_citation_list_are_ignored() {
    let raw = fixture(r#"{"citations": ["https://a.example"]}"#);
    let enhanced = enhanced_reply(
        r#"{"choices":[{"message":{"role":"assistant","content":"[0][3]"}}]}"#,
        &raw.to_string(),
    );
    assert_eq!(
        enhanced["choices"][0]["message"]["annotations"],
        Value::Null
    );
    assert_eq!(
        enhanced["citations"],
        fixture(r#"["https://a.example"]"#),
        "a marker that resolves to nothing still copies the citations"
    );
}

#[test]
fn annotations_are_appended_to_the_existing_list() {
    let raw = fixture(r#"{"citations": ["https://a.example"]}"#);
    let existing = fixture(
        r#"[{"type": "url_citation", "url_citation": {"url": "x", "title": "x", "start_index": 0, "end_index": 1}}]"#,
    );
    let mut reply = fixture(
        r#"{"choices": [{"message": {"role": "assistant", "content": "[1]", "annotations": null}}]}"#,
    );
    reply["choices"][0]["message"]["annotations"] = existing;
    let enhanced = enhanced_reply(&reply.to_string(), &raw.to_string());
    let annotations = &enhanced["choices"][0]["message"]["annotations"];
    assert_eq!(annotations.as_array().map(Vec::len), Some(2));
    assert_eq!(
        annotations[1]["url_citation"]["url"],
        json!("https://a.example")
    );
}

#[test]
fn null_annotations_are_replaced() {
    let raw = fixture(r#"{"citations": ["https://a.example"]}"#);
    let enhanced = enhanced_reply(
        r#"{"choices":[{"message":{"role":"assistant","content":"[1]","annotations":null}}]}"#,
        &raw.to_string(),
    );
    assert_eq!(
        enhanced["choices"][0]["message"]["annotations"][0]["url_citation"]["url"],
        json!("https://a.example")
    );
}

#[test]
fn citations_and_search_results_are_copied_onto_the_reply() {
    let raw = fixture(
        r#"{
            "citations": ["https://a.example"],
            "search_results": [{"url": "https://a.example", "title": "A"}]
        }"#,
    );
    let enhanced = enhanced_reply(
        r#"{"choices":[{"message":{"role":"assistant","content":"[1]"}}]}"#,
        &raw.to_string(),
    );
    assert_eq!(enhanced["citations"], raw["citations"]);
    assert_eq!(enhanced["search_results"], raw["search_results"]);
}

#[test]
fn empty_content_skips_the_citation_copies_but_still_enhances_usage() {
    let raw = fixture(r#"{"citations": ["https://a.example"]}"#);
    let enhanced = enhanced_reply(
        r#"{"choices":[{"message":{"role":"assistant","content":""}}]}"#,
        &raw.to_string(),
    );
    assert_eq!(enhanced["citations"], Value::Null);
    assert_eq!(enhanced["search_results"], Value::Null);
    assert_eq!(
        enhanced["usage"]["citation_tokens"],
        json!(4_i64),
        "the 17-character citation counts four tokens"
    );
}

#[test]
fn no_choices_skips_the_citation_copies_but_still_enhances_usage() {
    let raw = fixture(r#"{"citations": ["https://a.example"]}"#);
    let enhanced = enhanced_reply(r#"{"choices":[]}"#, &raw.to_string());
    assert_eq!(enhanced["citations"], Value::Null);
    assert_eq!(
        enhanced["usage"]["citation_tokens"],
        json!(4_i64),
        "the 17-character citation counts four tokens"
    );
}

#[test]
fn a_missing_message_skips_the_citation_copies() {
    let raw = fixture(r#"{"citations": ["https://a.example"]}"#);
    let enhanced = enhanced_reply(
        r#"{"choices":[{"finish_reason":"stop"}]}"#,
        &raw.to_string(),
    );
    assert_eq!(enhanced["citations"], Value::Null);
    assert_eq!(
        enhanced["usage"]["citation_tokens"],
        json!(4_i64),
        "the 17-character citation counts four tokens"
    );
}

#[test]
fn a_non_object_choice_skips_the_citation_copies() {
    let raw = fixture(r#"{"citations": ["https://a.example"]}"#);
    let enhanced = enhanced_reply(r#"{"choices":[7]}"#, &raw.to_string());
    assert_eq!(enhanced["citations"], Value::Null);
}

#[rstest]
fn replays_the_perplexity_chat_reply_corpus(
    #[files("fuzz/corpus/perplexity_chat_reply/*")] path: PathBuf,
) {
    let body = fs::read(path).expect("the corpus fixture");
    let reply: Value = serde_json::from_slice(&body).unwrap_or_default();
    let _enhanced = with_perplexity_fields(reply, &body);
}

#[test]
fn invalid_raw_json_returns_the_reply_unchanged() {
    let reply = r#"{"choices":[{"message":{"role":"assistant","content":"[1]"}}]}"#;
    let enhanced = enhanced_reply(reply, "{not json");
    assert_eq!(enhanced, fixture(reply));
}

#[test]
fn a_usage_shape_failure_skips_the_annotations_too() {
    let reply = r#"{"choices":[{"message":{"role":"assistant","content":"[1]"}}]}"#;
    let raw = fixture(r#"{"usage": null, "citations": ["https://a.example"]}"#);
    let enhanced = enhanced_reply(reply, &raw.to_string());
    assert_eq!(enhanced, fixture(reply));
}

#[test]
fn a_null_search_results_fails_the_step_and_leaves_the_reply_unchanged() {
    let reply = r#"{"choices":[{"message":{"role":"assistant","content":"[1]"}}]}"#;
    let raw = fixture(r#"{"citations": ["https://a.example"], "search_results": null}"#);
    let enhanced = enhanced_reply(reply, &raw.to_string());
    assert_eq!(enhanced, fixture(reply));
}

#[test]
fn an_existing_usage_is_kept_and_extended() {
    let raw = fixture(r#"{"citations": ["abcdefgh"], "usage": {"num_search_queries": 3}}"#);
    let reply = fixture(
        r#"{
            "choices": [],
            "usage": {
                "prompt_tokens": 5,
                "completion_tokens": 6,
                "total_tokens": 11,
                "prompt_tokens_details": {"cached_tokens": 2}
            }
        }"#,
    );
    let enhanced = enhanced_reply(&reply.to_string(), &raw.to_string());
    assert_eq!(
        enhanced["usage"],
        fixture(
            r#"{
                "prompt_tokens": 5,
                "completion_tokens": 6,
                "total_tokens": 11,
                "citation_tokens": 2,
                "prompt_tokens_details": {"cached_tokens": 2, "web_search_requests": 3}
            }"#,
        )
    );
}

#[test]
fn a_null_reply_usage_is_replaced_by_a_zero_usage() {
    let raw = fixture(r#"{"citations": ["abcd"]}"#);
    let enhanced = enhanced_reply(r#"{"choices":[],"usage":null}"#, &raw.to_string());
    assert_eq!(
        enhanced["usage"],
        fixture(
            r#"{
                "prompt_tokens": 0,
                "completion_tokens": 0,
                "total_tokens": 0,
                "citation_tokens": 1,
                "prompt_tokens_details": {}
            }"#,
        )
    );
}

#[test]
fn a_non_object_reply_usage_leaves_the_reply_unchanged() {
    let reply = r#"{"choices":[],"usage":5}"#;
    let raw = fixture(r#"{"citations": ["abcd"]}"#);
    assert_eq!(enhanced_reply(reply, &raw.to_string()), fixture(reply));
}

#[test]
fn citation_tokens_alone_create_empty_prompt_token_details() {
    let raw = fixture(r#"{"citations": ["abcd"], "usage": {"num_search_queries": 0}}"#);
    let enhanced = enhanced_reply(r#"{"choices":[]}"#, &raw.to_string());
    assert_eq!(enhanced["usage"]["prompt_tokens_details"], fixture("{}"));
}

#[rstest]
#[case::zero(json!(0_i64))]
#[case::empty_text(json!(""))]
#[case::empty_object(json!({}))]
#[case::false_flag(json!(false))]
#[case::number(json!(5_i64))]
#[case::text(json!("x"))]
fn non_list_choices_fail_with_the_field_and_leave_the_reply_unchanged(#[case] choices: Value) {
    let mut reply = fixture(r#"{"choices": null}"#);
    reply["choices"] = choices;
    let original = reply.clone();
    let error =
        apply(&mut reply, br#"{"citations": ["abcd"]}"#).expect_err("the choices shape failure");
    assert!(matches!(
        error,
        PerplexityReplyError::Shape { field: "choices" }
    ));
    assert_eq!(reply, original);
}

#[test]
fn a_search_result_without_a_title_is_ignored() {
    let raw = fixture(
        r#"{
            "citations": ["https://a.example"],
            "search_results": [{"url": "https://a.example"}]
        }"#,
    );
    let enhanced = enhanced_reply(
        r#"{"choices":[{"message":{"role":"assistant","content":"[1]"}}]}"#,
        &raw.to_string(),
    );
    assert_eq!(
        enhanced["choices"][0]["message"]["annotations"][0]["url_citation"]["title"],
        json!("")
    );
}

#[test]
fn non_list_existing_annotations_leave_the_reply_unchanged() {
    let reply =
        r#"{"choices":[{"message":{"role":"assistant","content":"[1]","annotations":"x"}}]}"#;
    let raw = fixture(r#"{"citations": ["https://a.example"]}"#);
    assert_eq!(enhanced_reply(reply, &raw.to_string()), fixture(reply));
}
