#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;
use secrecy::ExposeSecret;

use super::{
    SystemPrincipal, is_team_service_account, system_principal, user_api_key_has_admin_view,
};
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use crate::user_roles::UserRole;

fn object(text: &str) -> BTreeMap<String, JsonData> {
    match serde_json::from_str(text).unwrap_or_else(|error| panic!("invalid JSON fixture: {error}"))
    {
        JsonData::Object(fields) => fields,
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::String(_)
        | JsonData::Array(_) => panic!("fixture must be a JSON object"),
    }
}

fn construct(text: &str) -> UserAPIKeyAuth {
    UserAPIKeyAuth::construct(AuthInput::Dict(object(text)))
        .unwrap_or_else(|error| panic!("fixture should construct: {error}"))
}

#[test]
fn health_principal_all_fields() {
    let auth = system_principal(&SystemPrincipal::HealthCheck)
        .unwrap_or_else(|error| panic!("factory should validate: {error}"));
    assert_eq!(
        auth.extra
            .api_key
            .0
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("litellm-internal-health-check")
    );
    assert_eq!(
        auth.view.token.team_id.as_deref(),
        Some("litellm-internal-health-check")
    );
    assert_eq!(
        auth.view.token.key_alias.as_deref(),
        Some("litellm-internal-health-check")
    );
    assert_eq!(
        auth.view.team_alias.as_deref(),
        Some("litellm-internal-health-check")
    );
}

#[test]
fn jobs_principal_all_fields() {
    let auth = system_principal(&SystemPrincipal::Jobs)
        .unwrap_or_else(|error| panic!("factory should validate: {error}"));
    assert_eq!(
        auth.extra
            .api_key
            .0
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("litellm_internal_jobs")
    );
    assert_eq!(
        auth.view.token.key_alias.as_deref(),
        Some("litellm_internal_jobs")
    );
    assert_eq!(auth.view.token.team_id.as_deref(), Some("system"));
    assert_eq!(auth.view.team_alias.as_deref(), Some("system"));
    assert_eq!(auth.view.token.user_id.as_deref(), Some("system"));
    assert_eq!(auth.extra.user_role, Some(UserRole::ProxyAdmin));
}

#[test]
fn factories_not_virtual_key_provenance() {
    let health = system_principal(&SystemPrincipal::HealthCheck)
        .unwrap_or_else(|error| panic!("factory should validate: {error}"));
    let jobs = system_principal(&SystemPrincipal::Jobs)
        .unwrap_or_else(|error| panic!("factory should validate: {error}"));
    assert!(!health.extra.provenance.via_virtual_key);
    assert!(!jobs.extra.provenance.via_virtual_key);
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
