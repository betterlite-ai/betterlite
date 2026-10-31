#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::{
    ApiKeySecret, AuthError, AuthInput, ParentOtelSpan, UserAPIKeyAuth, is_team_service_account,
    user_api_key_has_admin_view,
};

fn parse(text: &str) -> JsonData {
    serde_json::from_str(text).unwrap_or_else(|error| panic!("invalid JSON fixture: {error}"))
}

fn object(text: &str) -> BTreeMap<String, JsonData> {
    match parse(text) {
        JsonData::Object(fields) => fields,
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::String(_)
        | JsonData::Array(_) => panic!("fixture must be a JSON object"),
    }
}

fn field(auth: &BTreeMap<String, JsonData>, name: &str) -> JsonData {
    auth.get(name).cloned().unwrap_or(JsonData::Null)
}

fn construct(text: &str) -> UserAPIKeyAuth {
    UserAPIKeyAuth::construct(AuthInput::Dict(object(text)))
        .unwrap_or_else(|error| panic!("fixture should construct: {error}"))
}

fn serialized(auth: &UserAPIKeyAuth) -> BTreeMap<String, JsonData> {
    object(&serde_json::to_string(auth).unwrap_or_else(|error| panic!("serialize failed: {error}")))
}

fn auth_from(text: &str) -> BTreeMap<String, JsonData> {
    serialized(&construct(text))
}

fn exposed_api_key(auth: &UserAPIKeyAuth) -> Option<String> {
    auth.extra
        .api_key
        .0
        .as_ref()
        .map(|secret| secret.expose_secret().to_owned())
}

fn exposed_token(auth: &UserAPIKeyAuth) -> Option<String> {
    auth.view
        .token
        .token
        .as_ref()
        .map(|secret| secret.expose_secret().to_owned())
}

#[rstest]
#[case::auth_field_api_key("api_key", "\"plainkey\"")]
#[case::auth_field_user_role("user_role", "\"proxy_admin\"")]
#[case::auth_field_allowed_model_region("allowed_model_region", "\"eu\"")]
#[case::auth_field_parent_otel_span("parent_otel_span", "{\"trace_id\": \"t1\"}")]
#[case::auth_field_rpm_limit_per_model("rpm_limit_per_model", "{\"gpt-4\": 5}")]
#[case::auth_field_tpm_limit_per_model("tpm_limit_per_model", "{\"gpt-4\": 5}")]
#[case::auth_field_user_tpm_limit("user_tpm_limit", "100")]
#[case::auth_field_user_rpm_limit("user_rpm_limit", "100")]
#[case::auth_field_user_email("user_email", "\"a@b.com\"")]
#[case::auth_field_user_spend("user_spend", "12.5")]
#[case::auth_field_user_max_budget("user_max_budget", "50.0")]
#[case::auth_field_user_model_max_budget("user_model_max_budget", "{\"gpt-4\": 1.0}")]
#[case::auth_field_request_route("request_route", "\"/chat/completions\"")]
#[case::auth_field_user("user", "{\"id\": \"u1\"}")]
#[case::auth_field_created_by_user("created_by_user", "{\"id\": \"creator\"}")]
#[case::auth_field_end_user_object_permission(
    "end_user_object_permission",
    "{\"object_permission_id\": \"perm-1\", \"mcp_servers\": [], \"mcp_access_groups\": [], \
     \"mcp_tool_permissions\": null, \"vector_stores\": [], \"agents\": [], \"agent_access_groups\": [], \
     \"models\": [], \"mcp_toolsets\": null, \"blocked_tools\": [], \"search_tools\": [], \
     \"mcp_tool_search_enabled\": null, \"skills\": null}"
)]
#[case::auth_field_team_object_permission(
    "team_object_permission",
    "{\"object_permission_id\": \"perm-2\", \"mcp_servers\": [], \"mcp_access_groups\": [], \
     \"mcp_tool_permissions\": null, \"vector_stores\": [], \"agents\": [], \"agent_access_groups\": [], \
     \"models\": [], \"mcp_toolsets\": null, \"blocked_tools\": [], \"search_tools\": [], \
     \"mcp_tool_search_enabled\": null, \"skills\": null}"
)]
#[case::auth_field_jwt_claims("jwt_claims", "{\"sub\": \"u1\"}")]
fn auth_field_cases(#[case] field_name: &str, #[case] value_text: &str) {
    let value = parse(value_text);
    assert_eq!(field(&auth_from("{}"), field_name), JsonData::Null);
    assert_eq!(
        field(
            &auth_from(&format!("{{\"{field_name}\": null}}")),
            field_name
        ),
        JsonData::Null
    );
    let input = format!("{{\"{field_name}\": {value_text}}}");
    assert_eq!(field(&auth_from(&input), field_name), value);
}

