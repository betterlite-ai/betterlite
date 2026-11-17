#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use rust_decimal::Decimal;
use secrecy::ExposeSecret as _;

use crate::jwt_claims::TypedValue;
use crate::sso_types::decode::{
    MAX_SSO_RECORD_BYTES, SsoRecord, SsoRecordError, SsoRecordKind, decode_open_id_identity,
    decode_sso_record,
};
use crate::sso_types::identity::{
    LoginMethod, OAUTH_TOKEN_FIELDS, OpenIdIdentity, ParsedOpenIdResult, ReturnedUiTokenObject,
};
use crate::sso_types::records::{
    AccessControlUiAccessMode, DefaultTeamSsoParams, MicrosoftGraphGroupResponse,
    MicrosoftServicePrincipalTeam, RoleMappings, SsoConfig, TeamMappings, UiAccessMode,
    UiAccessModeType, UpperboundKeyGenerateParams, check_is_admin_only_access,
};
use crate::sso_types::roles::{
    has_admin_ui_access, highest_privilege_role, is_valid_litellm_user_role, role_from_claim,
    role_from_claim_value, role_from_name,
};
use crate::user_roles::UserRole;

fn claim(text: &str) -> TypedValue {
    serde_json::from_str(text).unwrap()
}

#[test]
fn highest_privilege_role_prefers_the_ranked_role_whatever_the_order() {
    assert_eq!(
        highest_privilege_role([UserRole::InternalUser, UserRole::ProxyAdmin]),
        Some(UserRole::ProxyAdmin)
    );
    assert_eq!(
        highest_privilege_role([UserRole::ProxyAdmin, UserRole::InternalUser]),
        Some(UserRole::ProxyAdmin)
    );
    assert_eq!(
        highest_privilege_role([UserRole::InternalUser, UserRole::ProxyAdminViewer]),
        Some(UserRole::ProxyAdminViewer)
    );
    assert_eq!(
        highest_privilege_role([UserRole::InternalUserViewer, UserRole::InternalUser]),
        Some(UserRole::InternalUser)
    );
    assert_eq!(
        highest_privilege_role([UserRole::ProxyAdminViewer, UserRole::ProxyAdmin]),
        Some(UserRole::ProxyAdmin)
    );
}

#[test]
fn unranked_roles_resolve_by_name_after_every_ranked_role() {
    assert_eq!(
        highest_privilege_role([UserRole::Team, UserRole::InternalUserViewer]),
        Some(UserRole::InternalUserViewer)
    );
    assert_eq!(
        highest_privilege_role([UserRole::Team, UserRole::Customer]),
        Some(UserRole::Customer)
    );
    assert_eq!(
        highest_privilege_role([UserRole::OrgAdmin, UserRole::Team]),
        Some(UserRole::OrgAdmin)
    );
    assert_eq!(
        highest_privilege_role([UserRole::Team, UserRole::OrgAdmin]),
        Some(UserRole::OrgAdmin)
    );
    assert_eq!(
        highest_privilege_role([UserRole::ProxyAdmin, UserRole::ProxyAdmin]),
        Some(UserRole::ProxyAdmin)
    );
    assert_eq!(highest_privilege_role(Vec::<UserRole>::new()), None);
}

#[test]
fn a_role_claim_reads_a_string_in_any_case() {
    assert_eq!(
        role_from_claim(&claim("\"proxy_admin\"")),
        Some(UserRole::ProxyAdmin)
    );
    assert_eq!(
        role_from_claim(&claim("\"Proxy_Admin_Viewer\"")),
        Some(UserRole::ProxyAdminViewer)
    );
    assert_eq!(
        role_from_claim(&claim("\"INTERNAL_USER\"")),
        Some(UserRole::InternalUser)
    );
    assert_eq!(role_from_claim(&claim("\"superuser\"")), None);
    assert_eq!(role_from_claim(&claim("\"\"")), None);
}

