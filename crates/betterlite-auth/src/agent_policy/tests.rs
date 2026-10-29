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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Record {
    Auth,
    Config,
    Response,
    Binding,
    Managed,
}

#[rstest]
#[case::kill_auth_field_header_name(
    Record::Auth,
    r#"{"type":"api_key","api_key":"s"}"#,
    "header_name",
    r#""x-api-key""#
)]
#[case::kill_config_field_method(
    Record::Config,
    r#"{"url":"https://example.com"}"#,
    "method",
    r#""POST""#
)]
#[case::kill_config_field_headers(
    Record::Config,
    r#"{"url":"https://example.com"}"#,
    "headers",
    "{}"
)]
#[case::kill_config_field_query_params(
    Record::Config,
    r#"{"url":"https://example.com"}"#,
    "query_params",
    "{}"
)]
#[case::kill_config_field_body(Record::Config, r#"{"url":"https://example.com"}"#, "body", "null")]
#[case::kill_config_field_auth(Record::Config, r#"{"url":"https://example.com"}"#, "auth", "null")]
#[case::agent_response_field_identity(Record::Response, RESPONSE_JSON, "identity", "null")]
#[case::agent_response_field_identity_managed(
    Record::Response,
    RESPONSE_JSON,
    "identity_managed",
    "false"
)]
#[case::agent_response_field_enabled(Record::Response, RESPONSE_JSON, "enabled", "true")]
#[case::agent_response_field_execution_mode(
    Record::Response,
    RESPONSE_JSON,
    "execution_mode",
    r#""autonomous""#
)]
#[case::agent_response_field_jwt_auth_configured(
    Record::Response,
    RESPONSE_JSON,
    "jwt_auth_configured",
    "false"
)]
#[case::binding_field_active(Record::Binding, r#"{"agent_id":"a","provider":"microsoft_entra","tenant_id":"t","client_id":"c","issuer":"i","revision":"r"}"#, "active", "true")]
#[case::binding_field_service_principal_id(Record::Binding, r#"{"agent_id":"a","provider":"microsoft_entra","tenant_id":"t","client_id":"c","issuer":"i","revision":"r"}"#, "service_principal_id", "null")]
#[case::binding_field_required_roles(Record::Binding, r#"{"agent_id":"a","provider":"microsoft_entra","tenant_id":"t","client_id":"c","issuer":"i","revision":"r"}"#, "required_roles", "[]")]
#[case::binding_field_required_scopes(Record::Binding, r#"{"agent_id":"a","provider":"microsoft_entra","tenant_id":"t","client_id":"c","issuer":"i","revision":"r"}"#, "required_scopes", r#"["user_impersonation"]"#)]
#[case::binding_field_last_authenticated_at(Record::Binding, r#"{"agent_id":"a","provider":"microsoft_entra","tenant_id":"t","client_id":"c","issuer":"i","revision":"r"}"#, "last_authenticated_at", "null")]
#[case::managed_context_field_binding_revision(
    Record::Managed,
    r#"{"agent_id":"a","mode":"autonomous"}"#,
    "binding_revision",
    "null"
)]
#[case::managed_context_field_user_id(
    Record::Managed,
    r#"{"agent_id":"a","mode":"autonomous"}"#,
    "user_id",
    "null"
)]
#[case::managed_context_field_subject_oid(
    Record::Managed,
    r#"{"agent_id":"a","mode":"autonomous"}"#,
    "subject_oid",
    "null"
)]
fn named_owned_defaults(
    #[case] kind: Record,
    #[case] input: &str,
    #[case] field: &str,
    #[case] expected: &str,
) {
    assert_eq!(owned_default_field(kind, input, field), expected);
}

