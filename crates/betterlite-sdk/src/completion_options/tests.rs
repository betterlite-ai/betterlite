#![cfg(test)]

use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::controls::control_options::ControlOptions;
use betterlite_models::decisions::Json;
use rstest::rstest;
use serde_json::Number;

use super::control::resolve_control_options;
use super::error::CompletionOptionsError;
use super::headers::merge_request_headers;
use super::tool_keys::{drop_anthropic_only_tool_keys, is_claude_tool_target};
use crate::call_setup::RequestMap;

const SIZE_MESSAGE: &str =
    "invalid stream_chunk_size: expected a positive integer of at most 18 digits";

fn request(text: &str) -> RequestMap {
    serde_json::from_str(text).unwrap()
}

fn headers(text: &str) -> AdditionalHeaders {
    serde_json::from_str(text).unwrap()
}

fn tools(text: &str) -> Vec<Json> {
    serde_json::from_str(text).unwrap()
}

#[rstest]
#[case::absent("{}", None)]
#[case::null(r#"{"stream_chunk_size":null}"#, None)]
#[case::number(r#"{"stream_chunk_size":5}"#, Some(5))]
#[case::digit_text(r#"{"stream_chunk_size":"42"}"#, Some(42))]
#[case::largest_allowed(
    r#"{"stream_chunk_size":999999999999999999}"#,
    Some(999_999_999_999_999_999)
)]
#[case::valid_wins_over_drop_params(r#"{"stream_chunk_size":3,"drop_params":true}"#, Some(3))]
fn control_a_valid_option_is_returned_as_is(#[case] kwargs: &str, #[case] size: Option<u64>) {
    let control = resolve_control_options(&request(kwargs), "gpt-4o", None).unwrap();
    assert_eq!(
        control,
        ControlOptions {
            stream_chunk_size: size
        }
    );
}

#[rstest]
#[case::zero(r#"{"stream_chunk_size":0}"#)]
#[case::negative(r#"{"stream_chunk_size":-4}"#)]
#[case::fraction(r#"{"stream_chunk_size":1.5}"#)]
#[case::limit(r#"{"stream_chunk_size":1000000000000000000}"#)]
#[case::bool(r#"{"stream_chunk_size":true}"#)]
#[case::list(r#"{"stream_chunk_size":[1]}"#)]
#[case::object(r#"{"stream_chunk_size":{"a":1}}"#)]
#[case::text(r#"{"stream_chunk_size":"abc"}"#)]
#[case::empty_text(r#"{"stream_chunk_size":""}"#)]
#[case::nineteen_digits(r#"{"stream_chunk_size":"1000000000000000000"}"#)]
#[case::zero_text(r#"{"stream_chunk_size":"0"}"#)]
fn control_an_invalid_option_fails_with_the_param_in_the_body(#[case] kwargs: &str) {
    let error = resolve_control_options(&request(kwargs), "gpt-4o", None).unwrap_err();
    let CompletionOptionsError::InvalidControlOption(details) = error;
    assert_eq!(details.message, SIZE_MESSAGE);
    assert_eq!(details.model.as_deref(), Some("gpt-4o"));
    assert_eq!(details.provider, None);
    assert_eq!(details.body_param.as_deref(), Some("stream_chunk_size"));
    assert_eq!(details.body_type, None);
    assert_eq!(details.body_code, None);
    assert_eq!(details.status.0, 400);
}

#[rstest]
#[case::global_true(Some(true), "{}", true)]
#[case::global_true_beats_call_false(Some(true), r#"{"drop_params":false}"#, true)]
#[case::call_true(None, r#"{"drop_params":true}"#, true)]
#[case::call_true_with_global_false(Some(false), r#"{"drop_params":true}"#, true)]
#[case::call_text_true(None, r#"{"drop_params":"true"}"#, true)]
#[case::call_one(None, r#"{"drop_params":1}"#, true)]
#[case::neither(None, "{}", false)]
#[case::both_false(Some(false), r#"{"drop_params":false}"#, false)]
#[case::call_null(None, r#"{"drop_params":null}"#, false)]
#[case::call_zero(None, r#"{"drop_params":0}"#, false)]
#[case::call_text_false(None, r#"{"drop_params":"false"}"#, false)]
#[case::call_unreadable_text(None, r#"{"drop_params":"maybe"}"#, false)]
#[case::call_list(None, r#"{"drop_params":[true]}"#, false)]
fn control_drop_params_turns_the_failure_into_defaults(
    #[case] global: Option<bool>,
    #[case] extra: &str,
    #[case] dropped: bool,
) {
    let mut kwargs = request(extra);
    kwargs.insert(
        "stream_chunk_size".to_owned(),
        Json::Number(Number::from(0_i32)),
    );
    let result = resolve_control_options(&kwargs, "gpt-4o", global);
    if dropped {
        assert_eq!(result.unwrap(), ControlOptions::default());
    } else {
        assert!(matches!(
            result,
            Err(CompletionOptionsError::InvalidControlOption(_))
        ));
    }
}

