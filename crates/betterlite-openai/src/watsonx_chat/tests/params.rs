use rstest::rstest;
use serde_json::{Value, json};

use super::support::object;
use crate::openai_base_config::Object;
use crate::watsonx_chat::params::{SUPPORTED_PARAMS, map_openai_params};

fn mapped(non_default: Value, optional: Value) -> Object {
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
    let optional = mapped(
        json!({
            "temperature": 0.5,
            "max_tokens": 64,
            "top_p": 0.9,
            "frequency_penalty": 0.1,
            "stop": ["end"],
            "seed": 7,
            "stream": true,
            "logprobs": true,
            "top_logprobs": 2,
            "n": 3,
            "presence_penalty": 0.2,
            "response_format": {"type": "json_object"},
            "reasoning_effort": "low",
            "user": "caller",
            "max_completion_tokens": 10,
            "top_k": 5,
        }),
        json!({}),
    );
    assert_eq!(
        Value::Object(optional),
        json!({
            "temperature": 0.5,
            "max_tokens": 64,
            "top_p": 0.9,
            "frequency_penalty": 0.1,
            "stop": ["end"],
            "seed": 7,
            "stream": true,
            "logprobs": true,
            "top_logprobs": 2,
            "n": 3,
            "presence_penalty": 0.2,
            "response_format": {"type": "json_object"},
            "reasoning_effort": "low",
        })
    );
}

#[test]
fn map_overwrites_earlier_values_and_keeps_other_keys() {
    let optional = mapped(
        json!({"temperature": 0.5}),
        json!({"temperature": 1, "max_tokens": 9, "extra_body": {"a": 1}}),
    );
    assert_eq!(
        Value::Object(optional),
        json!({"temperature": 0.5, "max_tokens": 9, "extra_body": {"a": 1}})
    );
}

#[test]
fn map_copies_a_null_supported_value() {
    let optional = mapped(json!({"temperature": null}), json!({}));
    assert_eq!(optional.get("temperature"), Some(&Value::Null));
}

#[rstest]
#[case::auto("auto")]
#[case::none("none")]
#[case::required("required")]
fn tool_choice_strings_become_the_option(#[case] choice: &str) {
    let optional = mapped(json!({"tool_choice": choice}), json!({}));
    assert_eq!(
        Value::Object(optional),
        json!({"tool_choice_option": choice})
    );
}

#[rstest]
#[case::unknown_string(json!("any"))]
#[case::wrong_case(json!("Auto"))]
#[case::empty_string(json!(""))]
#[case::function(json!({"type": "function", "function": {"name": "lookup"}}))]
#[case::number(json!(3))]
#[case::boolean(json!(false))]
fn other_tool_choices_stay_tool_choice(#[case] choice: Value) {
    let optional = mapped(json!({"tool_choice": choice.clone()}), json!({}));
    assert_eq!(Value::Object(optional), json!({"tool_choice": choice}));
}

#[test]
fn null_tool_choice_sets_nothing() {
    let optional = mapped(json!({"tool_choice": null}), json!({}));
    assert!(optional.is_empty());
}

#[test]
fn absent_tool_choice_sets_nothing() {
    let optional = mapped(json!({"temperature": 1}), json!({}));
    assert_eq!(Value::Object(optional), json!({"temperature": 1}));
}

#[test]
fn tools_lose_strict_and_false_additional_properties() {
    let optional = mapped(
        json!({"tools": [{
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
                        "typed": {"type": "object", "additionalProperties": {"type": "string"}},
                    },
                },
            },
        }]}),
        json!({}),
    );
    assert_eq!(
        Value::Object(optional),
        json!({"tools": [{
            "type": "function",
            "function": {
                "name": "lookup",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "inner": {"type": "object"},
                        "open": {"type": "object", "additionalProperties": true},
                        "typed": {"type": "object", "additionalProperties": {"type": "string"}},
                    },
                },
            },
        }]})
    );
}

#[test]
fn null_tools_are_dropped() {
    let optional = mapped(json!({"tools": null}), json!({"tools": [1]}));
    assert_eq!(Value::Object(optional), json!({"tools": [1]}));
}

#[test]
fn empty_tools_are_kept() {
    let optional = mapped(json!({"tools": []}), json!({}));
    assert_eq!(Value::Object(optional), json!({"tools": []}));
}

#[test]
fn tools_and_tool_choice_map_together() {
    let optional = mapped(
        json!({
            "tool_choice": "required",
            "tools": [{"type": "function", "strict": true}],
            "temperature": 0.2,
        }),
        json!({}),
    );
    assert_eq!(
        Value::Object(optional),
        json!({
            "tool_choice_option": "required",
            "tools": [{"type": "function"}],
            "temperature": 0.2,
        })
    );
}
