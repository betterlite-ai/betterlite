use super::support::{Agents, claims, rendered, settings};
use crate::jwt_team_selection::agent::resolve_agent_id;

fn agent_settings() -> crate::jwt_claims::settings::JwtAuthSettings {
    settings(|fields| fields.agent_id_jwt_field = Some("agent_id".to_owned()))
}

fn token_with(claim: &str) -> serde_json::Value {
    serde_json::json!({"agent_id": claim})
}

#[test]
fn agent_absent() {
    let fields = agent_settings();
    let document = serde_json::json!({"sub": "someone"});
    let jwt = claims(document, &fields);
    let agents = Agents::default();
    assert_eq!(resolve_agent_id(&jwt, &agents).unwrap(), None);
}

#[test]
fn agent_id_before_name() {
    let fields = agent_settings();
    let document = token_with("shared");
    let jwt = claims(document, &fields);
    let agents = Agents::default()
        .with(r#"{"agent_id": "by-name", "agent_name": "shared", "agent_card_params": {}}"#)
        .with(r#"{"agent_id": "shared", "agent_name": "by-id", "agent_card_params": {}}"#);
    assert_eq!(
        resolve_agent_id(&jwt, &agents).unwrap(),
        Some("shared".to_owned())
    );
}

#[test]
fn agent_unknown() {
    let fields = agent_settings();
    let document = token_with("ghost");
    let jwt = claims(document, &fields);
    let agents = Agents::default()
        .with(r#"{"agent_id": "a1", "agent_name": "agent-one", "agent_card_params": {}}"#);
    let (status, error_type, message) = rendered(resolve_agent_id(&jwt, &agents).unwrap_err());
    assert_eq!(status, 403);
    assert_eq!(error_type, "auth_error");
    assert_eq!(
        message,
        "No registered agent matches JWT claim agent_id=ghost"
    );
}

#[test]
fn unknown_agent() {
    let fields = agent_settings();
    let document = token_with("nobody");
    let jwt = claims(document, &fields);
    let agents = Agents::default();
    assert_eq!(
        rendered(resolve_agent_id(&jwt, &agents).unwrap_err()).0,
        403
    );
}

#[test]
fn agent_managed() {
    let fields = agent_settings();
    let document = token_with("a2");
    let jwt = claims(document, &fields);
    let agents = Agents::default().with(
        r#"{"agent_id": "a2", "agent_name": "agent-two", "agent_card_params": {}, "identity_managed": true}"#,
    );
    assert_eq!(
        rendered(resolve_agent_id(&jwt, &agents).unwrap_err()).0,
        403
    );
}

#[test]
fn agent_identity_present() {
    let fields = agent_settings();
    let document = token_with("a3");
    let jwt = claims(document, &fields);
    let agents = Agents::default().with(
        r#"{"agent_id": "a3", "agent_name": "agent-three", "agent_card_params": {}, "identity": {"agent_id": "a3", "provider": "microsoft_entra", "tenant_id": "tenant", "client_id": "client", "issuer": "issuer", "revision": "1"}}"#,
    );
    assert_eq!(
        rendered(resolve_agent_id(&jwt, &agents).unwrap_err()).0,
        403
    );
}

#[test]
fn agent_legacy_null() {
    let fields = agent_settings();
    let document = token_with("a4");
    let jwt = claims(document, &fields);
    let agents = Agents::default().with(
        r#"{"agent_id": "a4", "agent_name": "agent-four", "agent_card_params": {}, "litellm_params": {"identity": null}}"#,
    );
    assert_eq!(
        rendered(resolve_agent_id(&jwt, &agents).unwrap_err()).0,
        403
    );
}

#[test]
fn agent_legacy_identity_names_the_field() {
    let fields = agent_settings();
    let document = token_with("a5");
    let jwt = claims(document, &fields);
    let agents = Agents::default().with(
        r#"{"agent_id": "a5", "agent_name": "agent-five", "agent_card_params": {}, "litellm_params": {"identity": "bound"}}"#,
    );
    assert_eq!(
        rendered(resolve_agent_id(&jwt, &agents).unwrap_err()).2,
        "No registered agent matches JWT claim agent_id=a5"
    );
}
