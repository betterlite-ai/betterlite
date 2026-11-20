#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_core::agents::AgentKillSwitchMethod;
use betterlite_core::agents::json::AgentJson;
use betterlite_core::agents::kill_switch::{AgentKillSwitchAuth, AgentKillSwitchConfig};
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::{parse_agent_kill_switch, serialize_agent_kill_switch};
use crate::agent_credentials::REDACTED_BY_LITELM;
use crate::agent_credentials::support::{api_key_auth, basic_auth, bearer, json, nested, switch};

const STORED_BEARER: &str = r#"{"url":"https://agent.example/stop","method":"POST","headers":{},"query_params":{},"body":null,"auth":{"type":"bearer","token":"sk-live"}}"#;

#[test]
fn parsing_nothing_gives_none() {
    assert!(parse_agent_kill_switch(None).is_none());
}

#[rstest]
#[case::null(json("null"))]
#[case::invalid_text(json(r#""not json""#))]
#[case::null_text(json(r#""null""#))]
#[case::a_bool(json("true"))]
#[case::a_number(json("1"))]
#[case::an_array(json("[]"))]
#[case::a_lower_case_method(json(r#"{"url":"https://agent.example/stop","method":"post"}"#))]
#[case::a_missing_url(json(r#"{"auth":null}"#))]
#[case::a_relative_url(json(r#"{"url":"/stop"}"#))]
#[case::an_unknown_field(json(r#"{"url":"https://agent.example/stop","unknown":true}"#))]
fn an_invalid_stored_switch_gives_none(#[case] value: AgentJson) {
    assert!(parse_agent_kill_switch(Some(&value)).is_none());
}

#[test]
fn a_stored_object_parses_to_the_config() {
    let value = json(
        r#"{"url":"https://agent.example/stop","method":"PUT","headers":{"x-trace":"on"},"query_params":{},"body":null,"auth":{"type":"bearer","token":"sk-live"}}"#,
    );
    let config = parse_agent_kill_switch(Some(&value)).expect("the object is a config");
    assert_eq!(config.url, "https://agent.example/stop");
    assert_eq!(config.method, AgentKillSwitchMethod::Put);
    assert_eq!(config.headers["x-trace"], "on");
    let Some(AgentKillSwitchAuth::Bearer { token }) = config.auth else {
        panic!("the auth type is kept");
    };
    assert_eq!(token.expose_secret(), "sk-live");
}

#[test]
fn a_stored_json_string_parses_to_the_config() {
    let value = AgentJson::String(String::from(r#"{"url":"https://agent.example/stop"}"#));
    let config = parse_agent_kill_switch(Some(&value)).expect("the string holds a config");
    assert_eq!(config.url, "https://agent.example/stop");
    assert_eq!(config.method, AgentKillSwitchMethod::Post);
    assert!(config.auth.is_none());
}

#[test]
fn a_cleared_switch_is_stored_as_the_json_null_text() {
    let existing = json(STORED_BEARER);
    assert_eq!(
        serialize_agent_kill_switch(None, Some(&existing)).unwrap(),
        "null"
    );
    assert_eq!(serialize_agent_kill_switch(None, None).unwrap(), "null");
}

#[test]
fn a_marker_secret_is_restored_from_the_stored_switch() {
    let incoming = switch(Some(bearer(REDACTED_BY_LITELM)));
    let existing = json(STORED_BEARER);
    let stored = json(&serialize_agent_kill_switch(Some(&incoming), Some(&existing)).unwrap());
    assert_eq!(stored["auth"]["token"].as_str(), Some("sk-live"));
}

#[test]
fn a_marker_without_a_stored_switch_is_stored_as_an_empty_secret() {
    let incoming = switch(Some(bearer(REDACTED_BY_LITELM)));
    let stored = json(&serialize_agent_kill_switch(Some(&incoming), None).unwrap());
    assert_eq!(stored["auth"]["token"].as_str(), Some(""));
}

#[rstest]
#[case::bearer(switch(Some(bearer("sk-live"))), "token", "sk-live")]
#[case::api_key(
    switch(Some(api_key_auth("x-custom-key", "key-live"))),
    "api_key",
    "key-live"
)]
#[case::basic(switch(Some(basic_auth("operator", "hunter2"))), "password", "hunter2")]
fn the_stored_text_writes_the_real_secret(
    #[case] config: AgentKillSwitchConfig,
    #[case] field: &str,
    #[case] secret: &str,
) {
    let stored = json(&serialize_agent_kill_switch(Some(&config), None).unwrap());
    assert_eq!(stored["auth"][field].as_str(), Some(secret));
}

#[test]
fn the_stored_text_keeps_every_field() {
    let mut config = switch(Some(api_key_auth("x-custom-key", "key-live")));
    config.method = AgentKillSwitchMethod::Delete;
    config.headers = BTreeMap::from([(String::from("x-trace"), String::from("on"))]);
    config.query_params = BTreeMap::from([(String::from("region"), String::from("us"))]);
    config.body = Some(BTreeMap::from([(
        String::from("reason"),
        json(r#""kill""#),
    )]));
    let stored = json(&serialize_agent_kill_switch(Some(&config), None).unwrap());
    assert_eq!(
        stored,
        json(
            r#"{"url":"https://agent.example/stop","method":"DELETE","headers":{"x-trace":"on"},"query_params":{"region":"us"},"body":{"reason":"kill"},"auth":{"type":"api_key","header_name":"x-custom-key","api_key":"key-live"}}"#,
        )
    );
}

#[test]
fn the_stored_text_carries_no_nul_character() {
    let mut config = switch(Some(bearer("sk\0live")));
    config.url = String::from("https://agent.example/\0stop");
    config.headers = BTreeMap::from([(String::from("x-\0trace"), String::from("on\0"))]);
    config.query_params = BTreeMap::from([(String::from("re\0gion"), String::from("us"))]);
    config.body = Some(BTreeMap::from([(
        String::from("re\0ason"),
        AgentJson::String(String::from("kill\0")),
    )]));
    let text = serialize_agent_kill_switch(Some(&config), None).unwrap();
    assert!(!text.contains('\0'));
    assert!(!text.contains("\\u0000"));
    let stored = json(&text);
    assert_eq!(stored["url"].as_str(), Some("https://agent.example/stop"));
    assert_eq!(stored["headers"]["x-trace"].as_str(), Some("on"));
    assert_eq!(stored["query_params"]["region"].as_str(), Some("us"));
    assert_eq!(stored["body"]["reason"], json(r#""kill""#));
    assert_eq!(stored["auth"]["token"].as_str(), Some("sklive"));
}

#[test]
fn a_body_past_the_litellm_depth_cap_is_stored_as_sent() {
    let mut config = switch(None);
    config.body = Some(BTreeMap::from([(
        String::from("payload"),
        nested(101, json(r#""value""#)),
    )]));
    let stored = json(&serialize_agent_kill_switch(Some(&config), None).unwrap());
    assert_eq!(stored["body"]["payload"], nested(101, json(r#""value""#)));
}
