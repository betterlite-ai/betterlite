#![cfg(test)]

use rstest::rstest;
use serde_json::{Value, json};

use super::error::VolcEngineChatError;
use super::params::{SUPPORTED_OPENAI_PARAMS, map_openai_params};
use super::settings::VolcEngineChatConfig;
use crate::openai_base_config::Object;

fn object(value: Value) -> Object {
    serde_json::from_value(value).unwrap()
}

fn map(params: Value, options: Value) -> Result<Object, VolcEngineChatError> {
    let mut optional = object(options);
    map_openai_params(&object(params), &mut optional).map(|()| optional)
}

#[test]
fn the_supported_list_is_the_fixed_litellm_list() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        [
            "frequency_penalty",
            "logit_bias",
            "logprobs",
            "top_logprobs",
            "max_completion_tokens",
            "max_tokens",
            "n",
            "presence_penalty",
            "seed",
            "stop",
            "stream",
            "stream_options",
            "temperature",
            "top_p",
            "tools",
            "tool_choice",
            "function_call",
            "functions",
            "max_retries",
            "extra_headers",
            "thinking",
        ]
    );
}

#[test]
fn every_supported_parameter_is_copied_and_the_rest_are_dropped() {
    let params = json!({
        "frequency_penalty": 0.1,
        "logit_bias": {"1": 2},
        "logprobs": true,
        "top_logprobs": 2,
        "max_tokens": 7,
        "n": 2,
        "presence_penalty": 0.2,
        "seed": 4,
        "stop": ["END"],
        "stream": false,
        "stream_options": {"include_usage": true},
        "temperature": 0,
        "top_p": 1,
        "tools": [],
        "tool_choice": "auto",
        "function_call": "none",
        "functions": [],
        "max_retries": 3,
        "extra_headers": {"x-call": "1"},
        "response_format": {"type": "json_object"},
        "modalities": ["text"],
        "unknown": 1,
    });
    assert_eq!(
        map(params, json!({})).unwrap(),
        object(json!({
            "frequency_penalty": 0.1,
            "logit_bias": {"1": 2},
            "logprobs": true,
            "top_logprobs": 2,
            "max_tokens": 7,
            "n": 2,
            "presence_penalty": 0.2,
            "seed": 4,
            "stop": ["END"],
            "stream": false,
            "stream_options": {"include_usage": true},
            "temperature": 0,
            "top_p": 1,
            "tools": [],
            "tool_choice": "auto",
            "function_call": "none",
            "functions": [],
            "max_retries": 3,
            "extra_headers": {"x-call": "1"},
        }))
    );
}

#[rstest]
#[case::only_completion_tokens(
    json!({"max_completion_tokens": 5}),
    json!({}),
    json!({"max_tokens": 5})
)]
#[case::only_max_tokens(json!({"max_tokens": 7}), json!({}), json!({"max_tokens": 7}))]
#[case::completion_tokens_wins(
    json!({"max_tokens": 7, "max_completion_tokens": 9}),
    json!({}),
    json!({"max_tokens": 9})
)]
#[case::null_is_renamed(
    json!({"max_completion_tokens": null}),
    json!({}),
    json!({"max_tokens": null})
)]
#[case::existing_limit_without_a_request(
    json!({}),
    json!({"max_tokens": 1, "max_completion_tokens": 2}),
    json!({"max_tokens": 1, "max_completion_tokens": 2})
)]
#[case::existing_max_tokens_is_overwritten(
    json!({"max_completion_tokens": 2}),
    json!({"max_tokens": 1}),
    json!({"max_tokens": 2})
)]
fn the_token_limit_is_renamed_after_the_filter(
    #[case] params: Value,
    #[case] options: Value,
    #[case] expected: Value,
) {
    assert_eq!(map(params, options).unwrap(), object(expected));
}

