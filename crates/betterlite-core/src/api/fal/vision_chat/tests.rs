#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use betterlite_openai::api::image_contracts::records::ImageHeaders;
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat::response::ChatResponse;
use rstest::rstest;
use secrecy::SecretString;

use super::error::FalAIError;
use super::headers::{complete_url, validate_environment};
use super::optional_params::{Params, SUPPORTED_OPENAI_PARAMS, map_openai_params};
use super::reply::{decode_fal_chat, transform_response};
use super::request::transform_request;
use crate::api::fal::images::settings::FalImageSettings;

fn settings(api_base: &str, api_key: Option<&str>) -> FalImageSettings {
    FalImageSettings {
        api_key: api_key.map(|key| SecretString::from(key.to_owned())),
        api_base: api_base.to_owned(),
    }
}

fn headers(entries: &[(&str, &str)]) -> ImageHeaders {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn params(text: &str) -> Params {
    serde_json::from_str(text).expect("a parameter object")
}

fn user_message(content: &str) -> ChatMessage {
    message("user", content)
}

fn message(role: &str, content: &str) -> ChatMessage {
    let text = format!(r#"{{"role": "{role}", "content": {content}}}"#);
    serde_json::from_str(&text).expect("a chat message")
}

fn parts(entries: &str) -> ChatMessage {
    user_message(&format!("[{entries}]"))
}

fn response(choices: usize) -> ChatResponse {
    let choice = r#"{"index": 0, "message": {"role": "assistant", "content": "old"}, "finish_reason": "length"}"#;
    let listed = vec![choice; choices].join(",");
    let text = format!(
        r#"{{"id": "chatcmpl-1", "object": "chat.completion", "created": 1, "model": "old-model", "choices": [{listed}], "usage": {{"prompt_tokens": 9, "completion_tokens": 9, "total_tokens": 18}}, "system_fingerprint": "fp"}}"#
    );
    serde_json::from_str(&text).expect("a chat response")
}

fn reply_body(usage: &str) -> String {
    format!(r#"{{"output": "a cat", "usage_info": {usage}}}"#)
}

fn counts(input: &str, output: &str) -> String {
    reply_body(&format!(
        r#"{{"input_tokens": {input}, "output_tokens": {output}}}"#
    ))
}

#[rstest]
#[case::explicit_wins(
    "https://explicit.test///",
    "https://env.test",
    "fal-ai/m",
    "https://explicit.test/fal-ai/m"
)]
#[case::empty_explicit_falls_back("", "https://env.test/", "fal-ai/m", "https://env.test/fal-ai/m")]
#[case::default_base("", "https://fal.run", "fal-ai/m", "https://fal.run/fal-ai/m")]
#[case::prefix_removed_once("", "https://fal.run", "fal_ai/fal_ai/m", "https://fal.run/fal_ai/m")]
#[case::prefix_only_at_start("", "https://fal.run", "x/fal_ai/m", "https://fal.run/x/fal_ai/m")]
#[case::no_second_namespace(
    "",
    "https://fal.run",
    "fal_ai/fal-ai/m/query",
    "https://fal.run/fal-ai/m/query"
)]
#[case::empty_endpoint_keeps_slash("", "https://fal.run", "fal_ai/", "https://fal.run/")]
fn complete_url_joins_base_and_endpoint(
    #[case] explicit: &str,
    #[case] configured: &str,
    #[case] model: &str,
    #[case] expected: &str,
) {
    let explicit = (!explicit.is_empty()).then_some(explicit);
    let url = complete_url(&settings(configured, None), explicit, model);
    assert_eq!(url, expected);
}

#[test]
fn complete_url_treats_a_missing_explicit_base_as_the_configured_one() {
    let url = complete_url(&settings("https://env.test//", None), None, "fal_ai/m");
    assert_eq!(url, "https://env.test/m");
}

#[test]
fn headers_default_to_json_and_use_the_key_scheme() {
    let merged =
        validate_environment(&settings("", Some("env-key")), &headers(&[]), None).expect("headers");
    assert_eq!(
        merged,
        headers(&[
            ("Authorization", "Key env-key"),
            ("content-type", "application/json")
        ])
    );
}

