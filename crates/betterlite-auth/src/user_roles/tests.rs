#![cfg(test)]

use super::{UserApiKeyAuth, UserRole, user_api_key_has_admin_view};
use rstest::rstest;

#[rstest]
#[case::admin(0)]
#[case::admin_viewer(1)]
fn admin_view_predicate_accepts_both_admin_roles(#[case] role_number: u8) {
    let role = match role_number {
        0 => UserRole::ProxyAdmin,
        1 => UserRole::ProxyAdminViewer,
        _ => UserRole::Customer,
    };
    let api_key = UserApiKeyAuth {
        user_role: Some(role),
    };
    assert!(user_api_key_has_admin_view(&api_key));
}

#[rstest]
#[case::internal_user(Some(0))]
#[case::team(Some(1))]
#[case::missing(None)]
fn admin_view_predicate_rejects_non_admin_and_missing_roles(#[case] role_number: Option<u8>) {
    let role = role_number.map(|candidate| match candidate {
        0 => UserRole::InternalUser,
        1 => UserRole::Team,
        _ => UserRole::Customer,
    });
    let api_key = UserApiKeyAuth { user_role: role };
    assert!(!user_api_key_has_admin_view(&api_key));
}