#[test]
fn auth_field_is_session_token() {
    assert_eq!(field(&auth_from("{}"), "is_session_token"), parse("false"));
    assert_eq!(
        field(
            &auth_from("{\"is_session_token\": true}"),
            "is_session_token"
        ),
        parse("true")
    );
}

#[test]
fn session_marker_serialized() {
    let output = auth_from("{\"is_session_token\": true}");
    assert_eq!(field(&output, "is_session_token"), parse("true"));
}

#[rstest]
#[case::eu("eu", true)]
#[case::us("us", true)]
#[case::global("global", false)]
#[case::uppercase_eu("EU", false)]
fn allowed_region_eu_us_only(#[case] region: &str, #[case] accepted: bool) {
    let outcome = UserAPIKeyAuth::construct(AuthInput::Dict(object(&format!(
        "{{\"allowed_model_region\": \"{region}\"}}"
    ))));
    assert_eq!(outcome.is_ok(), accepted);
}

#[test]
fn per_model_limits_integer_values() {
    let auth = construct("{\"rpm_limit_per_model\": {\"gpt-4\": 5}}");
    assert_eq!(
        field(&serialized(&auth), "rpm_limit_per_model"),
        parse("{\"gpt-4\": 5}")
    );
    let error = UserAPIKeyAuth::construct(AuthInput::Dict(object(
        "{\"rpm_limit_per_model\": {\"gpt-4\": 1.5}}",
    )))
    .unwrap_err();
    assert!(matches!(error, AuthError::Json(_)));
}

#[test]
fn malformed_unselected_user_budget_entry_retained() {
    let auth = construct("{\"user_model_max_budget\": {\"gpt-4\": \"not-a-number\"}}");
    assert_eq!(
        field(&serialized(&auth), "user_model_max_budget"),
        parse("{\"gpt-4\": \"not-a-number\"}")
    );
}

type MarkerCheck = fn(&UserAPIKeyAuth) -> bool;

const MARKER_CHECKS: [(&str, MarkerCheck); 15] = [
    ("mcp_admitted_user_subject", |key| {
        !key.extra.mcp_access.mcp_admitted_user_subject
    }),
    ("requires_fresh_policy", |key| {
        !key.extra.requires_fresh_policy
    }),
    ("mcp_explicit_grants_only", |key| {
        !key.extra.mcp_access.mcp_explicit_grants_only
    }),
    ("mcp_source_team_rpm_limits", |key| {
        key.extra.mcp_source_team_rpm_limits.is_none()
    }),
    ("mcp_session_resource_server_id", |key| {
        key.extra.mcp_session_resource_server_id.is_none()
    }),
    ("mcp_toolset_id", |key| key.extra.mcp_toolset_id.is_none()),
    ("via_virtual_key", |key| {
        !key.extra.provenance.via_virtual_key
    }),
    ("authenticated_by_custom_auth", |key| {
        !key.extra.provenance.authenticated_by_custom_auth
    }),
    ("agent_caller", |key| key.extra.agent_caller.is_none()),
    ("managed_agent_context", |key| {
        key.extra.managed_agent_context.is_none()
    }),
    ("managed_agent_policy", |key| {
        key.extra.managed_agent_policy.is_none()
    }),
    ("invoked_agent_id", |key| {
        key.extra.invoked_agent_id.is_none()
    }),
    ("invoked_agent_policy", |key| {
        key.extra.invoked_agent_policy.is_none()
    }),
    ("agent_invocation_cost", |key| {
        key.extra.agent_invocation_cost.is_none()
    }),
    ("billing_agent_policy", |key| {
        key.extra.billing_agent_policy.is_none()
    }),
];

