#![cfg(test)]

use std::collections::BTreeMap;

use axum::http::StatusCode;
use betterlite_analytics::canonical_date::{ScopeDenial, ScopeDenied};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::user_roles::UserRole;
use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;

use super::error::UserActivityScopeError;
use super::resolve_user_daily_activity_entity_ids;
use crate::management_guards::error::ManagementGuardError;

const NON_ADMIN_REASON: &str = "Non-admin users can only view their own spend data.";
const SERVICE_ACCOUNT_REASON: &str = "Service-account keys cannot query user analytics. Use a user-bound key, or call as a proxy admin.";

fn caller(user_id: Option<&str>, user_role: Option<UserRole>) -> UserAPIKeyAuth {
    let mut fields = BTreeMap::new();
    if let Some(id) = user_id {
        fields.insert("user_id".to_owned(), JsonData::String(id.to_owned()));
    }
    if let Some(role) = user_role {
        fields.insert("user_role".to_owned(), JsonData::String(role.to_string()));
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap()
}

#[rstest]
#[case::proxy_admin_absent(None, UserRole::ProxyAdmin, None, None)]
#[case::proxy_admin_another_user(None, UserRole::ProxyAdmin, Some("test-user-2"), Some(vec!["test-user-2".to_owned()]))]
#[case::proxy_admin_for_an_owned_key(Some("test-user-1"), UserRole::ProxyAdmin, Some("test-user-2"), Some(vec!["test-user-2".to_owned()]))]
#[case::proxy_admin_empty_string(Some("test-user-1"), UserRole::ProxyAdmin, Some(""), Some(vec![String::new()]))]
#[case::proxy_admin_viewer_absent(None, UserRole::ProxyAdminViewer, None, None)]
#[case::proxy_admin_viewer_another_user(Some("test-user-1"), UserRole::ProxyAdminViewer, Some("test-user-2"), Some(vec!["test-user-2".to_owned()]))]
#[case::proxy_admin_viewer_empty_string(None, UserRole::ProxyAdminViewer, Some(""), Some(vec![String::new()]))]
fn admin_selects_every_user_or_exactly_the_requested_id(
    #[case] key_user_id: Option<&str>,
    #[case] user_role: UserRole,
    #[case] requested: Option<&str>,
    #[case] expected: Option<Vec<String>>,
) {
    let auth = caller(key_user_id, Some(user_role));
    let entity_ids = resolve_user_daily_activity_entity_ids(requested, &auth).unwrap();
    assert_eq!(entity_ids, expected);
}

#[rstest]
#[case::internal_user_omitted_id("test-user-1", UserRole::InternalUser, None)]
#[case::internal_user_own_id("test-user-1", UserRole::InternalUser, Some("test-user-1"))]
#[case::internal_user_viewer_omitted_id("test-user-1", UserRole::InternalUserViewer, None)]
#[case::internal_user_viewer_own_id(
    "test-user-1",
    UserRole::InternalUserViewer,
    Some("test-user-1")
)]
#[case::org_admin_omitted_id("test-user-1", UserRole::OrgAdmin, None)]
#[case::team_omitted_id("test-user-1", UserRole::Team, None)]
#[case::customer_omitted_id("test-user-1", UserRole::Customer, None)]
#[case::empty_caller_omitted_id("", UserRole::InternalUser, None)]
#[case::empty_caller_own_id("", UserRole::InternalUser, Some(""))]
fn self_scopes_non_admins_to_their_own_id(
    #[case] caller_id: &str,
    #[case] user_role: UserRole,
    #[case] requested: Option<&str>,
) {
    let auth = caller(Some(caller_id), Some(user_role));
    let entity_ids = resolve_user_daily_activity_entity_ids(requested, &auth).unwrap();
    assert_eq!(entity_ids, Some(vec![caller_id.to_owned()]));
}

#[rstest]
#[case::no_role_omitted_id(None, None)]
#[case::no_role_another_user(Some("test-user-2"), None)]
#[case::no_role_empty_string(Some(""), None)]
#[case::internal_user_role_omitted_id(None, Some(UserRole::InternalUser))]
#[case::internal_user_role_another_user(Some("test-user-2"), Some(UserRole::InternalUser))]
#[case::team_role_empty_string(Some(""), Some(UserRole::Team))]
fn denied_service_account_keys_fail_before_the_id_comparison(
    #[case] requested: Option<&str>,
    #[case] user_role: Option<UserRole>,
) {
    let auth = caller(None, user_role);
    let error = resolve_user_daily_activity_entity_ids(requested, &auth).unwrap_err();
    let UserActivityScopeError::Caller(caller_error) = error else {
        panic!("a non-admin key without a user id is a service account");
    };
    assert_eq!(caller_error, ManagementGuardError::ServiceAccountAnalytics);
    assert_eq!(caller_error.status_code(), Some(StatusCode::FORBIDDEN));
    assert_eq!(caller_error.to_string(), SERVICE_ACCOUNT_REASON);
}

#[rstest]
#[case::another_user(UserRole::InternalUser, Some("test-user-2"))]
#[case::empty_requested_id(UserRole::InternalUser, Some(""))]
#[case::case_only_difference(UserRole::InternalUser, Some("Test-User-1"))]
#[case::internal_user_viewer_another_user(UserRole::InternalUserViewer, Some("test-user-2"))]
#[case::org_admin_another_user(UserRole::OrgAdmin, Some("test-user-2"))]
#[case::customer_another_user(UserRole::Customer, Some("test-user-2"))]
fn denied_non_admins_asking_for_another_user(
    #[case] user_role: UserRole,
    #[case] requested: Option<&str>,
) {
    let auth = caller(Some("test-user-1"), Some(user_role));
    let error = resolve_user_daily_activity_entity_ids(requested, &auth).unwrap_err();
    let UserActivityScopeError::Denied(denied) = error else {
        panic!("a non-admin asking for another user is denied");
    };
    assert_eq!(
        denied,
        ScopeDenied {
            status: ScopeDenial::Forbidden,
            reason: NON_ADMIN_REASON.to_owned(),
        }
    );
}
