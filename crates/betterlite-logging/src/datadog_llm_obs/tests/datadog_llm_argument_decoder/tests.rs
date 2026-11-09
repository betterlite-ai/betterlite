#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::datadog_llm_obs::arguments::{ToolArguments, decode_tool_arguments, to_dd_arguments};
use crate::datadog_llm_obs::constants::MAX_PARSED_TOOL_ARGUMENT_CHARS;
use crate::datadog_llm_obs::tests::support::{json, serialized};

fn bounded_object_argument(fill: char, repeated: usize) -> String {
    format!("{{\"k\": \"{}\"}}", fill.to_string().repeat(repeated))
}

#[rstest]
#[case::object(r#"{"city": "Paris", "days": 3}"#)]
#[case::whitespace("  {\"a\": 1}\n")]
#[case::nested(r#"{"a": {"b": [1, {"c": null}]}}"#)]
fn object_strings_parse_to_the_object(#[case] text: &str) {
    let decoded = decode_tool_arguments(text);
    let ToolArguments::Object(fields) = decoded else {
        panic!("{text} should parse to an object");
    };
    assert!(!fields.is_empty(), "{text}");
}

#[rstest]
#[case::array("[1, 2]")]
#[case::json_string(r#""hello""#)]
#[case::number("7")]
#[case::boolean("true")]
#[case::null("null")]
#[case::truncated(r#"{"a": 1"#)]
#[case::python_literal("{'a': 1}")]
#[case::plain_text("not json")]
#[case::empty("")]
fn non_object_or_unparsed_arguments_stay_the_raw_string(#[case] text: &str) {
    assert_eq!(
        decode_tool_arguments(text),
        ToolArguments::Text(text.to_owned())
    );
}

#[test]
fn arguments_at_the_character_bound_are_parsed() {
    let text = bounded_object_argument('x', MAX_PARSED_TOOL_ARGUMENT_CHARS - 9);
    assert_eq!(text.chars().count(), MAX_PARSED_TOOL_ARGUMENT_CHARS);
    assert!(matches!(
        decode_tool_arguments(&text),
        ToolArguments::Object(_)
    ));
}

#[test]
fn arguments_one_character_past_the_bound_stay_raw() {
    let text = bounded_object_argument('x', MAX_PARSED_TOOL_ARGUMENT_CHARS - 8);
    assert_eq!(text.chars().count(), MAX_PARSED_TOOL_ARGUMENT_CHARS + 1);
    assert_eq!(
        decode_tool_arguments(&text),
        ToolArguments::Text(text.clone())
    );
}

#[test]
fn the_bound_counts_characters_not_bytes() {
    let text = bounded_object_argument('\u{e9}', MAX_PARSED_TOOL_ARGUMENT_CHARS - 9);
    assert_eq!(text.chars().count(), MAX_PARSED_TOOL_ARGUMENT_CHARS);
    assert!(text.len() > MAX_PARSED_TOOL_ARGUMENT_CHARS);
    assert!(matches!(
        decode_tool_arguments(&text),
        ToolArguments::Object(_)
    ));
}

#[test]
fn an_object_value_passes_through_as_the_object() {
    let arguments = to_dd_arguments(&json(r#"{"city": "Paris"}"#));
    assert_eq!(serialized(&arguments), r#"{"city":"Paris"}"#);
    assert!(matches!(arguments, ToolArguments::Object(_)));
}

#[rstest]
#[case::array("[1, 2]")]
#[case::number("7")]
#[case::null("null")]
fn a_non_string_non_object_value_rides_along(#[case] fixture: &str) {
    let value = match serde_json::from_str::<Json>(fixture) {
        Ok(value) => value,
        Err(error) => panic!("{fixture} is JSON: {error}"),
    };
    let arguments = to_dd_arguments(&value);
    let ToolArguments::Value(carried) = arguments else {
        panic!("{fixture} is outside the object-or-string type");
    };
    assert_eq!(carried, value);
}

// serde_json bounds recursion at 128 levels, beyond which LiteLLM's json.loads still returns the object.
#[test]
fn an_argument_object_nested_past_the_json_parser_limit_stays_raw() {
    let refused = first_refused_depth();
    let text = nested_object_argument(refused);
    assert_eq!(
        decode_tool_arguments(&text),
        ToolArguments::Text(text.clone())
    );
    let accepted = nested_object_argument(refused.saturating_sub(1));
    assert!(matches!(
        decode_tool_arguments(&accepted),
        ToolArguments::Object(_)
    ));
}

fn first_refused_depth() -> usize {
    (1_usize..512)
        .find(|depth| serde_json::from_str::<Json>(&nested_object_argument(*depth)).is_err())
        .expect("the JSON parser refuses some depth")
}

fn nested_object_argument(depth: usize) -> String {
    let innermost = String::from("{\"k\": 1}");
    (0..depth).fold(innermost, |inner, _| format!("{{\"k\": {inner}}}"))
}
