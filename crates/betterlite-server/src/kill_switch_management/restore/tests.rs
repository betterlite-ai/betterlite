#![cfg(test)]

use betterlite_core::agents::{AgentKillSwitchAuth, AgentKillSwitchConfig};
use betterlite_logging::spend_sanitize::REDACTED_BY_LITELM;
use secrecy::ExposeSecret as _;

use super::restore_kill_switch;
use crate::kill_switch_management::support::{
    api_key_auth, basic_auth, bearer_auth, config_with_auth,
};

fn bearer_token(config: &AgentKillSwitchConfig) -> String {
    match config.auth.as_ref() {
        Some(AgentKillSwitchAuth::Bearer { token }) => token.expose_secret().clone(),
        Some(AgentKillSwitchAuth::ApiKey { .. } | AgentKillSwitchAuth::Basic { .. }) | None => {
            panic!("the auth type is bearer")
        }
    }
}

fn api_key_parts(config: &AgentKillSwitchConfig) -> (String, String) {
    let Some(AgentKillSwitchAuth::ApiKey {
        header_name,
        api_key,
    }) = config.auth.as_ref()
    else {
        panic!("the auth type is an api key");
    };
    (header_name.0.clone(), api_key.expose_secret().clone())
}

fn basic_parts(config: &AgentKillSwitchConfig) -> (String, String) {
    let Some(AgentKillSwitchAuth::Basic { username, password }) = config.auth.as_ref() else {
        panic!("the auth type is basic");
    };
    (username.clone(), password.expose_secret().clone())
}

#[test]
fn restoring_none_returns_none() {
    assert!(restore_kill_switch(None, None).is_none());
}

#[test]
fn restoring_a_bearer_marker_takes_the_stored_token() {
    let incoming = config_with_auth(Some(bearer_auth(REDACTED_BY_LITELM)));
    let existing = config_with_auth(Some(bearer_auth("sk-stored")));
    let restored = restore_kill_switch(Some(&incoming), Some(&existing)).expect("a config");
    assert_eq!(bearer_token(&restored), "sk-stored");
}

#[test]
fn restoring_a_bearer_marker_without_a_stored_config_clears_the_token() {
    let incoming = config_with_auth(Some(bearer_auth(REDACTED_BY_LITELM)));
    let restored = restore_kill_switch(Some(&incoming), None).expect("a config");
    assert_eq!(bearer_token(&restored), "");
}

#[test]
fn restoring_a_bearer_marker_over_a_stored_api_key_clears_the_token() {
    let incoming = config_with_auth(Some(bearer_auth(REDACTED_BY_LITELM)));
    let existing = config_with_auth(Some(api_key_auth("x-api-key", "key-stored")));
    let restored = restore_kill_switch(Some(&incoming), Some(&existing)).expect("a config");
    assert_eq!(bearer_token(&restored), "");
}

#[test]
fn restoring_a_bearer_marker_over_a_stored_password_clears_the_token() {
    let incoming = config_with_auth(Some(bearer_auth(REDACTED_BY_LITELM)));
    let existing = config_with_auth(Some(basic_auth("stored-user", "hunter2")));
    let restored = restore_kill_switch(Some(&incoming), Some(&existing)).expect("a config");
    assert_eq!(bearer_token(&restored), "");
}

#[test]
fn restoring_a_bearer_token_that_is_not_the_marker_keeps_it() {
    let incoming = config_with_auth(Some(bearer_auth("sk-new")));
    let existing = config_with_auth(Some(bearer_auth("sk-stored")));
    let restored = restore_kill_switch(Some(&incoming), Some(&existing)).expect("a config");
    assert_eq!(bearer_token(&restored), "sk-new");
}

#[test]
fn restoring_an_api_key_marker_takes_the_stored_key() {
    let incoming = config_with_auth(Some(api_key_auth("x-api-key", REDACTED_BY_LITELM)));
    let existing = config_with_auth(Some(api_key_auth("x-api-key", "key-stored")));
    let restored = restore_kill_switch(Some(&incoming), Some(&existing)).expect("a config");
    assert_eq!(
        api_key_parts(&restored),
        (String::from("x-api-key"), String::from("key-stored"))
    );
}