#[rstest]
#[case::headers_only(Some(r#"{"a":"1"}"#), None, "{}", r#"{"a":"1"}"#)]
#[case::extra_only(None, Some(r#"{"b":"2"}"#), "{}", r#"{"b":"2"}"#)]
#[case::nothing(None, None, "{}", "{}")]
#[case::empty_headers_fall_through(Some("{}"), Some(r#"{"b":"2"}"#), "{}", r#"{"b":"2"}"#)]
#[case::both_empty(Some("{}"), Some("{}"), "{}", "{}")]
#[case::extra_wins_over_headers(
    Some(r#"{"a":"1","b":"1"}"#),
    Some(r#"{"b":"2","c":"2"}"#),
    "{}",
    r#"{"a":"1","b":"2","c":"2"}"#
)]
#[case::proxy_wins_over_both(
    Some(r#"{"a":"1"}"#),
    Some(r#"{"a":"2","b":"2"}"#),
    r#"{"a":"3"}"#,
    r#"{"a":"3","b":"2"}"#
)]
#[case::proxy_adds_its_own(Some(r#"{"a":"1"}"#), None, r#"{"p":"x"}"#, r#"{"a":"1","p":"x"}"#)]
#[case::proxy_alone(None, None, r#"{"p":"x"}"#, r#"{"p":"x"}"#)]
fn headers_extra_wins_over_headers_and_proxy_wins_over_both(
    #[case] given: Option<&str>,
    #[case] extra: Option<&str>,
    #[case] proxy: &str,
    #[case] expected: &str,
) {
    let merged = merge_request_headers(
        given.map(headers).as_ref(),
        extra.map(headers).as_ref(),
        &headers(proxy),
    );
    assert_eq!(merged, headers(expected));
}

