#![cfg(test)]

use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;
use serde_json::{Number, Value, json};

use crate::databricks_chat::request::{
    SUPPORTED_OPENAI_PARAMS, get_complete_url, get_config, get_required_params, pop_user_agent,
    pop_user_agent_parameter, should_fake_stream,
};

fn object(value: Value) -> Object {
    serde_json::from_value(value).expect("the fixture is an object")
}

// get_complete_url: two dots in the model after the provider prefix turn the AI gateway on
#[rstest]
#[case(
    Some("https://adb-1.cloud.databricks.com/serving-endpoints"),
    "databricks-meta-llama-3-1-70b",
    "https://adb-1.cloud.databricks.com/serving-endpoints/chat/completions"
)]
#[case(
    Some("https://adb-1.cloud.databricks.com/serving-endpoints"),
    "databricks/anthropic.claude-3-5-sonnet-2",
    "https://adb-1.cloud.databricks.com/ai-gateway/mlflow/v1/chat/completions"
)]
#[case(
    Some("https://adb-1.cloud.databricks.com/"),
    "databricks/anthropic.claude-3-5-sonnet-2",
    "https://adb-1.cloud.databricks.com/ai-gateway/mlflow/v1/chat/completions"
)]
#[case(
    Some("https://adb-1.cloud.databricks.com/ai-gateway/mlflow/v1/"),
    "databricks/anthropic.claude-3-5-sonnet-2",
    "https://adb-1.cloud.databricks.com/ai-gateway/mlflow/v1/chat/completions"
)]
#[case(
    Some("https://adb-1.cloud.databricks.com/serving-endpoints/"),
    "databricks/one.dot",
    "https://adb-1.cloud.databricks.com/serving-endpoints//chat/completions"
)]
fn the_complete_url_follows_the_gateway_rule(
    #[case] api_base: Option<&str>,
    #[case] model: &str,
    #[case] expected: &str,
) {
    assert_eq!(
        get_complete_url(api_base, model).expect("the url builds"),
        expected
    );
}

#[test]
fn a_missing_api_base_is_a_bad_request() {
    let error = get_complete_url(None, "databricks-meta-llama-3-1-70b")
        .expect_err("the url needs an api base");
    assert_eq!(error.status_code(), 400);
}

#[test]
fn the_user_agent_wins_and_keeps_the_databricks_one() {
    let mut params = object(json!({"user_agent": "first", "databricks_user_agent": "second"}));
    assert_eq!(
        pop_user_agent_parameter(&mut params).expect("the user agents pop"),
        Some(String::from("first"))
    );
    assert_eq!(
        params.get("databricks_user_agent"),
        Some(&Json::Text(String::from("second")))
    );
}

#[test]
fn the_databricks_user_agent_is_popped_when_the_user_agent_is_absent() {
    let mut params = object(json!({"databricks_user_agent": "second"}));
    assert_eq!(
        pop_user_agent_parameter(&mut params).expect("the user agent pops"),
        Some(String::from("second"))
    );
    assert!(params.is_empty());
}

#[rstest]
#[case(json!(null))]
#[case(json!(""))]
fn an_empty_user_agent_falls_through_to_the_databricks_one(#[case] value: Value) {
    let mut params = object(json!({"user_agent": value, "databricks_user_agent": "second"}));
    assert_eq!(
        pop_user_agent_parameter(&mut params).expect("the user agents pop"),
        Some(String::from("second"))
    );
    assert!(params.is_empty());
}

#[rstest]
#[case(json!(5), "user_agent")]
#[case(json!([]), "user_agent")]
#[case(json!({}), "databricks_user_agent")]
fn a_non_string_user_agent_is_a_bad_request(#[case] value: Value, #[case] name: &str) {
    let mut params = object(json!({ (name): value }));
    let error = pop_user_agent_parameter(&mut params).expect_err("the user agent is invalid");
    assert_eq!(error.status_code(), 400);
    assert!(
        error
            .to_string()
            .contains(&format!("the {name} optional parameter must be a string")),
        "{error}"
    );
}

#[test]
fn a_popped_user_agent_leaves_no_parameter_behind() {
    let mut params = object(json!({"user_agent": "first", "temperature": 1}));
    pop_user_agent_parameter(&mut params).expect("the user agent pops");
    assert_eq!(params.len(), 1);
    assert!(params.contains_key("temperature"));
}

#[test]
fn popping_a_user_agent_that_is_not_there_is_none() {
    let mut params = Object::new();
    assert_eq!(
        pop_user_agent(&mut params, "user_agent").expect("an absent user agent is fine"),
        None
    );
}

#[rstest]
#[case(json!(null), false)]
#[case(json!({"type": "json_object"}), true)]
#[case(json!("json"), true)]
fn should_fake_stream_follows_a_set_response_format(#[case] value: Value, #[case] expected: bool) {
    let params = object(json!({"response_format": value}));
    assert_eq!(should_fake_stream(&params), expected);
}

#[test]
fn the_config_carries_the_default_chat_max_tokens() {
    assert_eq!(
        get_config(4096).get("max_tokens"),
        Some(&Json::Number(Number::from(4096_i64)))
    );
}

#[test]
fn the_required_params_name_the_api_key_and_base() {
    let params = get_required_params();
    let names: Vec<&str> = params.iter().map(|field| field.field_name).collect();
    assert_eq!(names, ["api_key", "api_base"]);
}

#[test]
fn the_supported_params_are_the_openai_subset_databricks_maps() {
    assert!(SUPPORTED_OPENAI_PARAMS.contains(&"response_format"));
    assert!(SUPPORTED_OPENAI_PARAMS.contains(&"thinking"));
    assert!(!SUPPORTED_OPENAI_PARAMS.contains(&"presence_penalty"));
}