#[test]
fn a_role_array_ignores_members_that_are_not_strings() {
    let mixed = claim(
        r#"["internal_user", 5, null, true, {"role": "proxy_admin"}, ["proxy_admin"], "proxy_admin"]"#,
    );
    assert_eq!(role_from_claim(&mixed), Some(UserRole::ProxyAdmin));
    let without_a_role = claim(r#"[7, null, false, ["proxy_admin"], {"role": "proxy_admin"}]"#);
    assert_eq!(role_from_claim(&without_a_role), None);
    assert_eq!(role_from_claim(&claim(r#"[["internal_user"]]"#)), None);
}

#[test]
fn conflicting_app_roles_resolve_to_the_highest_privilege() {
    assert_eq!(
        role_from_claim(&claim(r#"["internal_user", "proxy_admin"]"#)),
        Some(UserRole::ProxyAdmin)
    );
    assert_eq!(
        role_from_claim(&claim(r#"["proxy_admin", "internal_user"]"#)),
        Some(UserRole::ProxyAdmin)
    );
    assert_eq!(
        role_from_claim(&claim(r#"["customer", "proxy_admin_viewer", "team"]"#)),
        Some(UserRole::ProxyAdminViewer)
    );
    assert_eq!(
        role_from_claim(&claim(r#"["team", "customer"]"#)),
        Some(UserRole::Customer)
    );
    assert_eq!(
        role_from_claim(&claim(r#"["customer", "superuser"]"#)),
        Some(UserRole::Customer)
    );
}

#[test]
fn a_claim_that_is_not_a_string_or_array_resolves_to_no_role() {
    assert_eq!(role_from_claim(&claim("42")), None);
    assert_eq!(role_from_claim(&claim("null")), None);
    assert_eq!(role_from_claim(&claim("true")), None);
    assert_eq!(role_from_claim(&claim("{}")), None);
    assert_eq!(role_from_claim(&claim("[]")), None);
    assert_eq!(
        role_from_claim_value(&claim("\"proxy_admin\"")),
        Some(UserRole::ProxyAdmin)
    );
    assert_eq!(role_from_claim_value(&claim(r#"["proxy_admin"]"#)), None);
    assert_eq!(role_from_claim_value(&claim("3.5")), None);
}

#[test]
fn role_names_are_recognized_in_any_case_but_only_by_wire_value() {
    let names = [
        "proxy_admin",
        "proxy_admin_viewer",
        "org_admin",
        "internal_user",
        "internal_user_viewer",
        "team",
        "customer",
    ];
    assert!(names.into_iter().all(is_valid_litellm_user_role));
    assert!(
        names
            .into_iter()
            .all(|name| is_valid_litellm_user_role(&name.to_uppercase()))
    );
    assert!(names.into_iter().all(|name| role_from_name(name).is_some()));
    assert!(!is_valid_litellm_user_role("proxy_admin_view_only"));
    assert!(!is_valid_litellm_user_role("proxy_admin_viewer "));
    assert!(!is_valid_litellm_user_role(""));
    assert_eq!(role_from_name("CUSTOMER"), Some(UserRole::Customer));
    assert_eq!(role_from_name("admin_only"), None);
}

#[test]
fn has_admin_ui_access_covers_only_the_two_admin_roles() {
    assert!(has_admin_ui_access("proxy_admin"));
    assert!(has_admin_ui_access("proxy_admin_viewer"));
    assert!(!has_admin_ui_access("internal_user"));
    assert!(!has_admin_ui_access("internal_user_viewer"));
    assert!(!has_admin_ui_access("org_admin"));
    assert!(!has_admin_ui_access("team"));
    assert!(!has_admin_ui_access("customer"));
    assert!(!has_admin_ui_access("proxy_admin_view_only"));
    assert!(!has_admin_ui_access("PROXY_ADMIN"));
    assert!(!has_admin_ui_access(""));
}

#[test]
fn an_empty_settings_record_leaves_every_field_absent() {
    let config = decode_sso_record(b"{}").unwrap();
    assert_eq!(
        serde_json::to_value(&config).unwrap(),
        serde_json::to_value(SsoConfig::default()).unwrap()
    );
    assert!(config.google_client_id.is_none());
    assert!(config.google_client_secret.is_none());
    assert!(config.ui_access_mode.is_none());
    assert!(config.role_mappings.is_none());
    assert!(config.team_mappings.is_none());
}

#[test]
fn a_settings_record_keeps_absent_optional_fields_out() {
    let config =
        decode_sso_record(br#"{"google_client_id":"google-id","proxy_base_url":"/litellm"}"#)
            .unwrap();
    assert_eq!(config.google_client_id.as_deref(), Some("google-id"));
    assert_eq!(config.proxy_base_url.as_deref(), Some("/litellm"));
    assert!(config.google_client_secret.is_none());
    assert!(config.microsoft_client_id.is_none());
    assert!(config.saml_allow_unsolicited.is_none());
    assert!(config.role_mappings.is_none());
}

#[test]
fn a_settings_record_reads_every_documented_key() {
    let config = decode_sso_record(
        br#"{
            "google_client_id": "gid",
            "google_client_secret": "gsecret",
            "microsoft_client_id": "mid",
            "microsoft_client_secret": "msecret",
            "microsoft_tenant": "tenant",
            "generic_client_id": "genid",
            "generic_client_secret": "gensecret",
            "generic_authorization_endpoint": "https://idp.example/authorize",
            "generic_token_endpoint": "https://idp.example/token",
            "generic_userinfo_endpoint": "https://idp.example/userinfo",
            "generic_scope": "openid email profile",
            "saml_idp_metadata_url": "https://idp.example/metadata",
            "saml_idp_metadata_xml": "<xml/>",
            "saml_sp_entity_id": "sp",
            "saml_allow_unsolicited": "true",
            "proxy_base_url": "https://proxy.example",
            "user_email": "admin@example.com"
        }"#,
    )
    .unwrap();
    assert_eq!(config.microsoft_tenant.as_deref(), Some("tenant"));
    assert_eq!(
        config.generic_scope.as_deref(),
        Some("openid email profile")
    );
    assert_eq!(
        config.generic_authorization_endpoint.as_deref(),
        Some("https://idp.example/authorize")
    );
    assert_eq!(config.saml_allow_unsolicited.as_deref(), Some("true"));
    assert_eq!(config.user_email.as_deref(), Some("admin@example.com"));
    let written = serde_json::to_value(&config).unwrap();
    assert_eq!(written["saml_sp_entity_id"], "sp");
    assert_eq!(written["microsoft_client_secret"], "msecret");
}

#[test]
fn an_unknown_settings_key_adds_no_field() {
    let config =
        decode_sso_record(br#"{"unknown_setting":"value","google_client_id":"google-id"}"#)
            .unwrap();
    assert_eq!(config.google_client_id.as_deref(), Some("google-id"));
    let written = serde_json::to_value(&config).unwrap();
    assert!(written.get("unknown_setting").is_none());
}

#[test]
fn a_settings_record_redacts_secrets_in_debug_and_writes_them_for_storage() {
    let config = decode_sso_record(
        br#"{"google_client_secret":"google-secret","microsoft_client_secret":"microsoft-secret","generic_client_secret":"generic-secret"}"#,
    )
    .unwrap();
    let debug = format!("{config:?}");
    assert!(!debug.contains("google-secret"));
    assert!(!debug.contains("microsoft-secret"));
    assert!(!debug.contains("generic-secret"));
    assert_eq!(
        config
            .google_client_secret
            .as_ref()
            .expect("the google secret")
            .expose_secret(),
        "google-secret"
    );
    let written = serde_json::to_string(&config).unwrap();
    assert!(written.contains("google-secret"));
    assert!(written.contains("microsoft-secret"));
    assert!(written.contains("generic-secret"));
}

#[test]
fn role_mappings_start_with_no_roles_and_no_default_role() {
    let config =
        decode_sso_record(br#"{"role_mappings":{"provider":"google","group_claim":"groups"}}"#)
            .unwrap();
    let mappings: RoleMappings = config.role_mappings.unwrap();
    assert_eq!(mappings.provider, "google");
    assert_eq!(mappings.group_claim, "groups");
    assert_eq!(mappings.default_role, None);
    assert!(mappings.roles.entries.is_empty());
    let written = serde_json::to_value(&mappings).unwrap();
    assert_eq!(written["roles"], serde_json::json!({}));
    assert_eq!(written["default_role"], serde_json::json!(null));
}

#[test]
fn role_mappings_read_groups_by_their_role_value() {
    let config = decode_sso_record(
        br#"{"role_mappings":{"provider":"microsoft","group_claim":"groups","default_role":"internal_user","roles":{"proxy_admin":["admins"],"team":["team-a","team-b"]}}}"#,
    )
    .unwrap();
    let mappings = config.role_mappings.unwrap();
    assert_eq!(mappings.default_role, Some(UserRole::InternalUser));
    assert_eq!(
        mappings.roles.entries,
        vec![
            (UserRole::ProxyAdmin, vec!["admins".to_owned()]),
            (
                UserRole::Team,
                vec!["team-a".to_owned(), "team-b".to_owned()]
            ),
        ]
    );
    let written = serde_json::to_value(&mappings).unwrap();
    assert_eq!(written["roles"]["proxy_admin"][0], "admins");
    assert_eq!(written["roles"]["team"][1], "team-b");
}

#[test]
fn role_mappings_reject_an_unknown_role_name() {
    let error = decode_sso_record(
        br#"{"role_mappings":{"provider":"google","group_claim":"groups","roles":{"superuser":["admins"]}}}"#,
    )
    .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("role_mappings.roles"), "{message}");
    assert!(message.contains("superuser"), "{message}");
}

#[test]
fn team_mappings_read_the_configured_jwt_field() {
    let config =
        decode_sso_record(br#"{"team_mappings":{"team_ids_jwt_field":"groups.teams"}}"#).unwrap();
    let mappings: TeamMappings = config.team_mappings.unwrap();
    assert_eq!(mappings.team_ids_jwt_field.as_deref(), Some("groups.teams"));
    let empty = decode_sso_record(br#"{"team_mappings":{}}"#).unwrap();
    assert!(empty.team_mappings.unwrap().team_ids_jwt_field.is_none());
}

#[test]
fn microsoft_graph_records_read_their_wire_names() {
    let response: MicrosoftGraphGroupResponse = serde_json::from_str(
        r##"{
            "odata_context": "https://graph.example/$metadata#groups",
            "odata_nextLink": "https://graph.example/next",
            "value": [
                {
                    "odata_type": "#microsoft.graph.group",
                    "id": "g1",
                    "deletedDateTime": null,
                    "description": "proxy administrators",
                    "displayName": "Admins",
                    "roleTemplateId": "r1"
                }
            ]
        }"##,
    )
    .unwrap();
    assert_eq!(
        response.odata_context.as_deref(),
        Some("https://graph.example/$metadata#groups")
    );
    assert_eq!(
        response.odata_next_link.as_deref(),
        Some("https://graph.example/next")
    );
    let page = response.value.expect("the group page");
    assert_eq!(page[0].display_name.as_deref(), Some("Admins"));
    assert_eq!(page[0].role_template_id.as_deref(), Some("r1"));
    assert!(page[0].deleted_date_time.is_none());
    let written = serde_json::to_value(&page[0]).unwrap();
    assert_eq!(written["displayName"], "Admins");
    assert_eq!(written["deletedDateTime"], serde_json::json!(null));
    let principal: MicrosoftServicePrincipalTeam =
        serde_json::from_str(r#"{"principalDisplayName":"Admins","principalId":"p1"}"#).unwrap();
    assert_eq!(principal.principal_display_name.as_deref(), Some("Admins"));
    assert_eq!(principal.principal_id.as_deref(), Some("p1"));
    let empty: MicrosoftGraphGroupResponse = serde_json::from_str("{}").unwrap();
    assert!(empty.value.is_none());
}

#[test]
fn a_restricted_access_mode_object_never_reads_as_admin_only() {
    let config = decode_sso_record(
        br#"{"ui_access_mode":{"type":"restricted_sso_group","restricted_sso_group":"litellm-admins","sso_group_jwt_field":"groups"}}"#,
    )
    .unwrap();
    let mode = config.ui_access_mode.unwrap();
    assert_eq!(
        mode,
        UiAccessMode::Restricted(AccessControlUiAccessMode {
            mode_type: UiAccessModeType::RestrictedSsoGroup,
            restricted_sso_group: "litellm-admins".to_owned(),
            sso_group_jwt_field: "groups".to_owned(),
        })
    );
    assert!(!check_is_admin_only_access(&mode));
    let written = serde_json::to_value(&mode).unwrap();
    assert_eq!(written["type"], "restricted_sso_group");
}

#[test]
fn only_the_bare_string_admin_only_reads_as_admin_only() {
    let plain = decode_sso_record(br#"{"ui_access_mode":"admin_only"}"#).unwrap();
    assert!(check_is_admin_only_access(&plain.ui_access_mode.unwrap()));
    let upper = decode_sso_record(br#"{"ui_access_mode":"ADMIN_ONLY"}"#).unwrap();
    assert!(!check_is_admin_only_access(&upper.ui_access_mode.unwrap()));
    let padded = decode_sso_record(br#"{"ui_access_mode":" admin_only"}"#).unwrap();
    assert!(!check_is_admin_only_access(&padded.ui_access_mode.unwrap()));
    let other = decode_sso_record(br#"{"ui_access_mode":"restricted_sso_group"}"#).unwrap();
    assert!(!check_is_admin_only_access(&other.ui_access_mode.unwrap()));
}

#[test]
fn an_access_mode_object_with_another_type_is_refused() {
    let error = decode_sso_record(
        br#"{"ui_access_mode":{"type":"open_ui","restricted_sso_group":"admins","sso_group_jwt_field":"groups"}}"#,
    )
    .unwrap_err();
    assert!(error.to_string().contains("ui_access_mode"));
}

#[test]
fn default_team_params_start_with_no_models_and_no_limits() {
    let params: DefaultTeamSsoParams = serde_json::from_str("{}").unwrap();
    assert!(params.models.is_empty());
    assert!(params.max_budget.is_none());
    assert!(params.budget_duration.is_none());
    assert!(params.tpm_limit.is_none());
    assert!(params.rpm_limit.is_none());
    assert!(params.team_member_permissions.is_none());
    assert!(params.organization_id.is_none());
}

#[test]
fn default_team_params_read_a_float_budget_and_route_paths() {
    let params: DefaultTeamSsoParams = serde_json::from_str(
        r#"{"models":["gpt-4o"],"max_budget":25.5,"budget_duration":"30d","tpm_limit":100,"rpm_limit":10,"team_member_permissions":["/key/generate"],"organization_id":"org-1"}"#,
    )
    .unwrap();
    assert_eq!(params.models, vec!["gpt-4o".to_owned()]);
    assert_eq!(params.max_budget, Some("25.5".parse::<Decimal>().unwrap()));
    assert_eq!(params.budget_duration.as_deref(), Some("30d"));
    assert_eq!(params.tpm_limit, Some(100));
    assert_eq!(
        params.team_member_permissions,
        Some(vec!["/key/generate".to_owned()])
    );
    assert_eq!(params.organization_id.as_deref(), Some("org-1"));
    let written = serde_json::to_value(&params).unwrap();
    assert_eq!(written["max_budget"], 25.5);
    let open_routes: DefaultTeamSsoParams =
        serde_json::from_str(r#"{"team_member_permissions":["/not/a/key/route"]}"#).unwrap();
    assert_eq!(
        open_routes.team_member_permissions,
        Some(vec!["/not/a/key/route".to_owned()])
    );
}

#[test]
fn upperbound_key_params_default_to_no_limits_and_read_a_budget() {
    let params: UpperboundKeyGenerateParams =
        serde_json::from_str(r#"{"max_budget":5,"duration":"1d"}"#).unwrap();
    assert_eq!(params.max_budget, Some("5".parse::<Decimal>().unwrap()));
    assert_eq!(params.duration.as_deref(), Some("1d"));
    assert!(params.budget_duration.is_none());
    assert!(params.max_parallel_requests.is_none());
    assert!(params.tpm_limit.is_none());
    assert!(params.rpm_limit.is_none());
    let written = serde_json::to_value(&params).unwrap();
    assert_eq!(written["max_budget"], 5.0);
}

#[test]
fn a_ui_token_object_redacts_its_key_in_debug_and_writes_it_to_the_response() {
    let token: ReturnedUiTokenObject = serde_json::from_str(
        r#"{"user_id":"u1","key":"sk-virtual","user_email":null,"user_role":"proxy_admin","login_method":"sso","premium_user":true,"auth_header_name":"Authorization","disabled_non_admin_personal_key_creation":false,"server_root_path":"/litellm","password_reset_required":false}"#,
    )
    .unwrap();
    assert_eq!(token.key.expose_secret(), "sk-virtual");
    assert_eq!(token.login_method, LoginMethod::Sso);
    assert!(token.premium_user.0);
    assert!(!token.personal_key_creation_disabled.0);
    assert!(!token.password_reset_required.0);
    assert!(!format!("{token:?}").contains("sk-virtual"));
    let written = serde_json::to_string(&token).unwrap();
    assert!(written.contains("sk-virtual"));
    assert!(written.contains("disabled_non_admin_personal_key_creation"));
    assert!(written.contains("\"login_method\":\"sso\""));
}

#[test]
fn a_ui_token_object_reads_the_password_login_method() {
    let token: ReturnedUiTokenObject = serde_json::from_str(
        r#"{"user_id":"u1","key":"sk-virtual","user_email":"user@example.com","user_role":"internal_user","login_method":"username_password","premium_user":false,"auth_header_name":"Authorization","disabled_non_admin_personal_key_creation":true,"server_root_path":"/litellm","password_reset_required":true}"#,
    )
    .unwrap();
    assert_eq!(token.login_method, LoginMethod::UsernamePassword);
    assert_eq!(token.user_email.as_deref(), Some("user@example.com"));
    assert!(token.personal_key_creation_disabled.0);
    assert!(token.password_reset_required.0);
    let written = serde_json::to_string(&token).unwrap();
    assert!(written.contains("\"login_method\":\"username_password\""));
}

#[test]
fn a_parsed_open_id_result_may_carry_no_fields() {
    let empty: ParsedOpenIdResult = serde_json::from_str("{}").unwrap();
    assert_eq!(empty.user_email, None);
    assert_eq!(empty.user_id, None);
    assert_eq!(empty.user_role, None);
    let filled: ParsedOpenIdResult = serde_json::from_str(
        r#"{"user_email":"user@example.com","user_id":"u1","user_role":"internal_user"}"#,
    )
    .unwrap();
    assert_eq!(filled.user_email.as_deref(), Some("user@example.com"));
    assert_eq!(filled.user_id.as_deref(), Some("u1"));
    assert_eq!(filled.user_role.as_deref(), Some("internal_user"));
}

#[test]
fn an_identity_record_requires_its_team_ids() {
    let error = decode_open_id_identity(br#"{"id":"u1","email":"user@example.com"}"#).unwrap_err();
    assert!(error.to_string().contains("team_ids"), "{error}");
    let identity: OpenIdIdentity =
        decode_open_id_identity(br#"{"id":"u1","team_ids":[]}"#).unwrap();
    assert_eq!(identity.id.as_deref(), Some("u1"));
    assert!(identity.email.is_none());
    assert!(identity.user_role.is_none());
    assert!(identity.extra_fields.is_none());
}

#[test]
fn an_identity_record_keeps_an_address_litellm_would_refuse_as_text() {
    let identity = decode_open_id_identity(br#"{"team_ids":[],"email":"not-an-address"}"#).unwrap();
    assert_eq!(identity.email.as_deref(), Some("not-an-address"));
}

#[test]
fn an_identity_record_reads_its_role_by_wire_value() {
    let identity =
        decode_open_id_identity(br#"{"team_ids":["t1"],"user_role":"proxy_admin_viewer"}"#)
            .unwrap();
    assert_eq!(identity.user_role, Some(UserRole::ProxyAdminViewer));
    assert_eq!(identity.team_ids, vec!["t1".to_owned()]);
    let error = decode_open_id_identity(br#"{"team_ids":[],"user_role":"proxy_admin_view_only"}"#)
        .unwrap_err();
    assert!(error.to_string().contains("user_role"), "{error}");
}

#[test]
fn an_identity_record_keeps_extra_claims_typed() {
    let identity = decode_open_id_identity(
        br#"{"team_ids":[],"extra_fields":{"groups":["a"],"count":3,"profile":{"active":true}},"unknown_claim":"dropped"}"#,
    )
    .unwrap();
    let extra = identity.extra_fields.expect("extra fields");
    assert!(extra["groups"].json.is_array());
    assert_eq!(extra["count"].json.as_i64(), Some(3));
    assert_eq!(extra["profile"].json["active"].as_bool(), Some(true));
    assert!(!extra.contains_key("unknown_claim"));
}

#[test]
fn the_bearer_fields_are_the_three_oauth_credentials() {
    assert_eq!(
        OAUTH_TOKEN_FIELDS,
        ["access_token", "id_token", "refresh_token"]
    );
    let claims = ["token_type", "expires_in", "scope", "access_token"];
    assert_eq!(
        claims
            .into_iter()
            .filter(|name| OAUTH_TOKEN_FIELDS.contains(name))
            .collect::<Vec<&str>>(),
        vec!["access_token"]
    );
}

#[test]
fn a_record_over_the_size_limit_is_refused_before_parsing() {
    let oversized = vec![b' '; MAX_SSO_RECORD_BYTES + 1];
    assert!(matches!(
        decode_sso_record(&oversized),
        Err(SsoRecordError::TooLarge { .. })
    ));
    assert!(matches!(
        decode_open_id_identity(&oversized),
        Err(SsoRecordError::TooLarge { .. })
    ));
    let mut at_limit = vec![b' '; MAX_SSO_RECORD_BYTES - 2];
    at_limit.push(b'{');
    at_limit.push(b'}');
    assert_eq!(at_limit.len(), MAX_SSO_RECORD_BYTES);
    assert!(decode_sso_record(&at_limit).is_ok());
}

#[test]
fn a_record_with_bytes_after_its_json_is_refused() {
    assert!(matches!(
        decode_sso_record(b"{} trailing"),
        Err(SsoRecordError::Trailing(_))
    ));
    assert!(decode_sso_record(b"{}  \n").is_ok());
}

#[test]
fn a_record_that_is_not_a_json_object_is_refused() {
    assert!(decode_sso_record(b"[]").is_err());
    assert!(decode_sso_record(b"\"text\"").is_err());
    assert!(decode_sso_record(b"null").is_err());
    assert!(decode_sso_record(b"").is_err());
    assert!(decode_sso_record(b"{").is_err());
}

#[test]
fn a_record_that_names_a_bad_field_reports_its_path() {
    let error = decode_sso_record(
        br#"{"role_mappings":{"provider":"google","group_claim":"groups","roles":[]}}"#,
    )
    .unwrap_err();
    assert!(error.to_string().contains("role_mappings.roles"), "{error}");
}

#[test]
fn a_kind_tag_selects_the_record_the_bytes_decode_into() {
    let settings = SsoRecordKind::try_from(0).expect("the settings tag");
    assert!(matches!(
        settings.decode(b"{}"),
        Ok(SsoRecord::Settings(_config))
    ));
    let identity = SsoRecordKind::try_from(1).expect("the identity tag");
    assert!(matches!(
        identity.decode(br#"{"team_ids":[]}"#),
        Ok(SsoRecord::Identity(_identity))
    ));
    assert!(matches!(
        SsoRecordKind::try_from(2),
        Err(SsoRecordError::UnknownKind { tag: 2 })
    ));
    let wrong_shape = SsoRecordKind::try_from(1).expect("the identity tag");
    assert!(wrong_shape.decode(b"{}").is_err());
}

#[rstest::rstest]
fn replays_the_sso_record_corpus(#[files("fuzz/corpus/sso_record/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let Some((&tag, payload)) = bytes.split_first() else {
        return;
    };
    if let Ok(kind) = SsoRecordKind::try_from(tag) {
        drop(kind.decode(payload));
    }
}

#[test]
fn the_sso_record_corpus_reaches_both_record_kinds() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/sso_record");
    let kinds = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| fs::read(entry.unwrap().path()).unwrap())
        .filter_map(|bytes| bytes.first().copied())
        .filter_map(|tag| SsoRecordKind::try_from(tag).ok())
        .collect::<Vec<SsoRecordKind>>();
    assert!(kinds.contains(&SsoRecordKind::Settings));
    assert!(kinds.contains(&SsoRecordKind::Identity));
}