fn assert_marker_cleared(auth: &UserAPIKeyAuth, field_name: &str) {
    let check = MARKER_CHECKS
        .into_iter()
        .find_map(|(name, check)| (name == field_name).then_some(check))
        .unwrap_or_else(|| panic!("unexpected stripped field name {field_name}"));
    assert!(check(auth), "{field_name} marker was not cleared");
}

#[rstest]
#[case::mcp_admitted_user_subject("mcp_admitted_user_subject")]
#[case::requires_fresh_policy("requires_fresh_policy")]
#[case::mcp_explicit_grants_only("mcp_explicit_grants_only")]
#[case::mcp_source_team_rpm_limits("mcp_source_team_rpm_limits")]
#[case::mcp_session_resource_server_id("mcp_session_resource_server_id")]
#[case::mcp_toolset_id("mcp_toolset_id")]
#[case::via_virtual_key("via_virtual_key")]
#[case::authenticated_by_custom_auth("authenticated_by_custom_auth")]
#[case::agent_caller("agent_caller")]
#[case::managed_agent_context("managed_agent_context")]
#[case::managed_agent_policy("managed_agent_policy")]
#[case::invoked_agent_id("invoked_agent_id")]
#[case::invoked_agent_policy("invoked_agent_policy")]
#[case::agent_invocation_cost("agent_invocation_cost")]
#[case::billing_agent_policy("billing_agent_policy")]
fn each_untrusted_marker_stripped(#[case] field_name: &str) {
    let input = format!("{{\"{field_name}\": true}}");
    let auth = construct(&input);
    assert!(!serialized(&auth).contains_key(field_name));
    assert_marker_cleared(&auth, field_name);
}

#[test]
fn server_markers_survive_trusted_assignment() {
    let mut auth = construct("{}");
    auth.extra.provenance.via_virtual_key = true;
    assert!(auth.extra.provenance.via_virtual_key);
    assert!(!serialized(&auth).contains_key("via_virtual_key"));
}

#[test]
fn excluded_but_not_stripped_fields_accepted() {
    let auth = construct("{\"budget_reservation\": {\"k\": \"v\"}}");
    assert!(auth.extra.budget_reservation.is_some());
    assert!(!serialized(&auth).contains_key("budget_reservation"));
}

#[rstest]
#[case::mcp_admitted_user_subject("mcp_admitted_user_subject")]
#[case::requires_fresh_policy("requires_fresh_policy")]
#[case::mcp_explicit_grants_only("mcp_explicit_grants_only")]
#[case::authenticated_by_custom_auth("authenticated_by_custom_auth")]
#[case::via_virtual_key("via_virtual_key")]
#[case::mcp_source_team_rpm_limits("mcp_source_team_rpm_limits")]
#[case::mcp_session_resource_server_id("mcp_session_resource_server_id")]
#[case::mcp_toolset_id("mcp_toolset_id")]
#[case::invoked_agent_id("invoked_agent_id")]
#[case::invoked_agent_policy("invoked_agent_policy")]
#[case::agent_invocation_cost("agent_invocation_cost")]
#[case::billing_agent_policy("billing_agent_policy")]
#[case::managed_agent_policy("managed_agent_policy")]
#[case::managed_agent_context("managed_agent_context")]
#[case::agent_caller("agent_caller")]
#[case::budget_reservation("budget_reservation")]
#[case::team_budget_snapshot("team_budget_snapshot")]
#[case::user_budget_snapshot("user_budget_snapshot")]
#[case::org_budget_snapshot("org_budget_snapshot")]
#[case::matched_model_access_groups("matched_model_access_groups")]
#[case::budget_throttle_pct("budget_throttle_pct")]
fn every_server_field_excluded(#[case] field_name: &str) {
    let output = serialized(&construct("{}"));
    assert!(
        !output.contains_key(field_name),
        "{field_name} must not serialize"
    );
}

#[test]
fn private_delegation_flag_not_serialized() {
    let mut auth = construct("{}");
    assert!(!auth.take_delegation_verified());
    auth.managed_delegation_verified = true;
    assert!(!serialized(&auth).contains_key("managed_delegation_verified"));
    assert!(auth.take_delegation_verified());
    assert!(!auth.take_delegation_verified());
}

