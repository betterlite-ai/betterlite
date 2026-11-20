#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;
use rstest::rstest;

use super::parse_agent_litellm_params;
use crate::agent_credentials::support::json;

#[test]
fn none_gives_an_empty_map() {
    assert!(parse_agent_litellm_params(None).is_empty());
}

#[rstest]
#[case::null(json("null"))]
#[case::a_bool(json("true"))]
#[case::a_number(json("1"))]
#[case::an_array(json("[1]"))]
#[case::invalid_text(json(r#""not json""#))]
#[case::a_non_object_text(json(r#""[1]""#))]
#[case::a_null_text(json(r#""null""#))]
#[case::an_incomplete_object_text(json(r#""{\"api_key\":\"sk""#))]
fn a_value_that_is_not_an_object_gives_an_empty_map(#[case] value: AgentJson) {
    assert!(parse_agent_litellm_params(Some(&value)).is_empty());
}

#[test]
fn an_object_is_taken_as_it_is() {
    let value = json(r#"{"api_key":"sk-live","model":"gpt"}"#);
    assert_eq!(
        parse_agent_litellm_params(Some(&value)),
        BTreeMap::from([
            (String::from("api_key"), json(r#""sk-live""#)),
            (String::from("model"), json(r#""gpt""#)),
        ])
    );
}

#[test]
fn a_json_object_string_is_parsed() {
    let value = AgentJson::String(String::from(r#"{"model":"gpt","max_tokens":12}"#));
    assert_eq!(
        parse_agent_litellm_params(Some(&value)),
        BTreeMap::from([
            (String::from("max_tokens"), json("12")),
            (String::from("model"), json(r#""gpt""#)),
        ])
    );
}

#[test]
fn the_stored_value_is_never_changed() {
    let value = json(r#"{"api_key":"sk-live"}"#);
    let parsed = parse_agent_litellm_params(Some(&value));
    assert_eq!(parsed["api_key"], json(r#""sk-live""#));
    assert_eq!(value, json(r#"{"api_key":"sk-live"}"#));
}
