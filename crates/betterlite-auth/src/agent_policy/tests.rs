#![cfg(test)]

use super::*;

#[test]
fn auth_is_discriminated_and_secret_safe() {
    let auth: AgentKillSwitchAuth =
        serde_json::from_str(r#"{"type":"bearer","token":"secret"}"#).unwrap();
    assert!(!format!("{auth:?}").contains("secret"));
    assert_eq!(auth.secret_value(), "secret");
    serde_json::from_str::<AgentKillSwitchAuth>(r#"{"token":"secret"}"#).unwrap_err();
}

#[test]
fn kill_switch_preserves_url_and_defaults() {
    let config: AgentKillSwitchConfig =
        serde_json::from_str(r#"{"url":" HTTP://example.com:bad/path"}"#).unwrap();
    assert_eq!(config.url, " HTTP://example.com:bad/path");
    assert_eq!(config.method, AgentKillSwitchMethod::Post);
    assert!(config.headers.is_empty());
    assert!(config.query_params.is_empty());
    assert!(config.body.is_none());
}

#[test]
fn response_ignores_extras_and_uses_defaults() {
    let response: AgentResponse = serde_json::from_str(
        r#"{"agent_id":"a","agent_name":"n","agent_card_params":{},"unknown":true}"#,
    )
    .unwrap();
    assert!(response.enabled.0);
    assert_eq!(response.execution_mode, AgentExecutionMode::Autonomous);
    assert!(response.litellm_params.is_none());
}

#[test]
fn identity_binding_keeps_string_identifiers() {
    let binding: AgentIdentityBinding = serde_json::from_str(
        r#"{"agent_id":"a","provider":"microsoft_entra","tenant_id":"NOT-A-UUID","client_id":"c","issuer":"i","revision":"r"}"#,
    )
    .unwrap();
    assert_eq!(binding.tenant_id, "NOT-A-UUID");
    assert_eq!(binding.required_scopes, vec!["user_impersonation"]);
}
