#![cfg(test)]

use super::caller::{AgentCaller, CALLER_TEAM_ID_HEADER, CALLER_USER_ID_HEADER};
use super::{
    AgentIdentityBinding, AgentKeySummary, AgentKillSwitchAuth, AgentKillSwitchConfig,
    AgentResponse, ManagedAgentContext,
};
use rstest::rstest;
use secrecy::ExposeSecret as _;
use serde_json::value::RawValue;
use std::collections::BTreeMap;

const RESPONSE_JSON: &str = r#"{"agent_id":"a","agent_name":"n","agent_card_params":{}}"#;

#[rstest]
#[case::kill_auth_field_header_name(
    "auth",
    r#"{"type":"api_key","api_key":"s"}"#,
    "header_name",
    r#""x-api-key""#
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
#[case::agent_response_field_identity("response", RESPONSE_JSON, "identity", "null")]
#[case::agent_response_field_identity_managed(
    "response",
    RESPONSE_JSON,
    "identity_managed",
    "false"
)]
#[case::agent_response_field_enabled("response", RESPONSE_JSON, "enabled", "true")]
#[case::agent_response_field_execution_mode(
    "response",
    RESPONSE_JSON,
    "execution_mode",
    r#""autonomous""#
)]
#[case::agent_response_field_jwt_auth_configured(
    "response",
    RESPONSE_JSON,
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
        return serde_json::to_value(serde_json::from_str::<AgentKillSwitchAuth>(input).unwrap())
            .unwrap()[field]
            .to_string();
    }
    if kind == "config" {
        return serde_json::to_value(serde_json::from_str::<AgentKillSwitchConfig>(input).unwrap())
            .unwrap()[field]
            .to_string();
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
#[case::kill_method_five_variants("methods", "POST,PUT,PATCH,DELETE,GET")]
#[case::kill_maps_fresh("maps", "")]
#[case::kill_url_original_not_normalized("original", "  HTTP://example.com/path")]
#[case::kill_url_userinfo_and_port_not_restricted("authority", r#"{"url":"http://user@:80"}"#)]
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
        "auth" => {
            serde_json::from_str::<AgentKillSwitchAuth>(input).unwrap_err();
        }
        "extras" => {
            serde_json::from_str::<AgentKillSwitchAuth>(input).unwrap_err();
            serde_json::from_str::<AgentKillSwitchConfig>(
                r#"{"url":"https://example.com","extra":1}"#,
            )
            .unwrap_err();
        }
        "empty" => {
            let auth: AgentKillSwitchAuth = serde_json::from_str(input).unwrap();
            let AgentKillSwitchAuth::Basic { password, .. } = &auth else {
                panic!("expected basic auth");
            };
            assert_eq!(password.expose_secret(), "");
        }
        "methods" => {
            for method in input.split(',') {
                let method_input =
                    format!(r#"{{"url":"https://example.com","method":"{method}"}}"#);
                let config: AgentKillSwitchConfig = serde_json::from_str(&method_input).unwrap();
                assert_eq!(serde_json::to_value(config).unwrap()["method"], method);
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
        "caller" => assert_eq!(
            (CALLER_USER_ID_HEADER, CALLER_TEAM_ID_HEADER),
            ("x-litellm-user-id", "x-litellm-team-id")
        ),
        "response_extra" | "open" | "sequence" => {
            let response: AgentResponse = serde_json::from_str(input).unwrap();
            let expected = match kind {
                "open" => r#"{"x":[1,1]}"#,
                "sequence" => r#"{"x":[2,1,2]}"#,
                _ => "{}",
            };
            let output = serde_json::to_value(response).unwrap();
            assert_eq!(output["agent_card_params"].to_string(), expected);
            assert!(output.get("extra").is_none());
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
                    .required_scopes
                    .0,
                vec!["user_impersonation"]
            );
        }
        "managed_both" => {
            serde_json::from_str::<ManagedAgentContext>(input).unwrap_err();
        }
        _ => panic!("unknown behavior case"),
    }
}

const BINDING_JSON: &str = r#"{"agent_id":"a","provider":"microsoft_entra","tenant_id":"t","client_id":"c","issuer":"i","revision":"r"}"#;

#[rstest]
#[case::kill_config_field_url("config", r#"{"url":"https://example.com"}"#, "url")]
#[case::kill_auth_field_type("auth", r#"{"type":"bearer","token":"s"}"#, "type")]
#[case::kill_auth_field_token("auth", r#"{"type":"bearer","token":"s"}"#, "token")]
#[case::kill_auth_field_api_key("auth", r#"{"type":"api_key","api_key":"s"}"#, "api_key")]
#[case::kill_auth_field_username(
    "auth",
    r#"{"type":"basic","username":"u","password":"p"}"#,
    "username"
)]
#[case::kill_auth_field_password(
    "auth",
    r#"{"type":"basic","username":"u","password":"p"}"#,
    "password"
)]
#[case::binding_field_agent_id("binding", BINDING_JSON, "agent_id")]
#[case::binding_field_provider("binding", BINDING_JSON, "provider")]
#[case::binding_field_tenant_id("binding", BINDING_JSON, "tenant_id")]
#[case::binding_field_client_id("binding", BINDING_JSON, "client_id")]
#[case::binding_field_issuer("binding", BINDING_JSON, "issuer")]
#[case::binding_field_revision("binding", BINDING_JSON, "revision")]
#[case::managed_context_field_agent_id(
    "managed",
    r#"{"agent_id":"a","mode":"autonomous"}"#,
    "agent_id"
)]
#[case::managed_context_field_mode("managed", r#"{"agent_id":"a","mode":"autonomous"}"#, "mode")]
fn required_field_omission_and_null_rejected(
    #[case] kind: &str,
    #[case] input: &str,
    #[case] field: &str,
    #[values(false, true)] explicit_null: bool,
) {
    let mut fields: BTreeMap<String, Box<RawValue>> = serde_json::from_str(input).unwrap();
    if explicit_null {
        fields.insert(field.to_owned(), serde_json::from_str("null").unwrap());
    } else {
        fields.remove(field);
    }
    let bytes = serde_json::to_vec(&fields).unwrap();
    match kind {
        "config" => {
            serde_json::from_slice::<AgentKillSwitchConfig>(&bytes).unwrap_err();
        }
        "auth" => {
            serde_json::from_slice::<AgentKillSwitchAuth>(&bytes).unwrap_err();
        }
        "binding" => {
            serde_json::from_slice::<AgentIdentityBinding>(&bytes).unwrap_err();
        }
        "managed" => {
            serde_json::from_slice::<ManagedAgentContext>(&bytes).unwrap_err();
        }
        _ => panic!("unknown kind"),
    }
}