fn owned_default_field(kind: Record, input: &str, field: &str) -> String {
    match kind {
        Record::Auth => {
            serde_json::to_value(serde_json::from_str::<AgentKillSwitchAuth>(input).unwrap())
                .unwrap()[field]
                .to_string()
        }
        Record::Config => {
            serde_json::to_value(serde_json::from_str::<AgentKillSwitchConfig>(input).unwrap())
                .unwrap()[field]
                .to_string()
        }
        Record::Response => {
            serde_json::to_value(serde_json::from_str::<AgentResponse>(input).unwrap()).unwrap()
                [field]
                .to_string()
        }
        Record::Binding => {
            serde_json::to_value(serde_json::from_str::<AgentIdentityBinding>(input).unwrap())
                .unwrap()[field]
                .to_string()
        }
        Record::Managed => {
            serde_json::to_value(serde_json::from_str::<ManagedAgentContext>(input).unwrap())
                .unwrap()[field]
                .to_string()
        }
    }
}

#[test]
fn auth_discriminator_exact() {
    serde_json::from_str::<AgentKillSwitchAuth>(r#"{"token":"s"}"#).unwrap_err();
}

#[test]
fn auth_and_kill_config_extras_forbidden() {
    serde_json::from_str::<AgentKillSwitchAuth>(r#"{"type":"bearer","token":"s","extra":1}"#)
        .unwrap_err();
    serde_json::from_str::<AgentKillSwitchConfig>(r#"{"url":"https://example.com","extra":1}"#)
        .unwrap_err();
}

#[test]
fn empty_credentials_preserved() {
    let auth: AgentKillSwitchAuth =
        serde_json::from_str(r#"{"type":"basic","username":"","password":""}"#).unwrap();
    let AgentKillSwitchAuth::Basic { password, .. } = &auth else {
        panic!("expected basic auth");
    };
    assert_eq!(password.expose_secret(), "");
}

#[rstest]
#[case::post("POST")]
#[case::put("PUT")]
#[case::patch("PATCH")]
#[case::delete("DELETE")]
#[case::get("GET")]
fn kill_method_five_variants(#[case] method: &str) {
    let input = format!(r#"{{"url":"https://example.com","method":"{method}"}}"#);
    let config: AgentKillSwitchConfig = serde_json::from_str(&input).unwrap();
    assert_eq!(serde_json::to_value(config).unwrap()["method"], method);
}

#[test]
fn kill_maps_fresh() {
    let mut first: AgentKillSwitchConfig =
        serde_json::from_str(r#"{"url":"https://example.com"}"#).unwrap();
    let second: AgentKillSwitchConfig =
        serde_json::from_str(r#"{"url":"https://example.com"}"#).unwrap();
    first.headers.insert(String::from("x"), String::from("y"));
    assert!(second.headers.is_empty());
}

#[test]
fn kill_url_original_not_normalized() {
    let input = "  HTTP://example.com/path";
    assert_eq!(
        serde_json::from_str::<AgentKillSwitchConfig>(&format!(r#"{{"url":{input:?}}}"#))
            .unwrap()
            .url
            .0,
        input
    );
}

#[test]
fn kill_url_userinfo_and_port_not_restricted() {
    serde_json::from_str::<AgentKillSwitchConfig>(r#"{"url":"http://user@:80"}"#).unwrap();
}

#[test]
fn kill_headers_not_validated_here() {
    serde_json::from_str::<AgentKillSwitchConfig>(
        r#"{"url":"https://example.com","headers":{"Bad Header":"\u0001"}}"#,
    )
    .unwrap();
}

#[test]
fn caller_header_spellings() {
    assert_eq!(
        (CALLER_USER_ID_HEADER, CALLER_TEAM_ID_HEADER),
        ("x-litellm-user-id", "x-litellm-team-id")
    );
}

#[test]
fn agent_record_extras_ignored() {
    let response: AgentResponse = serde_json::from_str(
        r#"{"agent_id":"a","agent_name":"n","agent_card_params":{},"extra":true}"#,
    )
    .unwrap();
    assert!(
        serde_json::to_value(response)
            .unwrap()
            .get("extra")
            .is_none()
    );
}

#[test]
fn response_open_objects_retained() {
    let response: AgentResponse = serde_json::from_str(
        r#"{"agent_id":"a","agent_name":"n","agent_card_params":{"x":[1,1]}}"#,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(response).unwrap()["agent_card_params"].to_string(),
        r#"{"x":[1,1]}"#
    );
}

#[test]
fn response_sequence_order_duplicates() {
    let response: AgentResponse = serde_json::from_str(
        r#"{"agent_id":"a","agent_name":"n","agent_card_params":{"x":[2,1,2]}}"#,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(response).unwrap()["agent_card_params"].to_string(),
        r#"{"x":[2,1,2]}"#
    );
}

#[test]
fn binding_ids_not_uuid_normalized() {
    let input = r#"{"agent_id":"a","provider":"microsoft_entra","tenant_id":"not-uuid","client_id":"c","issuer":"i","revision":"r"}"#;
    assert_eq!(
        serde_json::from_str::<AgentIdentityBinding>(input)
            .unwrap()
            .tenant_id,
        "not-uuid"
    );
}

#[test]
fn binding_scopes_default_user_impersonation() {
    assert_eq!(
        serde_json::from_str::<AgentIdentityBinding>(BINDING_JSON)
            .unwrap()
            .required_scopes
            .0,
        vec!["user_impersonation"]
    );
}

#[test]
fn managed_mode_both_rejected() {
    serde_json::from_str::<ManagedAgentContext>(r#"{"agent_id":"a","mode":"both"}"#).unwrap_err();
}

const BINDING_JSON: &str = r#"{"agent_id":"a","provider":"microsoft_entra","tenant_id":"t","client_id":"c","issuer":"i","revision":"r"}"#;

#[rstest]
#[case::kill_config_field_url(Record::Config, r#"{"url":"https://example.com"}"#, "url")]
#[case::kill_auth_field_type(Record::Auth, r#"{"type":"bearer","token":"s"}"#, "type")]
#[case::kill_auth_field_token(Record::Auth, r#"{"type":"bearer","token":"s"}"#, "token")]
#[case::kill_auth_field_api_key(Record::Auth, r#"{"type":"api_key","api_key":"s"}"#, "api_key")]
#[case::kill_auth_field_username(
    Record::Auth,
    r#"{"type":"basic","username":"u","password":"p"}"#,
    "username"
)]
#[case::kill_auth_field_password(
    Record::Auth,
    r#"{"type":"basic","username":"u","password":"p"}"#,
    "password"
)]
#[case::binding_field_agent_id(Record::Binding, BINDING_JSON, "agent_id")]
#[case::binding_field_provider(Record::Binding, BINDING_JSON, "provider")]
#[case::binding_field_tenant_id(Record::Binding, BINDING_JSON, "tenant_id")]
#[case::binding_field_client_id(Record::Binding, BINDING_JSON, "client_id")]
#[case::binding_field_issuer(Record::Binding, BINDING_JSON, "issuer")]
#[case::binding_field_revision(Record::Binding, BINDING_JSON, "revision")]
#[case::managed_context_field_agent_id(
    Record::Managed,
    r#"{"agent_id":"a","mode":"autonomous"}"#,
    "agent_id"
)]
#[case::managed_context_field_mode(
    Record::Managed,
    r#"{"agent_id":"a","mode":"autonomous"}"#,
    "mode"
)]
fn required_field_omission_and_null_rejected(
    #[case] kind: Record,
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
        Record::Config => {
            serde_json::from_slice::<AgentKillSwitchConfig>(&bytes).unwrap_err();
        }
        Record::Auth => {
            serde_json::from_slice::<AgentKillSwitchAuth>(&bytes).unwrap_err();
        }
        Record::Response => {
            serde_json::from_slice::<AgentResponse>(&bytes).unwrap_err();
        }
        Record::Binding => {
            serde_json::from_slice::<AgentIdentityBinding>(&bytes).unwrap_err();
        }
        Record::Managed => {
            serde_json::from_slice::<ManagedAgentContext>(&bytes).unwrap_err();
        }
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
