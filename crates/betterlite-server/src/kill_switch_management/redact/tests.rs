#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;
use betterlite_core::agents::{AgentKillSwitchAuth, AgentKillSwitchConfig, AgentKillSwitchMethod};
use betterlite_logging::spend_sanitize::REDACTED_BY_LITELM;
use secrecy::ExposeSecret as _;

use super::redact_kill_switch;
use crate::kill_switch_management::support::{
    api_key_auth, basic_auth, bearer_auth, config_with_auth,
};

fn auth_of(config: &AgentKillSwitchConfig) -> &AgentKillSwitchAuth {
    config.auth.as_ref().expect("the config keeps its auth")
}

#[test]
fn redacting_none_returns_none() {
    assert!(redact_kill_switch(None).is_none());
}

#[test]
fn redacting_a_bearer_config_replaces_the_token() {
    let config = config_with_auth(Some(bearer_auth("sk-live")));
    let redacted = redact_kill_switch(Some(&config)).expect("the config is returned");
    let AgentKillSwitchAuth::Bearer { token } = auth_of(&redacted) else {
        panic!("the auth type is kept");
    };
    assert_eq!(token.expose_secret(), REDACTED_BY_LITELM);
}

#[test]
fn redacting_an_api_key_config_keeps_the_header_name() {
    let config = config_with_auth(Some(api_key_auth("x-custom-key", "key-live")));
    let redacted = redact_kill_switch(Some(&config)).expect("the config is returned");
    let AgentKillSwitchAuth::ApiKey {
        header_name,
        api_key,
    } = auth_of(&redacted)
    else {
        panic!("the auth type is kept");
    };
    assert_eq!(header_name.0, "x-custom-key");
    assert_eq!(api_key.expose_secret(), REDACTED_BY_LITELM);
}

#[test]
fn redacting_a_basic_config_keeps_the_username() {
    let config = config_with_auth(Some(basic_auth("operator", "hunter2")));
    let redacted = redact_kill_switch(Some(&config)).expect("the config is returned");
    let AgentKillSwitchAuth::Basic { username, password } = auth_of(&redacted) else {
        panic!("the auth type is kept");
    };
    assert_eq!(username, "operator");
    assert_eq!(password.expose_secret(), REDACTED_BY_LITELM);
}

#[test]
fn redacting_a_config_without_auth_returns_it_unchanged() {
    let config = config_with_auth(None);
    let redacted = redact_kill_switch(Some(&config)).expect("the config is returned");
    assert!(redacted.auth.is_none());
    assert_eq!(redacted.url, config.url);
    assert_eq!(redacted.method, config.method);
}

#[test]
fn redacting_keeps_every_field_that_is_not_a_secret() {
    let mut config = config_with_auth(Some(bearer_auth("sk-live")));
    config.url = String::from("https://agent.example/stop?region=us");
    config.method = AgentKillSwitchMethod::Put;
    config
        .headers
        .insert(String::from("x-trace"), String::from("on"));
    config
        .query_params
        .insert(String::from("region"), String::from("us"));
    config.body = Some(BTreeMap::from([(
        String::from("reason"),
        AgentJson::String(String::from("kill")),
    )]));
    let redacted = redact_kill_switch(Some(&config)).expect("the config is returned");
    assert_eq!(redacted.url, config.url);
    assert_eq!(redacted.method, AgentKillSwitchMethod::Put);
    assert_eq!(redacted.headers, config.headers);
    assert_eq!(redacted.query_params, config.query_params);
    assert_eq!(redacted.body, config.body);
}
