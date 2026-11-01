#![cfg(test)]

use super::UserRole;
use rstest::rstest;

#[rstest]
#[case::proxy_admin(UserRole::ProxyAdmin, "proxy_admin")]
#[case::proxy_admin_viewer(UserRole::ProxyAdminViewer, "proxy_admin_viewer")]
#[case::org_admin(UserRole::OrgAdmin, "org_admin")]
#[case::internal_user(UserRole::InternalUser, "internal_user")]
#[case::internal_user_viewer(UserRole::InternalUserViewer, "internal_user_viewer")]
#[case::team(UserRole::Team, "team")]
#[case::customer(UserRole::Customer, "customer")]
fn each_legacy_role_value(#[case] role: UserRole, #[case] value: &str) {
    assert_eq!(role.to_string(), value);
    assert_eq!(
        serde_json::to_value(role).unwrap(),
        serde_json::json!(value)
    );
}

#[rstest]
#[case::proxy_admin(
    UserRole::ProxyAdmin,
    "admin over litellm proxy, has all permissions",
    "Admin (All Permissions)"
)]
#[case::proxy_admin_viewer(
    UserRole::ProxyAdminViewer,
    "view all keys, view all spend",
    "Admin (View Only)"
)]
#[case::internal_user(
    UserRole::InternalUser,
    "view/create/delete their own keys, view their own spend",
    "Internal User (Create/Delete/View)"
)]
#[case::internal_user_viewer(
    UserRole::InternalUserViewer,
    "view their own keys, view their own spend",
    "Internal User (View Only)"
)]
#[case::team(UserRole::Team, "team scope used for JWT auth", "Team")]
#[case::customer(UserRole::Customer, "customer", "Customer")]
fn each_role_description_and_ui_label(
    #[case] role: UserRole,
    #[case] description: &str,
    #[case] ui_label: &str,
) {
    assert_eq!(role.description_and_ui_label(), (description, ui_label));
}

#[test]
fn org_admin_labels_empty() {
    assert_eq!(UserRole::OrgAdmin.description_and_ui_label(), ("", ""));
}

#[rstest]
#[case::proxy_admin(UserRole::ProxyAdmin, false)]
#[case::proxy_admin_viewer(UserRole::ProxyAdminViewer, false)]
#[case::org_admin(UserRole::OrgAdmin, false)]
#[case::internal_user(UserRole::InternalUser, true)]
#[case::internal_user_viewer(UserRole::InternalUserViewer, true)]
#[case::team(UserRole::Team, false)]
#[case::customer(UserRole::Customer, false)]
fn only_internal_roles_match(#[case] role: UserRole, #[case] expected: bool) {
    assert_eq!(role.is_internal_user_role(), expected);
}