#[test]
fn headers_an_overridden_key_keeps_its_place() {
    let merged = merge_request_headers(
        Some(&headers(r#"{"a":"1","b":"1"}"#)),
        Some(&headers(r#"{"c":"2","a":"2"}"#)),
        &headers(r#"{"b":"3"}"#),
    );
    let keys: Vec<&str> = merged.keys().map(String::as_str).collect();
    assert_eq!(keys, ["a", "b", "c"]);
    assert_eq!(merged, headers(r#"{"a":"2","b":"3","c":"2"}"#));
}

#[test]
fn headers_the_callers_maps_are_not_changed() {
    let given = headers(r#"{"a":"1"}"#);
    let extra = headers(r#"{"a":"2"}"#);
    let proxy = headers(r#"{"a":"3"}"#);
    let _merged = merge_request_headers(Some(&given), Some(&extra), &proxy);
    assert_eq!(given, headers(r#"{"a":"1"}"#));
    assert_eq!(extra, headers(r#"{"a":"2"}"#));
}

#[rstest]
#[case::anthropic_any_model(Some("anthropic"), "gpt-4o", true)]
#[case::bedrock_claude(Some("bedrock"), "anthropic.claude-3", true)]
#[case::bedrock_upper_case_claude(Some("bedrock"), "Anthropic.CLAUDE-3", true)]
#[case::bedrock_arn(
    Some("bedrock"),
    "arn:aws:bedrock:us-east-1:123:inference-profile/x",
    true
)]
#[case::bedrock_upper_case_arn(Some("bedrock"), "ARN:AWS:BEDROCK:us-east-1:1:x", true)]
#[case::bedrock_arn_without_the_service(Some("bedrock"), "arn:aws:sagemaker:us-east-1:1:x", false)]
#[case::bedrock_service_without_arn(Some("bedrock"), "x:bedrock:y", false)]
#[case::bedrock_other_model(Some("bedrock"), "amazon.titan-text", false)]
#[case::azure_ai_claude(Some("azure_ai"), "claude-sonnet", true)]
#[case::azure_ai_other(Some("azure_ai"), "mistral-large", false)]
#[case::azure_ai_arn_is_not_enough(Some("azure_ai"), "arn:aws:bedrock:x", false)]
#[case::vertex_claude(Some("vertex_ai"), "claude-3-5@20240620", true)]
#[case::vertex_other(Some("vertex_ai"), "gemini-pro", false)]
#[case::vertex_arn_is_not_enough(Some("vertex_ai"), "arn:aws:bedrock:x", false)]
#[case::other_provider_with_claude(Some("openai"), "claude-3", false)]
#[case::no_provider_with_claude(None, "claude-3", false)]
fn tool_keys_claude_targets(
    #[case] provider: Option<&str>,
    #[case] model: &str,
    #[case] expected: bool,
) {
    assert_eq!(is_claude_tool_target(provider, model), expected);
}

#[test]
fn tool_keys_none_stays_none() {
    assert_eq!(drop_anthropic_only_tool_keys(None), None);
}

#[test]
fn tool_keys_an_empty_list_stays_an_empty_list() {
    assert_eq!(drop_anthropic_only_tool_keys(Some(&[])), Some(Vec::new()));
}

#[rstest]
#[case::from_the_tool(
    r#"[{"type":"function","input_examples":[1],"eager_input_streaming":true,"name":"n"}]"#,
    r#"[{"type":"function","name":"n"}]"#
)]
#[case::from_the_function(
    r#"[{"type":"function","function":{"name":"n","input_examples":[1],"eager_input_streaming":false,"parameters":{}}}]"#,
    r#"[{"type":"function","function":{"name":"n","parameters":{}}}]"#
)]
#[case::from_both(
    r#"[{"input_examples":[],"function":{"eager_input_streaming":true,"name":"n"}}]"#,
    r#"[{"function":{"name":"n"}}]"#
)]
#[case::only_one_of_the_keys(
    r#"[{"input_examples":[],"function":{"eager_input_streaming":true}}]"#,
    r#"[{"function":{}}]"#
)]
#[case::nothing_to_drop(
    r#"[{"type":"function","function":{"name":"n"}}]"#,
    r#"[{"type":"function","function":{"name":"n"}}]"#
)]
#[case::no_function_adds_none(r#"[{"name":"n"}]"#, r#"[{"name":"n"}]"#)]
#[case::a_function_that_is_not_an_object(
    r#"[{"input_examples":[],"function":"text"}]"#,
    r#"[{"function":"text"}]"#
)]
#[case::a_null_function(r#"[{"input_examples":[],"function":null}]"#, r#"[{"function":null}]"#)]
#[case::nested_keys_stay(
    r#"[{"function":{"parameters":{"input_examples":1}}}]"#,
    r#"[{"function":{"parameters":{"input_examples":1}}}]"#
)]
#[case::tools_that_are_not_objects_stay(
    r#"["text",5,null,[{"input_examples":1}],{"input_examples":1}]"#,
    r#"["text",5,null,[{"input_examples":1}],{}]"#
)]
#[case::each_tool_on_its_own(
    r#"[{"input_examples":1,"a":1},{"b":2,"eager_input_streaming":1}]"#,
    r#"[{"a":1},{"b":2}]"#
)]
fn tool_keys_are_dropped_from_the_tool_and_its_function(
    #[case] given: &str,
    #[case] expected: &str,
) {
    let kept = drop_anthropic_only_tool_keys(Some(&tools(given)));
    assert_eq!(kept, Some(tools(expected)));
}

#[test]
fn tool_keys_the_remaining_keys_keep_their_order() {
    let given = tools(
        r#"[{"z":1,"input_examples":0,"function":{"y":2,"eager_input_streaming":0,"b":3},"a":4}]"#,
    );
    let kept = drop_anthropic_only_tool_keys(Some(&given)).unwrap();
    assert_eq!(
        serde_json::to_string(&kept).unwrap(),
        r#"[{"z":1,"function":{"y":2,"b":3},"a":4}]"#
    );
}