#[test]
fn a_caller_content_type_overrides_the_default() {
    let caller = headers(&[("content-type", "application/x-ndjson")]);
    let merged = validate_environment(&settings("", Some("k")), &caller, None).expect("headers");
    assert_eq!(
        merged.get("content-type").map(String::as_str),
        Some("application/x-ndjson")
    );
}

#[test]
fn a_differently_cased_content_type_sits_beside_the_default() {
    let caller = headers(&[("Content-Type", "text/plain")]);
    let merged = validate_environment(&settings("", Some("k")), &caller, None).expect("headers");
    assert_eq!(
        merged.get("content-type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        merged.get("Content-Type").map(String::as_str),
        Some("text/plain")
    );
}

#[test]
fn authorization_is_written_last() {
    let caller = headers(&[("Authorization", "Bearer other"), ("x-request-id", "r1")]);
    let merged =
        validate_environment(&settings("", Some("env-key")), &caller, None).expect("headers");
    assert_eq!(
        merged.get("Authorization").map(String::as_str),
        Some("Key env-key")
    );
    assert_eq!(merged.get("x-request-id").map(String::as_str), Some("r1"));
}

#[rstest]
#[case::explicit_wins(Some("explicit"), "Key explicit")]
#[case::empty_explicit_falls_back(Some(""), "Key env-key")]
#[case::absent_explicit_falls_back(None, "Key env-key")]
fn the_explicit_key_wins_when_it_is_not_empty(
    #[case] explicit: Option<&str>,
    #[case] expected: &str,
) {
    let explicit = explicit.map(|key| SecretString::from(key.to_owned()));
    let merged = validate_environment(
        &settings("", Some("env-key")),
        &headers(&[]),
        explicit.as_ref(),
    )
    .expect("headers");
    assert_eq!(
        merged.get("Authorization").map(String::as_str),
        Some(expected)
    );
}

#[rstest]
#[case::no_key(None, None)]
#[case::empty_environment_key(Some(""), Some(""))]
fn a_missing_key_is_refused(#[case] configured: Option<&str>, #[case] explicit: Option<&str>) {
    let explicit = explicit.map(|key| SecretString::from(key.to_owned()));
    let error = validate_environment(&settings("", configured), &headers(&[]), explicit.as_ref())
        .expect_err("no key");
    assert!(matches!(error, FalAIError::MissingApiKey(_)));
    assert_eq!(error.status(), None);
}

#[test]
fn the_supported_parameters_are_reasoning_effort_temperature_and_top_p() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        ["reasoning_effort", "temperature", "top_p"]
    );
}

