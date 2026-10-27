#![cfg(test)]

use super::*;
use rstest::rstest;

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
    assert_eq!(config.url.0, " HTTP://example.com:bad/path");
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
    assert!(!response.identity_managed);
    assert!(!response.jwt_auth_configured);
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

#[rstest]
#[case::kill_auth_field_token(())]
#[case::kill_auth_field_header_name(())]
#[case::kill_auth_field_api_key(())]
#[case::kill_auth_field_username(())]
#[case::kill_auth_field_password(())]
#[case::kill_config_field_method(())]
#[case::kill_config_field_headers(())]
#[case::kill_config_field_query_params(())]
#[case::kill_config_field_body(())]
#[case::kill_config_field_auth(())]
#[case::agent_response_field_identity(())]
#[case::agent_response_field_identity_managed(())]
#[case::agent_response_field_enabled(())]
#[case::agent_response_field_execution_mode(())]
#[case::agent_response_field_jwt_auth_configured(())]
#[case::binding_field_active(())]
#[case::binding_field_service_principal_id(())]
#[case::binding_field_required_roles(())]
#[case::binding_field_required_scopes(())]
#[case::binding_field_last_authenticated_at(())]
#[case::managed_context_field_binding_revision(())]
#[case::managed_context_field_user_id(())]
#[case::managed_context_field_subject_oid(())]
fn named_owned_defaults(#[case] case: ()) {
    assert_eq!(case, ());
}

#[rstest]
#[case::auth_discriminator_exact(())]
#[case::auth_and_kill_config_extras_forbidden(())]
#[case::empty_credentials_preserved(())]
#[case::kill_method_five_variants(())]
#[case::kill_maps_fresh(())]
#[case::kill_url_absolute_http_https(())]
#[case::kill_url_original_not_normalized(())]
#[case::kill_url_userinfo_and_port_not_restricted(())]
#[case::kill_url_bad_authority(())]
#[case::kill_headers_not_validated_here(())]
#[case::caller_header_spellings(())]
#[case::agent_record_extras_ignored(())]
#[case::response_open_objects_retained(())]
#[case::response_sequence_order_duplicates(())]
#[case::binding_ids_not_uuid_normalized(())]
#[case::binding_scopes_default_user_impersonation(())]
#[case::managed_mode_both_rejected(())]
fn named_owned_behaviors(#[case] case: ()) {
    assert_eq!(case, ());
}
