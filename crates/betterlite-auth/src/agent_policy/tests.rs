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
#[case::kill_method_five_variants("methods", "POST")]
#[case::kill_method_put("methods", "PUT")]
#[case::kill_method_patch("methods", "PATCH")]
#[case::kill_method_delete("methods", "DELETE")]
#[case::kill_method_get("methods", "GET")]
#[case::kill_maps_fresh("maps", "")]
#[case::kill_url_absolute_http_https("urls", "http://example.com")]
#[case::kill_url_https("urls", "https://example.com")]
#[case::kill_url_original_not_normalized("original", "  HTTP://example.com/path")]
#[case::kill_url_userinfo_and_port_not_restricted("authority", r#"{"url":"http://user@:80"}"#)]
#[case::kill_url_bad_authority("bad_url", "http://[::1")]
#[case::kill_url_missing_authority("bad_url", "http:example.com")]
#[case::kill_url_one_slash("bad_url", "http:/example.com")]
#[case::kill_url_backslash("bad_url", r"http:\example.com")]
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
            let method_input = format!(r#"{{"url":"https://example.com","method":"{input}"}}"#);
            let config: AgentKillSwitchConfig = serde_json::from_str(&method_input).unwrap();
            assert_eq!(serde_json::to_value(config).unwrap()["method"], input);
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
            assert_eq!(KillSwitchUrl::try_from(input.to_owned()).unwrap().0, input);
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
            KillSwitchUrl::try_from(input.to_owned()).unwrap_err();
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
    let input = r#"{"agent_id":"a","agent_name":"n","agent_card_params":{}}"#;
    let missing: AgentResponse = serde_json::from_str(input).unwrap();
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
#[case::userinfo_ipv6("http://user@[::1]/", true)]
#[case::invalid_ip_literal("http://[abc]", false)]
#[case::bracket_suffix("http://[::1]x", false)]
#[case::scheme_tab("ht\ttp://example.com", true)]
#[case::authority_tab("http:/\t/example.com", true)]
#[case::cr_lf("h\rtt\np://example.com", true)]
#[case::ipvfuture("http://[v1.a]", true)]
#[case::invalid_ipvfuture("http://[v.a]", false)]
#[case::scoped_ipv6("http://[fe80::1%eth0]", true)]
#[case::invalid_scope("http://[fe80::1%]", false)]
#[case::nfkc_separator("http://example.com\u{ff0f}bad", false)]
fn urlsplit_acceptance(#[case] input: &str, #[case] accepted: bool) {
    let result = KillSwitchUrl::try_from(input.to_owned());
    assert_eq!(result.is_ok(), accepted);
    if let Ok(url) = result {
        assert_eq!(url.0, input);
    }
}

#[rstest]
#[case::agent_response_field_agent_id("agent_id")]
#[case::agent_response_field_agent_name("agent_name")]
#[case::agent_response_field_agent_card_params("agent_card_params")]
fn response_required_fields(#[case] field: &str, #[values(false, true)] explicit_null: bool) {
    let mut fields: BTreeMap<String, Box<RawValue>> =
        serde_json::from_str(r#"{"agent_id":"a","agent_name":"n","agent_card_params":{}}"#)
            .unwrap();
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
