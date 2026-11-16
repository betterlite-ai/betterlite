#![cfg(test)]

use betterlite_prompts::arguments::json::Json;
use rstest::rstest;

use crate::databricks_chat::response::tests::{body, failure, success, usage_of};
use crate::databricks_chat::tests::{at, fill, has, json, set};

#[test]
fn usage_of_an_empty_block_is_zeroed() {
    let mut value = body(json!({"role": "assistant", "content": "x"}));
    set(&mut value, "usage", json!({}));
    assert_eq!(
        usage_of(&success(&value, false)),
        json!({
            "completion_tokens": 0,
            "prompt_tokens": 0,
            "total_tokens": 0,
            "completion_tokens_details": null,
            "prompt_tokens_details": null
        })
    );
}

#[test]
fn reasoning_tokens_fill_the_completion_details() {
    let mut value = body(json!({"role": "assistant", "content": "x"}));
    set(
        &mut value,
        "usage",
        json!({"prompt_tokens": 1, "completion_tokens": 10, "total_tokens": 11, "reasoning_tokens": 3,
               "completion_tokens_details": {"image_tokens": 2, "audio_tokens": 1}}),
    );
    assert_eq!(
        usage_of(&success(&value, false)),
        json!({
            "completion_tokens": 10,
            "prompt_tokens": 1,
            "total_tokens": 11,
            "completion_tokens_details": {"image_tokens": 2, "audio_tokens": 1, "reasoning_tokens": 3, "text_tokens": 4},
            "prompt_tokens_details": null
        })
    );
}

#[test]
fn reasoning_tokens_keep_a_provider_text_tokens_value() {
    let mut value = body(json!({"role": "assistant", "content": "x"}));
    set(
        &mut value,
        "usage",
        json!({"prompt_tokens": 1, "completion_tokens": 10, "total_tokens": 11, "reasoning_tokens": 3,
               "completion_tokens_details": {"text_tokens": 99}}),
    );
    assert_eq!(
        *at(
            &usage_of(&success(&value, false)),
            "completion_tokens_details"
        ),
        json!({"text_tokens": 99, "reasoning_tokens": 3})
    );
}

#[test]
fn reasoning_tokens_without_completion_tokens_skip_text_tokens() {
    let mut value = body(json!({"role": "assistant", "content": "x"}));
    set(
        &mut value,
        "usage",
        json!({"prompt_tokens": 1, "completion_tokens": null, "total_tokens": 3, "reasoning_tokens": 3}),
    );
    assert_eq!(
        usage_of(&success(&value, false)),
        json!({
            "completion_tokens": 0,
            "prompt_tokens": 1,
            "total_tokens": 3,
            "completion_tokens_details": {"reasoning_tokens": 3},
            "prompt_tokens_details": null
        })
    );
}

#[test]
fn text_tokens_clamp_at_zero() {
    let mut value = body(json!({"role": "assistant", "content": "x"}));
    set(
        &mut value,
        "usage",
        json!({"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3, "reasoning_tokens": 5}),
    );
    assert_eq!(
        *at(
            &usage_of(&success(&value, false)),
            "completion_tokens_details"
        ),
        json!({"reasoning_tokens": 5, "text_tokens": 0})
    );
}

#[test]
fn cache_keys_fold_into_the_prompt_details_and_stay_on_the_usage() {
    let mut value = body(json!({"role": "assistant", "content": "x"}));
    set(
        &mut value,
        "usage",
        json!({"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3,
               "cache_read_input_tokens": 9, "cache_creation_input_tokens": 4, "prompt_cache_hit_tokens": 2}),
    );
    assert_eq!(
        usage_of(&success(&value, false)),
        json!({
            "completion_tokens": 2,
            "prompt_tokens": 1,
            "total_tokens": 3,
            "completion_tokens_details": null,
            "prompt_tokens_details": {"cached_tokens": 9, "cache_write_tokens": 4, "cache_creation_tokens": 4},
            "cache_read_input_tokens": 9,
            "cache_creation_input_tokens": 4,
            "prompt_cache_hit_tokens": 2
        })
    );
}

#[test]
fn a_nested_cache_creation_key_fills_the_write_tokens() {
    let mut value = body(json!({"role": "assistant", "content": "x"}));
    set(
        &mut value,
        "usage",
        json!({"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3,
               "prompt_tokens_details": {"cache_creation_input_tokens": 3}}),
    );
    assert_eq!(
        *at(&usage_of(&success(&value, false)), "prompt_tokens_details"),
        json!({"cache_creation_input_tokens": 3, "cache_write_tokens": 3, "cache_creation_tokens": 3})
    );
}

#[test]
fn prompt_details_drop_their_unset_optional_fields() {
    let mut value = body(json!({"role": "assistant", "content": "x"}));
    set(
        &mut value,
        "usage",
        json!({"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3,
               "prompt_tokens_details": {"character_count": null, "cached_tokens": 5, "zz": 1}}),
    );
    assert_eq!(
        *at(&usage_of(&success(&value, false)), "prompt_tokens_details"),
        json!({"cached_tokens": 5, "zz": 1})
    );
}

#[rstest]
#[case(json!({}))]
#[case(json!("x"))]
#[case(json!([1]))]
fn a_truthy_non_object_details_value_is_dropped(#[case] details: Json) {
    let mut value = body(json!({"role": "assistant", "content": "x"}));
    set(
        &mut value,
        "usage",
        fill(
            fill(
                json!({"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3,
               "completion_tokens_details": "$0", "prompt_tokens_details": "$1"}),
                "$0",
                &details,
            ),
            "$1",
            &details,
        ),
    );
    let output = usage_of(&success(&value, false));
    assert_eq!(*at(&output, "completion_tokens_details"), json!(null));
    assert_eq!(*at(&output, "prompt_tokens_details"), json!(null));
}

#[test]
fn server_tool_use_keeps_its_declared_fields_and_cost_is_forwarded() {
    let mut value = body(json!({"role": "assistant", "content": "x"}));
    set(
        &mut value,
        "usage",
        json!({"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3,
               "server_tool_use": {"web_search_requests": 2, "zz": 1}, "cost": 0.5, "zz": null}),
    );
    let output = usage_of(&success(&value, false));
    assert_eq!(
        *at(&output, "server_tool_use"),
        json!({"web_search_requests": 2})
    );
    assert_eq!(*at(&output, "cost"), json!(0.5));
    assert_eq!(*at(&output, "zz"), json!(null));
}

#[test]
fn a_null_server_tool_use_and_cost_are_dropped() {
    let mut value = body(json!({"role": "assistant", "content": "x"}));
    set(
        &mut value,
        "usage",
        json!({"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3,
               "server_tool_use": null, "cost": null}),
    );
    let output = usage_of(&success(&value, false));
    assert!(!has(&output, "server_tool_use"));
    assert!(!has(&output, "cost"));
}

#[rstest]
#[case("prompt_tokens", json!("5"))]
#[case("completion_tokens", json!(2.5))]
#[case("total_tokens", json!(true))]
#[case("reasoning_tokens", json!("3"))]
fn a_wrong_typed_usage_count_is_a_shape_error(#[case] name: &str, #[case] value: Json) {
    let mut body = body(json!({"role": "assistant", "content": "x"}));
    let mut usage = json!({"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3});
    set(&mut usage, name, value);
    set(&mut body, "usage", usage);
    let error = failure(&body, false);
    assert_eq!(error.status_code(), 500);
    assert_eq!(
        error.to_string(),
        format!("the {name} of the Databricks usage must be an integer")
    );
}
