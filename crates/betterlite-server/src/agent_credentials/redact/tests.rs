#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;
use rstest::rstest;

use super::{redact_agent_litellm_params_map, redact_sensitive_agent_litellm_params};
use crate::agent_credentials::support::{json, marker, masker, nested};

#[rstest]
#[case::api_key("api_key", r#""sk-live""#)]
#[case::access_token("access_token", "null")]
#[case::vertex_credentials("vertex_credentials", r#"{"client_email":"agent@example"}"#)]
#[case::password("password", "42")]
#[case::an_array_value("api_key", r#"["sk-one","sk-two"]"#)]
#[case::a_hyphenated_key("api-key", r#""sk-live""#)]
#[case::an_upper_case_key("API_KEY", r#""sk-live""#)]
fn a_secret_key_becomes_the_marker_whatever_its_value(#[case] key: &str, #[case] value: &str) {
    let params = BTreeMap::from([(String::from(key), json(value))]);
    let redacted = redact_agent_litellm_params_map(&params, &masker());
    assert_eq!(redacted[key], marker());
}

#[rstest]
#[case::max_tokens("max_tokens")]
#[case::input_cost_per_token("input_cost_per_token")]
#[case::model("model")]
#[case::cost("cost")]
#[case::api_keys("api_keys")]
fn a_key_without_a_secret_segment_is_left_alone(#[case] key: &str) {
    let params = BTreeMap::from([(String::from(key), json("1"))]);
    let redacted = redact_agent_litellm_params_map(&params, &masker());
    assert_eq!(redacted[key], json("1"));
}

#[test]
fn nested_objects_and_lists_are_walked() {
    let params = json(
        r#"{"model":"gpt","providers":[{"name":"one","api_key":"sk-1","limits":{"access_token":"t"}},{"name":"two"}],"password":"p"}"#,
    );
    let before = params.clone();
    assert_eq!(
        redact_sensitive_agent_litellm_params(Some(&params), &masker()),
        Some(json(
            r#"{"model":"gpt","providers":[{"name":"one","api_key":"REDACTED_BY_LITELM","limits":{"access_token":"REDACTED_BY_LITELM"}},{"name":"two"}],"password":"REDACTED_BY_LITELM"}"#,
        ))
    );
    assert_eq!(params, before);
}

#[test]
fn a_secret_key_never_recurses() {
    let params = json(r#"{"api_key":{"nested":"value"}}"#);
    let Some(AgentJson::Object(redacted)) =
        redact_sensitive_agent_litellm_params(Some(&params), &masker())
    else {
        panic!("the object stays an object");
    };
    assert_eq!(redacted["api_key"], marker());
}

#[test]
fn redacting_none_gives_none() {
    assert_eq!(redact_sensitive_agent_litellm_params(None, &masker()), None);
}

#[rstest]
#[case::null(json("null"))]
#[case::a_bool(json("true"))]
#[case::a_number(json("7.5"))]
#[case::an_array(json(r#"[{"api_key":"sk-live"}]"#))]
fn values_that_are_not_json_text_are_walked_from_depth_zero(#[case] value: AgentJson) {
    let expected = match &value {
        AgentJson::Array(_) => json(r#"[{"api_key":"REDACTED_BY_LITELM"}]"#),
        AgentJson::Null | AgentJson::Bool(_) | AgentJson::Number(_) => value.clone(),
        AgentJson::String(_) | AgentJson::Object(_) => panic!("this case is not JSON text"),
    };
    assert_eq!(
        redact_sensitive_agent_litellm_params(Some(&value), &masker()),
        Some(expected)
    );
}

#[test]
fn a_json_string_object_is_redacted_and_returned_as_text() {
    let input = AgentJson::String(String::from(r#"{"api_key":"sk-live","model":"gpt"}"#));
    let Some(AgentJson::String(text)) =
        redact_sensitive_agent_litellm_params(Some(&input), &masker())
    else {
        panic!("the result is a JSON string");
    };
    assert_eq!(text, r#"{"api_key":"REDACTED_BY_LITELM","model":"gpt"}"#);
}

#[rstest]
#[case::an_array("[1]")]
#[case::a_scalar("42")]
#[case::a_string_value(r#""text""#)]
#[case::null("null")]
#[case::invalid_text("not json")]
#[case::an_incomplete_object(r#"{"api_key":"sk-live""#)]
fn a_json_string_that_is_not_an_object_gives_the_marker(#[case] text: &str) {
    assert_eq!(
        redact_sensitive_agent_litellm_params(
            Some(&AgentJson::String(String::from(text))),
            &masker()
        ),
        Some(marker())
    );
}

#[test]
fn a_json_string_starts_its_walk_at_depth_one() {
    let kept = AgentJson::String(nested(8, json(r#""value""#)).to_string());
    let capped = AgentJson::String(nested(9, json(r#""value""#)).to_string());
    assert_eq!(
        redact_sensitive_agent_litellm_params(Some(&kept), &masker()),
        Some(kept.clone())
    );
    let Some(AgentJson::String(text)) =
        redact_sensitive_agent_litellm_params(Some(&capped), &masker())
    else {
        panic!("the result is a JSON string");
    };
    assert_eq!(json(&text), nested(9, marker()));
}

#[test]
fn a_value_at_depth_ten_becomes_the_marker_and_at_nine_it_stays() {
    let kept = nested(9, json(r#""value""#));
    assert_eq!(
        redact_sensitive_agent_litellm_params(Some(&kept), &masker()),
        Some(kept.clone())
    );
    assert_eq!(
        redact_sensitive_agent_litellm_params(Some(&nested(10, json(r#""value""#))), &masker()),
        Some(nested(10, marker()))
    );
}

#[test]
fn an_array_item_is_walked_one_depth_deeper() {
    let kept = nested(8, json(r#"["value"]"#));
    assert_eq!(
        redact_sensitive_agent_litellm_params(Some(&kept), &masker()),
        Some(kept.clone())
    );
    assert_eq!(
        redact_sensitive_agent_litellm_params(Some(&nested(9, json(r#"["value"]"#))), &masker()),
        Some(nested(9, json(r#"["REDACTED_BY_LITELM"]"#)))
    );
}

#[test]
fn the_map_redaction_leaves_its_input_unchanged() {
    let params = BTreeMap::from([
        (String::from("api_key"), json(r#""sk-live""#)),
        (String::from("providers"), json(r#"[{"access_token":"t"}]"#)),
    ]);
    let redacted = redact_agent_litellm_params_map(&params, &masker());
    assert_eq!(redacted["api_key"], marker());
    assert_eq!(params["api_key"], json(r#""sk-live""#));
    assert_eq!(params["providers"], json(r#"[{"access_token":"t"}]"#));
}
