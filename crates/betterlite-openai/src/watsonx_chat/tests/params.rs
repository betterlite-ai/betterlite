use rstest::rstest;
use serde_json::Value;

use super::support::{fixture, object};
use crate::openai_base_config::Object;
use crate::watsonx_chat::params::{SUPPORTED_PARAMS, map_openai_params};

fn mapped(non_default: &str, optional: &str) -> Object {
    let mut optional_params = object(optional);
    map_openai_params(&object(non_default), &mut optional_params).unwrap();
    optional_params
}

#[test]
fn supported_params_keep_litellm_order() {
    assert_eq!(
        SUPPORTED_PARAMS,
        [
            "temperature",
            "max_tokens",
            "top_p",
            "frequency_penalty",
            "stop",
            "seed",
            "stream",
            "tools",
            "tool_choice",
            "logprobs",
            "top_logprobs",
            "n",
            "presence_penalty",
            "response_format",
            "reasoning_effort",
        ]
    );
}

#[test]
fn map_keeps_supported_params_and_drops_the_rest() {
    let supported = r#""temperature": 0.5, "max_tokens": 64, "top_p": 0.9, "frequency_penalty": 0.1,
        "stop": ["end"], "seed": 7, "stream": true, "logprobs": true, "top_logprobs": 2, "n": 3,
        "presence_penalty": 0.2, "response_format": {"type": "json_object"}, "reasoning_effort": "low""#;
    let unsupported = r#""user": "caller", "max_completion_tokens": 10, "top_k": 5"#;
    let optional = mapped(&format!("{{{supported}, {unsupported}}}"), "{}");
    assert_eq!(
        Value::Object(optional),
        fixture(&format!("{{{supported}}}"))
    );
}

#[test]
fn map_overwrites_earlier_values_and_keeps_other_keys() {
    let optional = mapped(
        r#"{"temperature": 0.5}"#,
        r#"{"temperature": 1, "max_tokens": 9, "extra_body": {"a": 1}}"#,
    );
    assert_eq!(
        Value::Object(optional),
        fixture(r#"{"temperature": 0.5, "max_tokens": 9, "extra_body": {"a": 1}}"#)
    );
}

#[test]
fn map_copies_a_null_supported_value() {
    let optional = mapped(r#"{"temperature": null}"#, "{}");
    assert_eq!(optional.get("temperature"), Some(&Value::Null));
}

#[rstest]
#[case::auto("auto")]
#[case::none("none")]
#[case::required("required")]
fn tool_choice_strings_become_the_option(#[case] choice: &str) {
    let optional = mapped(&format!(r#"{{"tool_choice": "{choice}"}}"#), "{}");
    assert_eq!(
        Value::Object(optional),
        fixture(&format!(r#"{{"tool_choice_option": "{choice}"}}"#))
    );
}

#[rstest]
#[case::unknown_string(r#""any""#)]
#[case::wrong_case(r#""Auto""#)]
#[case::empty_string(r#""""#)]
#[case::function(r#"{"type": "function", "function": {"name": "lookup"}}"#)]
#[case::number("3")]
#[case::boolean("false")]
fn other_tool_choices_stay_tool_choice(#[case] choice: &str) {
    let optional = mapped(&format!(r#"{{"tool_choice": {choice}}}"#), "{}");
    assert_eq!(
        Value::Object(optional),
        fixture(&format!(r#"{{"tool_choice": {choice}}}"#))
    );
}

#[test]
fn null_tool_choice_sets_nothing() {
    let optional = mapped(r#"{"tool_choice": null}"#, "{}");
    assert!(optional.is_empty());
}

#[test]
fn absent_tool_choice_sets_nothing() {
    let optional = mapped(r#"{"temperature": 1}"#, "{}");
    assert_eq!(Value::Object(optional), fixture(r#"{"temperature": 1}"#));
}

#[test]
fn tools_lose_strict_and_false_additional_properties() {
    let optional = mapped(
        r#"{"tools": [{
            "type": "function",
            "function": {
                "name": "lookup",
                "strict": true,
                "parameters": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "inner": {"type": "object", "additionalProperties": false, "strict": false},
                        "open": {"type": "object", "additionalProperties": true},
                        "typed": {"type": "object", "additionalProperties": {"type": "string"}}
                    }
                }
            }
        }]}"#,
        "{}",
    );
    assert_eq!(
        Value::Object(optional),
        fixture(
            r#"{"tools": [{
                "type": "function",
                "function": {
                    "name": "lookup",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "inner": {"type": "object"},
                            "open": {"type": "object", "additionalProperties": true},
                            "typed": {"type": "object", "additionalProperties": {"type": "string"}}
                        }
                    }
                }
            }]}"#
        )
    );
}

#[test]
fn null_tools_are_dropped() {
    let optional = mapped(r#"{"tools": null}"#, r#"{"tools": [1]}"#);
    assert_eq!(Value::Object(optional), fixture(r#"{"tools": [1]}"#));
}

#[test]
fn empty_tools_are_kept() {
    let optional = mapped(r#"{"tools": []}"#, "{}");
    assert_eq!(Value::Object(optional), fixture(r#"{"tools": []}"#));
}

#[test]
fn tools_and_tool_choice_map_together() {
    let optional = mapped(
        r#"{
            "tool_choice": "required",
            "tools": [{"type": "function", "strict": true}],
            "temperature": 0.2
        }"#,
        "{}",
    );
    assert_eq!(
        Value::Object(optional),
        fixture(
            r#"{
                "tool_choice_option": "required",
                "tools": [{"type": "function"}],
                "temperature": 0.2
            }"#
        )
    );
}