#[rstest]
#[case::enabled(
    json!({"thinking": {"type": "enabled", "budget_tokens": 3}}),
    json!({"extra_body": {"thinking": {"type": "enabled", "budget_tokens": 3}}})
)]
#[case::disabled(
    json!({"thinking": {"type": "disabled"}}),
    json!({"extra_body": {"thinking": {"type": "disabled"}}})
)]
#[case::auto(
    json!({"thinking": {"type": "auto"}}),
    json!({"extra_body": {"thinking": {"type": "auto"}}})
)]
#[case::other_type(json!({"thinking": {"type": "other"}}), json!({}))]
#[case::missing_type(json!({"thinking": {"budget_tokens": 3}}), json!({}))]
#[case::bool(json!({"thinking": true}), json!({}))]
#[case::text(json!({"thinking": "enabled"}), json!({}))]
#[case::null(json!({"thinking": null}), json!({}))]
#[case::list(json!({"thinking": [{"type": "enabled"}]}), json!({}))]
fn thinking_moves_into_extra_body_only_for_the_three_legal_types(
    #[case] params: Value,
    #[case] expected: Value,
) {
    assert_eq!(map(params, json!({})).unwrap(), object(expected));
}

#[test]
fn a_legal_thinking_merges_into_an_existing_extra_body_and_wins_on_its_key() {
    let mapped = map(
        json!({"thinking": {"type": "enabled"}}),
        json!({"extra_body": {"keep": 1, "thinking": {"type": "disabled"}}}),
    )
    .unwrap();
    assert_eq!(
        mapped,
        object(json!({"extra_body": {"keep": 1, "thinking": {"type": "enabled"}}}))
    );
}

#[test]
fn an_illegal_thinking_leaves_an_existing_extra_body_alone() {
    let mapped = map(
        json!({"thinking": {"type": "other"}}),
        json!({"extra_body": {"keep": 1}}),
    )
    .unwrap();
    assert_eq!(mapped, object(json!({"extra_body": {"keep": 1}})));
    let not_an_object = map(
        json!({"thinking": {"type": "other"}}),
        json!({"extra_body": 5}),
    );
    assert_eq!(not_an_object.unwrap(), object(json!({"extra_body": 5})));
}

#[test]
fn a_legal_thinking_needs_an_object_extra_body() {
    let error = map(
        json!({"thinking": {"type": "enabled"}}),
        json!({"extra_body": 5}),
    );
    assert!(matches!(
        error,
        Err(VolcEngineChatError::ExtraBodyNotObject)
    ));
    let null_body = map(
        json!({"thinking": {"type": "auto"}}),
        json!({"extra_body": null}),
    );
    assert!(matches!(
        null_body,
        Err(VolcEngineChatError::ExtraBodyNotObject)
    ));
}

#[test]
fn thinking_already_in_the_options_moves_the_same_way() {
    let mapped = map(
        json!({}),
        json!({"temperature": 0.2, "thinking": {"type": "auto"}}),
    )
    .unwrap();
    assert_eq!(
        mapped,
        object(json!({"temperature": 0.2, "extra_body": {"thinking": {"type": "auto"}}}))
    );
}

#[test]
fn the_class_snapshot_keeps_every_configured_value_including_zero_and_empty() {
    let config = VolcEngineChatConfig::try_from(object(json!({
        "temperature": 0,
        "stop": "",
        "top_p": [],
        "n": 2,
        "response_format": {"type": "text"},
    })))
    .unwrap();
    assert_eq!(
        config.config(),
        object(json!({
            "temperature": 0,
            "stop": "",
            "top_p": [],
            "n": 2,
            "response_format": {"type": "text"},
        }))
    );
}

#[test]
fn the_class_snapshot_omits_a_null_value_and_starts_empty() {
    let nulls =
        VolcEngineChatConfig::try_from(object(json!({"temperature": null, "stop": null}))).unwrap();
    assert_eq!(nulls.config(), Object::new());
    assert_eq!(VolcEngineChatConfig::default().config(), Object::new());
}

#[test]
fn an_unknown_setting_is_an_error_that_names_it() {
    let error = VolcEngineChatConfig::try_from(object(json!({"max_length": 5})));
    let Err(VolcEngineChatError::Settings { source }) = error else {
        panic!("an unknown setting must fail");
    };
    assert!(source.to_string().contains("max_length"));
}