#[rstest]
#[case::none("none", false)]
#[case::minimal("minimal", false)]
#[case::low("low", true)]
#[case::medium("medium", true)]
#[case::high("high", true)]
fn reasoning_effort_maps_to_a_boolean_even_when_false(
    #[case] effort: &str,
    #[case] expected: bool,
) {
    let input = params(&format!(r#"{{"reasoning_effort": "{effort}"}}"#));
    let mapped = map_openai_params(&input, Params::new(), "m", false).expect("mapped");
    assert_eq!(mapped, params(&format!(r#"{{"reasoning": {expected}}}"#)));
}

#[rstest]
#[case::unknown_text(r#""xhigh""#)]
#[case::wrong_case(r#""High""#)]
#[case::empty(r#""""#)]
#[case::number("1")]
#[case::null("null")]
#[case::boolean("true")]
fn an_unsupported_reasoning_effort_is_a_400_unless_params_are_dropped(#[case] effort: &str) {
    let input = params(&format!(
        r#"{{"reasoning_effort": {effort}, "temperature": 0.5}}"#
    ));
    let error = map_openai_params(&input, Params::new(), "fal_ai/m", false).expect_err("rejected");
    assert_eq!(error.status(), Some(400));
    assert!(error.to_string().contains("fal_ai/m"));
    let dropped =
        map_openai_params(&input, params(r#"{"top_p": 1}"#), "fal_ai/m", true).expect("dropped");
    assert_eq!(dropped, params(r#"{"top_p": 1, "temperature": 0.5}"#));
}

#[test]
fn mapped_values_overwrite_optional_ones_in_place() {
    let input = params(r#"{"reasoning_effort": "none", "temperature": 0, "top_p": null}"#);
    let optional = params(r#"{"temperature": 1.5, "reasoning": true, "seed": 3, "top_p": 0.9}"#);
    let mapped = map_openai_params(&input, optional, "m", false).expect("mapped");
    let expected = params(r#"{"temperature": 0, "reasoning": false, "seed": 3, "top_p": null}"#);
    assert_eq!(mapped, expected);
    assert_eq!(
        mapped.keys().map(String::as_str).collect::<Vec<_>>(),
        ["temperature", "reasoning", "seed", "top_p"]
    );
}

#[test]
fn unrelated_input_parameters_are_ignored() {
    let input = params(r#"{"max_tokens": 5, "stream": true, "n": 2}"#);
    let mapped = map_openai_params(&input, params(r#"{"seed": 1}"#), "m", false).expect("mapped");
    assert_eq!(mapped, params(r#"{"seed": 1}"#));
}

#[test]
fn a_valid_request_has_the_prompt_the_image_and_the_kept_options() {
    let messages = [parts(
        r#"{"type": "text", "text": "what is this"}, {"type": "image_url", "image_url": {"url": "https://img.test/a.png"}}"#,
    )];
    let options = params(
        r#"{"temperature": 0.2, "seed": 4, "stream": false, "top_p": null, "reasoning": false}"#,
    );
    let body = transform_request(&messages, &options).expect("body");
    assert_eq!(body.prompt, "what is this");
    assert_eq!(body.image_url, "https://img.test/a.png");
    let wire = serde_json::to_string(&body).expect("wire");
    assert_eq!(
        wire,
        r#"{"prompt":"what is this","image_url":"https://img.test/a.png","temperature":0.2,"reasoning":false}"#
    );
}

#[test]
fn only_the_three_options_survive_the_projection() {
    let messages = [parts(
        r#"{"type": "text", "text": "t"}, {"type": "image_url", "image_url": "u"}"#,
    )];
    let options = params(r#"{"max_tokens": 3, "prompt": "x", "image_url": "y", "n": 1}"#);
    let body = transform_request(&messages, &options).expect("body");
    assert_eq!(
        serde_json::to_string(&body).expect("wire"),
        r#"{"prompt":"t","image_url":"u"}"#
    );
}

#[rstest]
#[case::true_flag("true")]
#[case::string_flag(r#""yes""#)]
#[case::number_flag("1")]
fn a_stream_option_fails_before_the_messages_are_read(#[case] flag: &str) {
    let options = params(&format!(r#"{{"stream": {flag}}}"#));
    let error = transform_request(&[], &options).expect_err("stream");
    assert_eq!(error.status(), Some(400));
    assert!(!matches!(error, FalAIError::OneUserMessage));
}

#[test]
fn a_true_stream_is_the_streaming_error_and_other_values_are_not() {
    let messages = [parts(
        r#"{"type": "text", "text": "t"}, {"type": "image_url", "image_url": "u"}"#,
    )];
    let streaming =
        transform_request(&messages, &params(r#"{"stream": true}"#)).expect_err("stream");
    assert!(matches!(streaming, FalAIError::StreamingUnsupported));
    let invalid =
        transform_request(&messages, &params(r#"{"stream": "yes"}"#)).expect_err("invalid");
    assert!(matches!(invalid, FalAIError::StreamNotBoolean));
    assert!(transform_request(&messages, &params(r#"{"stream": false}"#)).is_ok());
    assert!(transform_request(&messages, &params(r#"{"stream": null}"#)).is_ok());
}

#[rstest]
#[case::no_messages(vec![])]
#[case::two_messages(vec![user_message(r#""a""#), user_message(r#""b""#)])]
#[case::system_role(vec![message("system", r#""a""#)])]
#[case::assistant_role(vec![message("assistant", r#""a""#)])]
#[case::developer_role(vec![message("developer", r#""a""#)])]
fn exactly_one_user_message_is_required(#[case] messages: Vec<ChatMessage>) {
    let error = transform_request(&messages, &Params::new()).expect_err("rejected");
    assert!(matches!(error, FalAIError::OneUserMessage));
    assert_eq!(error.status(), Some(400));
}

#[rstest]
#[case::empty_string(r#""""#, true)]
#[case::plain_text(r#""describe""#, false)]
#[case::null("null", true)]
#[case::empty_list("[]", true)]
fn plain_content_never_carries_an_image(#[case] content: &str, #[case] missing_text: bool) {
    let error = transform_request(&[user_message(content)], &Params::new()).expect_err("rejected");
    assert_eq!(matches!(error, FalAIError::TextRequired), missing_text);
    assert_eq!(matches!(error, FalAIError::OneImageRequired), !missing_text);
}

#[rstest]
#[case::two_texts(
    r#"{"type": "text", "text": "a"}, {"type": "text", "text": "b"}, {"type": "image_url", "image_url": "u"}"#,
    "a\nb"
)]
#[case::empty_texts_skipped(
    r#"{"type": "text", "text": ""}, {"type": "text", "text": "a"}, {"type": "text", "text": ""}, {"type": "image_url", "image_url": "u"}"#,
    "a"
)]
#[case::non_string_text_skipped(
    r#"{"type": "text", "text": 5}, {"type": "text", "text": null}, {"type": "text"}, {"type": "text", "text": "a"}, {"type": "image_url", "image_url": "u"}"#,
    "a"
)]
#[case::other_parts_ignored(
    r#"{"type": "audio", "text": "no"}, {"type": "text", "text": "a"}, {"type": "image_url", "image_url": "u"}"#,
    "a"
)]
#[case::text_after_image(
    r#"{"type": "image_url", "image_url": "u"}, {"type": "text", "text": "a"}, {"type": "text", "text": "b"}"#,
    "a\nb"
)]
fn the_prompt_joins_nonempty_text_parts_with_newlines(#[case] entries: &str, #[case] prompt: &str) {
    let body = transform_request(&[parts(entries)], &Params::new()).expect("body");
    assert_eq!(body.prompt, prompt);
    assert_eq!(body.image_url, "u");
}

#[rstest]
#[case::string_value(r#""https://a.test/1.png""#, "https://a.test/1.png")]
#[case::mapping_value(
    r#"{"url": "https://a.test/2.png", "detail": "low"}"#,
    "https://a.test/2.png"
)]
#[case::empty_string_is_an_image(r#""""#, "")]
#[case::empty_url_is_an_image(r#"{"url": ""}"#, "")]
fn image_urls_pass_through_untouched(#[case] image: &str, #[case] expected: &str) {
    let entries = format!(
        r#"{{"type": "text", "text": "t"}}, {{"type": "image_url", "image_url": {image}}}"#
    );
    let body = transform_request(&[parts(&entries)], &Params::new()).expect("body");
    assert_eq!(body.image_url, expected);
}

#[rstest]
#[case::number("5")]
#[case::null("null")]
#[case::boolean("true")]
#[case::list(r#"["u"]"#)]
#[case::mapping_without_url(r#"{"detail": "low"}"#)]
#[case::non_string_url(r#"{"url": 3}"#)]
#[case::null_url(r#"{"url": null}"#)]
fn a_malformed_image_member_is_not_an_image_part(#[case] image: &str) {
    let entries = format!(
        r#"{{"type": "text", "text": "t"}}, {{"type": "image_url", "image_url": {image}}}"#
    );
    let error = transform_request(&[parts(&entries)], &Params::new()).expect_err("no image");
    assert!(matches!(error, FalAIError::OneImageRequired));
}

#[test]
fn a_part_without_an_image_url_key_is_not_an_image_part() {
    let entries = r#"{"type": "text", "text": "t"}, {"type": "image_url"}"#;
    let error = transform_request(&[parts(entries)], &Params::new()).expect_err("no image");
    assert!(matches!(error, FalAIError::OneImageRequired));
}

#[test]
fn a_malformed_image_does_not_count_beside_a_good_one() {
    let entries = r#"{"type": "text", "text": "t"}, {"type": "image_url", "image_url": 1}, {"type": "image_url", "image_url": "u"}"#;
    let body = transform_request(&[parts(entries)], &Params::new()).expect("body");
    assert_eq!(body.image_url, "u");
}

#[test]
fn two_images_are_refused() {
    let entries = r#"{"type": "text", "text": "t"}, {"type": "image_url", "image_url": "a"}, {"type": "image_url", "image_url": {"url": "b"}}"#;
    let error = transform_request(&[parts(entries)], &Params::new()).expect_err("two images");
    assert!(matches!(error, FalAIError::OneImageRequired));
}

#[test]
fn the_text_error_wins_when_text_and_image_are_both_missing() {
    let error = transform_request(
        &[parts(r#"{"type": "image_url", "image_url": 1}"#)],
        &Params::new(),
    )
    .expect_err("rejected");
    assert!(matches!(error, FalAIError::TextRequired));
    let none = transform_request(&[parts(r#"{"type": "text", "text": ""}"#)], &Params::new())
        .expect_err("rejected");
    assert!(matches!(none, FalAIError::TextRequired));
}

#[test]
fn a_decoded_reply_carries_the_counts_and_optional_fields() {
    let body = r#"{"output": "a cat", "usage_info": {"input_tokens": 12, "output_tokens": 3, "total": 99}, "reasoning": "saw one", "finish_reason": "stop", "extra": [1]}"#;
    let reply = decode_fal_chat(&headers(&[]), body.as_bytes()).expect("reply");
    assert_eq!(reply.output, "a cat");
    assert_eq!(reply.reasoning.as_deref(), Some("saw one"));
    assert_eq!(reply.finish_reason.as_deref(), Some("stop"));
    assert_eq!(
        (
            reply.prompt_tokens,
            reply.completion_tokens,
            reply.total_tokens
        ),
        (12, 3, 15)
    );
}

#[test]
fn reasoning_and_finish_reason_are_optional_and_nullable() {
    let absent = decode_fal_chat(&headers(&[]), counts("1", "2").as_bytes()).expect("absent");
    assert_eq!((absent.reasoning, absent.finish_reason), (None, None));
    let body = r#"{"output": "", "usage_info": {"input_tokens": 1, "output_tokens": 2}, "reasoning": null, "finish_reason": null}"#;
    let nulls = decode_fal_chat(&headers(&[]), body.as_bytes()).expect("nulls");
    assert_eq!(
        (nulls.output.as_str(), nulls.reasoning, nulls.finish_reason),
        ("", None, None)
    );
}

#[rstest]
#[case::integer("7", 7)]
#[case::zero("0", 0)]
#[case::negative_zero("-0", 0)]
#[case::integral_float("2.0", 2)]
#[case::exponent("1e2", 100)]
#[case::capital_exponent("1.0E+1", 10)]
#[case::boolean_true("true", 1)]
#[case::boolean_false("false", 0)]
#[case::numeric_string(r#""7""#, 7)]
#[case::padded_string(r#"" 7 ""#, 7)]
#[case::signed_string(r#""+7""#, 7)]
#[case::underscored_string(r#""1_000""#, 1000)]
#[case::leading_zeros(r#""00_1""#, 1)]
#[case::zero_fraction_string(r#""2.00""#, 2)]
#[case::negative_zero_string(r#""-0""#, 0)]
#[case::unicode_space_string("\"\\u00a05\"", 5)]
#[case::underscored_with_fraction(r#""1_0.0""#, 10)]
#[case::largest_count("18446744073709551615", u64::MAX)]
#[case::largest_count_string(r#""18446744073709551615""#, u64::MAX)]
fn counts_are_read_the_way_pydantic_reads_an_int(#[case] raw: &str, #[case] expected: u64) {
    let reply = decode_fal_chat(&headers(&[]), counts(raw, "0").as_bytes()).expect("reply");
    assert_eq!(reply.prompt_tokens, expected);
}

#[rstest]
#[case::fraction("2.5")]
#[case::negative("-3")]
#[case::negative_string(r#""-3""#)]
#[case::one_past_the_largest("18446744073709551616")]
#[case::one_past_the_largest_string(r#""18446744073709551616""#)]
#[case::huge_exponent("1e400")]
#[case::exponent_string(r#""1e2""#)]
#[case::fractional_string(r#""1.5""#)]
#[case::bare_dot_string(r#""1.""#)]
#[case::leading_dot_string(r#"".0""#)]
#[case::empty_string(r#""""#)]
#[case::blank_string(r#""   ""#)]
#[case::word(r#""many""#)]
#[case::inner_space(r#""1 0""#)]
#[case::double_underscore(r#""1__0""#)]
#[case::trailing_underscore(r#""1_""#)]
#[case::underscored_fraction(r#""1.0_0""#)]
#[case::double_sign(r#""+-5""#)]
#[case::arabic_digits("\"\\u0663\"")]
#[case::null("null")]
#[case::list("[1]")]
#[case::object(r#"{"n": 1}"#)]
fn counts_pydantic_rejects_are_a_422(#[case] raw: &str) {
    let error = decode_fal_chat(&headers(&[]), counts(raw, "0").as_bytes()).expect_err("rejected");
    assert!(matches!(error, FalAIError::UnexpectedResponse { .. }));
    assert_eq!(error.status(), Some(422));
}

#[rstest]
#[case::missing_usage(r#"{"output": "a"}"#)]
#[case::null_usage(r#"{"output": "a", "usage_info": null}"#)]
#[case::empty_output_still_needs_usage(r#"{"output": ""}"#)]
#[case::missing_input(r#"{"output": "a", "usage_info": {"output_tokens": 1}}"#)]
#[case::missing_output_tokens(r#"{"output": "a", "usage_info": {"input_tokens": 1}}"#)]
#[case::missing_output(r#"{"usage_info": {"input_tokens": 1, "output_tokens": 1}}"#)]
#[case::numeric_output(r#"{"output": 5, "usage_info": {"input_tokens": 1, "output_tokens": 1}}"#)]
#[case::null_output(r#"{"output": null, "usage_info": {"input_tokens": 1, "output_tokens": 1}}"#)]
#[case::numeric_reasoning(
    r#"{"output": "a", "reasoning": 5, "usage_info": {"input_tokens": 1, "output_tokens": 1}}"#
)]
#[case::boolean_finish(r#"{"output": "a", "finish_reason": true, "usage_info": {"input_tokens": 1, "output_tokens": 1}}"#)]
#[case::duplicate_key(
    r#"{"output": "a", "output": "b", "usage_info": {"input_tokens": 1, "output_tokens": 1}}"#
)]
#[case::trailing_text(
    r#"{"output": "a", "usage_info": {"input_tokens": 1, "output_tokens": 1}} x"#
)]
#[case::array_root(r#"[{"output": "a"}]"#)]
#[case::null_root("null")]
#[case::not_json("not json at all")]
#[case::empty_body("")]
fn an_invalid_reply_is_a_422_with_the_body_and_headers(#[case] body: &str) {
    let sent = headers(&[("x-request-id", "req-1")]);
    let error = decode_fal_chat(&sent, body.as_bytes()).expect_err("rejected");
    assert_eq!(error.status(), Some(422));
    assert_eq!(error.headers(), Some(&sent));
    assert_eq!(
        error.to_string(),
        format!("fal_ai returned an unexpected response body: {body}")
    );
}

#[test]
fn malformed_utf8_is_replaced_in_the_error_text() {
    let body = b"{\"output\": \"bad \xff\xfe\"";
    let error = decode_fal_chat(&headers(&[]), body).expect_err("rejected");
    assert_eq!(
        error.to_string(),
        "fal_ai returned an unexpected response body: {\"output\": \"bad \u{FFFD}\u{FFFD}\""
    );
}

#[test]
fn the_total_may_reach_the_largest_count_but_not_pass_it() {
    let at_limit = decode_fal_chat(
        &headers(&[]),
        counts("18446744073709551614", "1").as_bytes(),
    )
    .expect("at the limit");
    assert_eq!(at_limit.total_tokens, u64::MAX);
    let single = decode_fal_chat(
        &headers(&[]),
        counts("18446744073709551615", "0").as_bytes(),
    )
    .expect("single");
    assert_eq!(single.total_tokens, u64::MAX);
    let sent = headers(&[("x-request-id", "req-2")]);
    let body = counts("18446744073709551615", "1");
    let error = decode_fal_chat(&sent, body.as_bytes()).expect_err("overflow");
    assert!(matches!(error, FalAIError::UsageOverflow { .. }));
    assert_eq!(error.status(), Some(422));
    assert_eq!(error.headers(), Some(&sent));
    assert_eq!(
        error.to_string(),
        format!("fal_ai returned an unexpected response body: {body}")
    );
}

#[test]
fn the_first_choice_is_rewritten_and_the_rest_is_left_alone() {
    let body = r#"{"output": "a cat", "usage_info": {"input_tokens": 12, "output_tokens": 3}, "reasoning": "saw one", "finish_reason": "length"}"#;
    let before = response(2);
    let completion = transform_response(
        "fal_ai/m/query",
        &headers(&[]),
        body.as_bytes(),
        before.clone(),
        1_700_000_000,
    )
    .expect("completion");
    let after = completion.response;
    assert_eq!(completion.reasoning_content.as_deref(), Some("saw one"));
    let first = after.choices.first().expect("first choice");
    assert_eq!(first.message.role, "assistant");
    assert_eq!(first.message.content.as_deref(), Some("a cat"));
    assert_eq!(first.finish_reason.as_deref(), Some("length"));
    assert_eq!(after.choices.get(1), before.choices.get(1));
    assert_eq!(after.id, before.id);
    assert_eq!(after.object, before.object);
    assert_eq!(after.system_fingerprint, before.system_fingerprint);
    assert_eq!(after.created, 1_700_000_000);
    assert_eq!(after.model, "fal_ai/m/query");
    assert_eq!(
        (
            after.usage.prompt_tokens,
            after.usage.completion_tokens,
            after.usage.total_tokens
        ),
        (12, 3, 15)
    );
}

#[rstest]
#[case::absent(None, "stop")]
#[case::empty(Some(""), "stop")]
#[case::stop(Some("stop"), "stop")]
#[case::length(Some("length"), "length")]
#[case::tool_calls(Some("tool_calls"), "tool_calls")]
#[case::content_filter(Some("content_filter"), "content_filter")]
#[case::mapped_alias(Some("end_turn"), "stop")]
#[case::unknown(Some("something_new"), "stop")]
fn the_finish_reason_goes_through_the_shared_mapping(
    #[case] reason: Option<&str>,
    #[case] expected: &str,
) {
    let finish = reason.map_or_else(String::new, |reason| {
        format!(r#", "finish_reason": "{reason}""#)
    });
    let body = format!(
        r#"{{"output": "x", "usage_info": {{"input_tokens": 1, "output_tokens": 1}}{finish}}}"#
    );
    let completion = transform_response("m", &headers(&[]), body.as_bytes(), response(1), 5)
        .expect("completion");
    let choice = completion.response.choices.first().expect("choice");
    assert_eq!(choice.finish_reason.as_deref(), Some(expected));
}

#[test]
fn a_null_finish_reason_and_missing_reasoning_leave_no_reasoning_content() {
    let body = r#"{"output": "x", "usage_info": {"input_tokens": 1, "output_tokens": 1}, "finish_reason": null}"#;
    let completion = transform_response("m", &headers(&[]), body.as_bytes(), response(1), 5)
        .expect("completion");
    assert_eq!(completion.reasoning_content, None);
    let first = completion.response.choices.first().expect("choice");
    assert_eq!(first.finish_reason.as_deref(), Some("stop"));
}

#[test]
fn empty_reasoning_is_kept() {
    let body =
        r#"{"output": "", "usage_info": {"input_tokens": 0, "output_tokens": 0}, "reasoning": ""}"#;
    let completion = transform_response("m", &headers(&[]), body.as_bytes(), response(1), 5)
        .expect("completion");
    assert_eq!(completion.reasoning_content.as_deref(), Some(""));
    let first = completion.response.choices.first().expect("choice");
    assert_eq!(first.message.content.as_deref(), Some(""));
}

#[test]
fn an_invalid_reply_leaves_the_response_to_the_caller() {
    let error =
        transform_response("m", &headers(&[]), b"{}", response(1), 5).expect_err("rejected");
    assert_eq!(error.status(), Some(422));
}

#[test]
fn an_upstream_error_keeps_the_status_message_and_headers() {
    let sent = headers(&[("retry-after", "3")]);
    let error = FalAIError::Upstream {
        status: 429,
        message: "slow down".to_owned(),
        headers: sent.clone(),
    };
    assert_eq!(error.status(), Some(429));
    assert_eq!(error.headers(), Some(&sent));
    assert_eq!(error.to_string(), "slow down");
}

#[test]
fn a_response_without_choices_is_an_error_not_a_panic() {
    let error = transform_response(
        "m",
        &headers(&[]),
        counts("1", "1").as_bytes(),
        response(0),
        5,
    )
    .expect_err("no choice");
    assert!(matches!(error, FalAIError::NoChoices));
    assert_eq!(error.status(), None);
}

#[rstest]
fn replays_the_fal_chat_corpus(#[files("fuzz/corpus/fal_chat_reply/*")] path: PathBuf) {
    let bytes = fs::read(path).expect("a corpus file");
    drop(decode_fal_chat(&BTreeMap::new(), &bytes));
}
