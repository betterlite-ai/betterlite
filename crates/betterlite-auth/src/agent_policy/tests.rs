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
#[case::kill_auth_field_token("auth", r#"{"type":"bearer","token":"s"}"#, "token", r#""s""#)]
#[case::kill_auth_field_header_name(
    "auth",
    r#"{"type":"api_key","api_key":"s"}"#,
    "header_name",
    r#""x-api-key""#
)]
#[case::kill_auth_field_api_key("auth", r#"{"type":"api_key","api_key":"s"}"#, "api_key", r#""s""#)]
#[case::kill_auth_field_username(
    "auth",
    r#"{"type":"basic","username":"u","password":"s"}"#,
    "username",
    r#""u""#
)]
#[case::kill_auth_field_password(
    "auth",
    r#"{"type":"basic","username":"u","password":"s"}"#,
    "password",
    r#""s""#
)]
#[case::kill_config_field_method(
    "config",
    r#"{"url":"https://example.com"}"#,
    "method",
    r#""POST""#
)]
#[case::kill_config_field_headers("config", r#"{"url":"https://example.com"}"#, "headers", "{}")]
#[case::kill_config_field_query_params(
    "config",
    r#"{"url":"https://example.com"}"#,
    "query_params",
    "{}"
)]
#[case::kill_config_field_body("config", r#"{"url":"https://example.com"}"#, "body", "null")]
#[case::kill_config_field_auth("config", r#"{"url":"https://example.com"}"#, "auth", "null")]
#[case::agent_response_field_identity(
    "response",
    r#"{"agent_id":"a","agent_name":"n","agent_card_params":{}}"#,
    "identity",
    "null"
)]
#[case::agent_response_field_identity_managed(
    "response",
    r#"{"agent_id":"a","agent_name":"n","agent_card_params":{}}"#,
    "identity_managed",
    "false"
)]
#[case::agent_response_field_enabled(
    "response",
    r#"{"agent_id":"a","agent_name":"n","agent_card_params":{}}"#,
    "enabled",
    "true"
)]
#[case::agent_response_field_execution_mode(
    "response",
    r#"{"agent_id":"a","agent_name":"n","agent_card_params":{}}"#,
    "execution_mode",
    r#""autonomous""#
)]
#[case::agent_response_field_jwt_auth_configured(
    "response",
    r#"{"agent_id":"a","agent_name":"n","agent_card_params":{}}"#,
    "jwt_auth_configured",
    "false"
)]
#[case::binding_field_active("binding", r#"{"agent_id":"a","provider":"microsoft_entra","tenant_id":"t","client_id":"c","issuer":"i","revision":"r"}"#, "active", "true")]
#[case::binding_field_service_principal_id("binding", r#"{"agent_id":"a","provider":"microsoft_entra","tenant_id":"t","client_id":"c","issuer":"i","revision":"r"}"#, "service_principal_id", "null")]
#[case::binding_field_required_roles("binding", r#"{"agent_id":"a","provider":"microsoft_entra","tenant_id":"t","client_id":"c","issuer":"i","revision":"r"}"#, "required_roles", "[]")]
#[case::binding_field_required_scopes("binding", r#"{"agent_id":"a","provider":"microsoft_entra","tenant_id":"t","client_id":"c","issuer":"i","revision":"r"}"#, "required_scopes", r#"["user_impersonation"]"#)]
#[case::binding_field_last_authenticated_at("binding", r#"{"agent_id":"a","provider":"microsoft_entra","tenant_id":"t","client_id":"c","issuer":"i","revision":"r"}"#, "last_authenticated_at", "null")]
#[case::managed_context_field_binding_revision(
    "managed",
    r#"{"agent_id":"a","mode":"autonomous"}"#,
    "binding_revision",
    "null"
)]
#[case::managed_context_field_user_id(
    "managed",
    r#"{"agent_id":"a","mode":"autonomous"}"#,
    "user_id",
    "null"
)]
#[case::managed_context_field_subject_oid(
    "managed",
    r#"{"agent_id":"a","mode":"autonomous"}"#,
    "subject_oid",
    "null"
)]
fn named_owned_defaults(
    #[case] kind: &str,
    #[case] input: &str,
    #[case] field: &str,
    #[case] expected: &str,
) {
    assert_eq!(owned_default_field(kind, input, field), expected);
}

fn owned_default_field(kind: &str, input: &str, field: &str) -> String {
    if kind == "auth" {
        return serde_json::to_value(
            serde_json::from_str::<AgentKillSwitchAuth>(input)
                .unwrap()
                .exposed(),
        )
        .unwrap()[field]
            .to_string();
    }
    if kind == "config" {
        let config: AgentKillSwitchConfig = serde_json::from_str(input).unwrap();
        let mut output = serde_json::to_value(&config).unwrap();
        if let Some(auth) = config.auth.as_ref() {
            output.as_object_mut().unwrap().insert(
                String::from("auth"),
                serde_json::to_value(auth.exposed()).unwrap(),
            );
        }
        return output[field].to_string();
    }
    if kind == "response" {
        return serde_json::to_value(serde_json::from_str::<AgentResponse>(input).unwrap())
            .unwrap()[field]
            .to_string();
    }
    if kind == "binding" {
        return serde_json::to_value(serde_json::from_str::<AgentIdentityBinding>(input).unwrap())
            .unwrap()[field]
            .to_string();
    }
    if kind == "managed" {
        return serde_json::to_value(serde_json::from_str::<ManagedAgentContext>(input).unwrap())
            .unwrap()[field]
            .to_string();
    }
    panic!("unknown case kind");
}