#[test]
fn restoring_an_api_key_marker_keeps_the_incoming_header_name() {
    let incoming = config_with_auth(Some(api_key_auth("x-new-key", REDACTED_BY_LITELM)));
    let existing = config_with_auth(Some(api_key_auth("x-old-key", "key-stored")));
    let restored = restore_kill_switch(Some(&incoming), Some(&existing)).expect("a config");
    assert_eq!(api_key_parts(&restored).0, "x-new-key");
}

#[test]
fn restoring_an_api_key_marker_without_a_stored_key_clears_it() {
    let incoming = config_with_auth(Some(api_key_auth("x-api-key", REDACTED_BY_LITELM)));
    let existing = config_with_auth(Some(bearer_auth("sk-stored")));
    let restored = restore_kill_switch(Some(&incoming), Some(&existing)).expect("a config");
    assert_eq!(api_key_parts(&restored).1, "");
}

#[test]
fn restoring_a_basic_marker_takes_the_stored_password() {
    let incoming = config_with_auth(Some(basic_auth("operator", REDACTED_BY_LITELM)));
    let existing = config_with_auth(Some(basic_auth("stored-user", "hunter2")));
    let restored = restore_kill_switch(Some(&incoming), Some(&existing)).expect("a config");
    assert_eq!(
        basic_parts(&restored),
        (String::from("operator"), String::from("hunter2"))
    );
}

#[test]
fn restoring_a_basic_marker_over_a_stored_api_key_clears_the_password() {
    let incoming = config_with_auth(Some(basic_auth("operator", REDACTED_BY_LITELM)));
    let existing = config_with_auth(Some(api_key_auth("x-api-key", "key-stored")));
    let restored = restore_kill_switch(Some(&incoming), Some(&existing)).expect("a config");
    assert_eq!(basic_parts(&restored).1, "");
}

#[test]
fn restoring_a_basic_marker_without_a_stored_password_clears_it() {
    let incoming = config_with_auth(Some(basic_auth("operator", REDACTED_BY_LITELM)));
    let restored = restore_kill_switch(Some(&incoming), None).expect("a config");
    assert_eq!(basic_parts(&restored).1, "");
}

#[test]
fn restoring_a_config_without_auth_returns_it_unchanged() {
    let incoming = config_with_auth(None);
    let existing = config_with_auth(Some(bearer_auth("sk-stored")));
    let restored = restore_kill_switch(Some(&incoming), Some(&existing)).expect("a config");
    assert!(restored.auth.is_none());
}

#[test]
fn restoring_twice_does_not_turn_a_restored_secret_back_into_the_marker() {
    let incoming = config_with_auth(Some(bearer_auth(REDACTED_BY_LITELM)));
    let existing = config_with_auth(Some(bearer_auth("sk-stored")));
    let once = restore_kill_switch(Some(&incoming), Some(&existing)).expect("a config");
    let twice = restore_kill_switch(Some(&once), None).expect("a config");
    assert_eq!(bearer_token(&twice), "sk-stored");
}

#[test]
fn restoring_keeps_the_incoming_config_over_the_stored_one() {
    let mut incoming = config_with_auth(Some(bearer_auth(REDACTED_BY_LITELM)));
    incoming.url = String::from("https://agent.example/new");
    incoming
        .headers
        .insert(String::from("x-trace"), String::from("incoming"));
    let mut existing = config_with_auth(Some(bearer_auth("sk-stored")));
    existing.url = String::from("https://agent.example/old");
    existing
        .headers
        .insert(String::from("x-other"), String::from("stored"));
    let restored = restore_kill_switch(Some(&incoming), Some(&existing)).expect("a config");
    assert_eq!(restored.url, "https://agent.example/new");
    assert_eq!(restored.headers, incoming.headers);
}
