#![cfg(test)]

use super::{UserApiKeyAuth, UserRole, user_api_key_has_admin_view};
use rstest::rstest;

#[rstest]
#[case::admin(0, true)]
#[case::admin_viewer(1, true)]
#[case::internal_user(2, false)]
#[case::team(3, false)]
#[case::missing(4, false)]
fn admin_view_predicate_matches_only_admin_roles(#[case] role_number: u8, #[case] expected: bool) {
    let role = match role_number {
        0 => UserRole::ProxyAdmin,
        1 => UserRole::ProxyAdminViewer,
        2 => UserRole::InternalUser,
        3 => UserRole::Team,
        _ => UserRole::Customer,
    };
    let user_role = (role_number != 4).then_some(role);
    let api_key = UserApiKeyAuth { user_role };
    assert_eq!(user_api_key_has_admin_view(&api_key), expected);
}