#[rstest]
#[case::auth_discriminator_exact("auth", r#"{"token":"s"}"#)]
#[case::auth_and_kill_config_extras_forbidden(
    "extras",
    r#"{"type":"bearer","token":"s","extra":1}"#
)]
#[case::empty_credentials_preserved("empty", r#"{"type":"basic","username":"","password":""}"#)]
#[case::kill_method_five_variants("methods", "")]
#[case::kill_maps_fresh("maps", "")]
#[case::kill_url_absolute_http_https("urls", "")]
#[case::kill_url_original_not_normalized("original", "  HTTP://example.com/path")]
#[case::kill_url_userinfo_and_port_not_restricted("authority", r#"{"url":"http://user@:80"}"#)]
#[case::kill_url_bad_authority("bad_url", "")]
#[case::kill_headers_not_validated_here(
    "headers",
    r#"{"url":"https://example.com","headers":{"Bad Header":"\u0001"}}"#
)]
#[case::caller_header_spellings("caller", "")]
#[case::agent_record_extras_ignored(
    "response_extra",
    r#"{"agent_id":"a","agent_name":"n","agent_card_params":{},"extra":true}"#
)]
#[case::response_open_objects_retained(
    "open",
    r#"{"agent_id":"a","agent_name":"n","agent_card_params":{"x":[1,1]}}"#
)]
#[case::response_sequence_order_duplicates(
    "sequence",
    r#"{"agent_id":"a","agent_name":"n","agent_card_params":{"x":[2,1,2]}}"#
)]
#[case::binding_ids_not_uuid_normalized("binding_id", r#"{"agent_id":"a","provider":"microsoft_entra","tenant_id":"not-uuid","client_id":"c","issuer":"i","revision":"r"}"#)]
#[case::binding_scopes_default_user_impersonation("scopes", "")]
#[case::managed_mode_both_rejected("managed_both", r#"{"agent_id":"a","mode":"both"}"#)]
fn named_owned_behaviors(#[case] kind: &str, #[case] input: &str) {
    match kind {
        "auth" | "extras" => {
            serde_json::from_str::<AgentKillSwitchAuth>(input).unwrap_err();
        }
        "empty" => {
            let auth: AgentKillSwitchAuth = serde_json::from_str(input).unwrap();
            assert_eq!(auth.secret_value(), "");
        }
        "methods" => {
            for method in ["POST", "PUT", "PATCH", "DELETE", "GET"] {
                let method_input =
                    format!(r#"{{"url":"https://example.com","method":"{method}"}}"#);
                serde_json::from_str::<AgentKillSwitchConfig>(&method_input).unwrap();
            }
        }
        "maps" => {
            let mut first: AgentKillSwitchConfig =
                serde_json::from_str(r#"{"url":"https://example.com"}"#).unwrap();
            let second: AgentKillSwitchConfig =
                serde_json::from_str(r#"{"url":"https://example.com"}"#).unwrap();
            first.headers.insert(String::from("x"), String::from("y"));
            assert!(second.headers.is_empty());
        }
        "urls" => {
            for url in ["http://example.com", "https://example.com"] {
                serde_json::from_str::<AgentKillSwitchConfig>(&format!(r#"{{"url":"{url}"}}"#))
                    .unwrap();
            }
        }
        "original" => assert_eq!(
            serde_json::from_str::<AgentKillSwitchConfig>(&format!(r#"{{"url":{input:?}}}"#))
                .unwrap()
                .url
                .0,
            input
        ),
        "authority" | "headers" => {
            serde_json::from_str::<AgentKillSwitchConfig>(input).unwrap();
        }
        "bad_url" => {
            for url in [
                "http:example.com",
                "http:/example.com",
                r"http:\example.com",
                "http://[::1",
            ] {
                serde_json::from_str::<AgentKillSwitchConfig>(&format!(r#"{{"url":{url:?}}}"#))
                    .unwrap_err();
            }
        }
        "caller" => assert_eq!(
            (CALLER_USER_ID_HEADER, CALLER_TEAM_ID_HEADER),
            ("x-litellm-user-id", "x-litellm-team-id")
        ),
        "response_extra" | "open" | "sequence" => {
            serde_json::from_str::<AgentResponse>(input).unwrap();
        }
        "binding_id" => assert_eq!(
            serde_json::from_str::<AgentIdentityBinding>(input)
                .unwrap()
                .tenant_id,
            "not-uuid"
        ),
        "scopes" => {
            let binding_input = r#"{"agent_id":"a","provider":"microsoft_entra","tenant_id":"t","client_id":"c","issuer":"i","revision":"r"}"#;
            assert_eq!(
                serde_json::from_str::<AgentIdentityBinding>(binding_input)
                    .unwrap()
                    .required_scopes,
                vec!["user_impersonation"]
            );
        }
        "managed_both" => {
            serde_json::from_str::<ManagedAgentContext>(input).unwrap_err();
        }
        _ => panic!("unknown behavior case"),
    }
}