#[test]
fn existing_model_instance_retained() {
    let mut auth = construct("{\"api_key\": \"sk-abc\"}");
    auth.extra.api_key = ApiKeySecret(Some(SecretString::from("sk-mutated")));
    let rebuilt = UserAPIKeyAuth::construct(AuthInput::Instance(Box::new(auth.clone())))
        .unwrap_or_else(|error| panic!("instance passthrough should not fail: {error}"));
    assert_eq!(exposed_api_key(&rebuilt), exposed_api_key(&auth));
}

#[test]
fn api_key_overwrites_token() {
    let hashed = super::super::credential::safe_hash_api_key("sk-abc");
    let auth = construct("{\"api_key\": \"sk-abc\", \"token\": \"stale\"}");
    assert_eq!(exposed_api_key(&auth), Some(hashed.clone()));
    assert_eq!(exposed_token(&auth), Some(hashed));
}

#[test]
fn api_key_none_preserves_token() {
    let auth = construct("{\"api_key\": null, \"token\": \"original\"}");
    assert_eq!(exposed_token(&auth), Some("original".to_owned()));
}

#[test]
fn empty_api_key_overwrites_token() {
    let auth = construct("{\"api_key\": \"\", \"token\": \"stale\"}");
    assert_eq!(exposed_api_key(&auth), Some(String::new()));
    assert_eq!(exposed_token(&auth), Some(String::new()));
}

#[test]
fn nonstring_api_key_not_stringified() {
    let error =
        UserAPIKeyAuth::construct(AuthInput::Dict(object("{\"api_key\": 123}"))).unwrap_err();
    assert!(matches!(error, AuthError::NonStringApiKey));
}

#[test]
fn serialized_span_json_value_preserved() {
    let auth = construct("{\"parent_otel_span\": {\"trace_id\": \"abc\"}}");
    assert_eq!(
        field(&serialized(&auth), "parent_otel_span"),
        parse("{\"trace_id\": \"abc\"}")
    );
}

#[test]
fn serialized_span_requires_caller_cleanup() {
    let mut auth = construct("{}");
    auth.extra.parent_otel_span = Some(ParentOtelSpan::Opaque);
    serde_json::to_string(&auth).unwrap_err();
}

#[test]
fn unknown_auth_extras_ignored() {
    let auth = construct("{\"something_brand_new\": \"x\"}");
    assert!(!serialized(&auth).contains_key("something_brand_new"));
}

#[rstest]
#[case::no_team_id(
    "{\"user_id\": null, \"metadata\": {\"service_account_id\": \"svc\"}}",
    false
)]
#[case::has_user_id(
    "{\"user_id\": \"u1\", \"team_id\": \"t1\", \"metadata\": {\"service_account_id\": \"svc\"}}",
    false
)]
#[case::empty_metadata("{\"user_id\": null, \"team_id\": \"t1\", \"metadata\": {}}", false)]
#[case::metadata_without_marker(
    "{\"user_id\": null, \"team_id\": \"t1\", \"metadata\": {\"other\": 1}}",
    false
)]
#[case::present_marker(
    "{\"user_id\": null, \"team_id\": \"t1\", \"metadata\": {\"service_account_id\": \"svc\"}}",
    true
)]
fn service_account_none_vs_empty_fields(#[case] fields: &str, #[case] expected: bool) {
    assert_eq!(is_team_service_account(&construct(fields)), expected);
}

#[test]
fn service_account_false_value_is_present() {
    let fields =
        "{\"user_id\": null, \"team_id\": \"t1\", \"metadata\": {\"service_account_id\": false}}";
    assert!(is_team_service_account(&construct(fields)));
}

#[rstest]
#[case::proxy_admin("proxy_admin", true)]
#[case::proxy_admin_viewer("proxy_admin_viewer", true)]
#[case::org_admin("org_admin", false)]
#[case::internal_user("internal_user", false)]
#[case::team("team", false)]
#[case::customer("customer", false)]
#[case::absent_role("", false)]
fn admin_view_admin_and_viewer_only(#[case] role: &str, #[case] expected: bool) {
    let fields = if role.is_empty() {
        "{}".to_owned()
    } else {
        format!("{{\"user_role\": \"{role}\"}}")
    };
    assert_eq!(user_api_key_has_admin_view(&construct(&fields)), expected);
}