#[test]
fn caller_defaults_and_extras_ignored() {
    let caller: AgentCaller = serde_json::from_str(r#"{"extra":1}"#).unwrap();
    assert!(caller.user_id.is_none());
    assert!(caller.team_id.is_none());
}

#[test]
fn key_summary_requires_token_and_defaults() {
    let summary: AgentKeySummary = serde_json::from_str(r#"{"token":"s","extra":1}"#).unwrap();
    assert_eq!(summary.token, "s");
    assert!(summary.key_alias.is_none());
    assert!(summary.key_name.is_none());
    serde_json::from_str::<AgentKeySummary>("{}").unwrap_err();
}

#[rstest]
#[case::agent_response_field_litellm_params("litellm_params")]
#[case::agent_response_field_object_permission("object_permission")]
#[case::agent_response_field_spend("spend")]
#[case::agent_response_field_tpm_limit("tpm_limit")]
#[case::agent_response_field_rpm_limit("rpm_limit")]
#[case::agent_response_field_session_tpm_limit("session_tpm_limit")]
#[case::agent_response_field_session_rpm_limit("session_rpm_limit")]
#[case::agent_response_field_static_headers("static_headers")]
#[case::agent_response_field_extra_headers("extra_headers")]
#[case::agent_response_field_access_group_ids("access_group_ids")]
#[case::agent_response_field_kill_switch("kill_switch")]
#[case::agent_response_field_keys("keys")]
#[case::agent_response_field_search_score("search_score")]
#[case::agent_response_field_created_at("created_at")]
#[case::agent_response_field_updated_at("updated_at")]
#[case::agent_response_field_created_by("created_by")]
#[case::agent_response_field_updated_by("updated_by")]
fn response_nullable_fields(#[case] field: &str) {
    let missing: AgentResponse = serde_json::from_str(RESPONSE_JSON).unwrap();
    assert!(
        serde_json::to_value(missing)
            .unwrap()
            .get(field)
            .unwrap()
            .is_null()
    );
    let explicit_input =
        format!(r#"{{"agent_id":"a","agent_name":"n","agent_card_params":{{}},"{field}":null}}"#);
    let explicit: AgentResponse = serde_json::from_str(&explicit_input).unwrap();
    assert!(
        serde_json::to_value(explicit)
            .unwrap()
            .get(field)
            .unwrap()
            .is_null()
    );
}

#[rstest]
#[case::bearer(r#"{"type":"bearer","token":"carried-secret"}"#, "token")]
#[case::api_key(r#"{"type":"api_key","api_key":"carried-secret"}"#, "api_key")]
#[case::basic(
    r#"{"type":"basic","username":"u","password":"carried-secret"}"#,
    "password"
)]
fn policy_secrets_absent_from_debug(#[case] input: &str, #[case] field: &str) {
    let auth: AgentKillSwitchAuth = serde_json::from_str(input).unwrap();
    assert!(!format!("{auth:?}").contains("carried-secret"));
    assert_eq!(
        serde_json::to_value(&auth).unwrap()[field],
        "carried-secret"
    );
    let config: AgentKillSwitchConfig = serde_json::from_str(&format!(
        r#"{{"url":"https://example.com","auth":{input}}}"#
    ))
    .unwrap();
    assert!(!format!("{config:?}").contains("carried-secret"));
    assert_eq!(
        serde_json::to_value(&config).unwrap()["auth"][field],
        "carried-secret"
    );
    let serialized = serde_json::to_string(&config).unwrap();
    let response: AgentResponse = serde_json::from_str(&format!(
        r#"{{"agent_id":"a","agent_name":"n","agent_card_params":{{}},"kill_switch":{serialized}}}"#
    ))
    .unwrap();
    assert!(!format!("{response:?}").contains("carried-secret"));
    assert_eq!(
        serde_json::to_value(response).unwrap()["kill_switch"]["auth"][field],
        "carried-secret"
    );
}

#[test]
fn binding_provider_other_rejected() {
    serde_json::from_str::<AgentIdentityBinding>(
        r#"{"agent_id":"a","provider":"other","tenant_id":"t","client_id":"c","issuer":"i","revision":"r"}"#
    ).unwrap_err();
}

#[rstest]
#[case::agent_response_field_agent_id("agent_id")]
#[case::agent_response_field_agent_name("agent_name")]
#[case::agent_response_field_agent_card_params("agent_card_params")]
fn response_required_fields(#[case] field: &str, #[values(false, true)] explicit_null: bool) {
    let mut fields: BTreeMap<String, Box<RawValue>> = serde_json::from_str(RESPONSE_JSON).unwrap();
    if explicit_null {
        fields.insert(field.to_owned(), serde_json::from_str("null").unwrap());
    } else {
        fields.remove(field);
    }
    serde_json::from_slice::<AgentResponse>(&serde_json::to_vec(&fields).unwrap()).unwrap_err();
}

#[rstest]
#[case::autonomous("autonomous")]
#[case::delegated("delegated")]
#[case::both("both")]
fn response_execution_modes(#[case] mode: &str) {
    let input = format!(
        r#"{{"agent_id":"a","agent_name":"n","agent_card_params":{{}},"execution_mode":"{mode}"}}"#
    );
    let response: AgentResponse = serde_json::from_str(&input).unwrap();
    assert_eq!(
        serde_json::to_value(response).unwrap()["execution_mode"],
        mode
    );
}

#[rstest]
#[case::autonomous("autonomous")]
#[case::delegated("delegated")]
fn managed_execution_modes(#[case] mode: &str) {
    let input = format!(r#"{{"agent_id":"a","mode":"{mode}"}}"#);
    let context: ManagedAgentContext = serde_json::from_str(&input).unwrap();
    assert_eq!(serde_json::to_value(context).unwrap()["mode"], mode);
}

#[rstest]
#[case::agent_response_field_created_at("created_at")]
#[case::agent_response_field_updated_at("updated_at")]
fn response_date_fields_pad_fractional_seconds(#[case] field: &str) {
    let input = format!(
        r#"{{"agent_id":"a","agent_name":"n","agent_card_params":{{}},"{field}":"2026-01-01T12:00:00.123456789Z"}}"#
    );
    let response: AgentResponse = serde_json::from_str(&input).unwrap();
    assert_eq!(
        serde_json::to_value(response).unwrap()[field],
        "2026-01-01T12:00:00.123456Z"
    );
}
