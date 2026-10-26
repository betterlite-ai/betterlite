#![cfg(test)]

use super::{UserApiKeyAuth, user_api_key_has_admin_view};
use rstest::rstest;

#[rstest]
#[case::admin("proxy_admin")]
#[case::admin_viewer("proxy_admin_viewer")]
fn admin_view_predicate_accepts_both_admin_roles(#[case] role: &str) {
    let api_key = UserApiKeyAuth {
        user_role: Some(String::from(role)),
    };
    assert!(user_api_key_has_admin_view(&api_key));
}

#[rstest]
#[case::internal_user(Some("internal_user"))]
#[case::team_admin(Some("team_admin"))]
#[case::missing(None)]
fn admin_view_predicate_rejects_non_admin_and_missing_roles(#[case] role: Option<&str>) {
    let api_key = UserApiKeyAuth {
        user_role: role.map(String::from),
    };
    assert!(!user_api_key_has_admin_view(&api_key));
}
