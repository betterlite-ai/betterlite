use rstest::rstest;

use crate::sso_claims::roles::determine_role_from_groups;
use crate::sso_types::records::RoleMappings;
use crate::user_roles::UserRole;

use super::support::{mappings, strings};

fn ranked() -> RoleMappings {
    mappings(
        Some(UserRole::InternalUserViewer),
        &[
            (UserRole::ProxyAdmin, &["admins"]),
            (UserRole::ProxyAdminViewer, &["viewers"]),
            (UserRole::InternalUser, &["staff", "admins"]),
        ],
    )
}

#[rstest]
#[case::proxy_admin(&["admins"], Some(UserRole::ProxyAdmin))]
#[case::admin_viewer(&["viewers"], Some(UserRole::ProxyAdminViewer))]
#[case::internal_user(&["staff"], Some(UserRole::InternalUser))]
#[case::highest_of_two_roles(&["staff", "viewers"], Some(UserRole::ProxyAdminViewer))]
#[case::an_admin_group_listed_twice(&["admins", "admins"], Some(UserRole::ProxyAdmin))]
#[case::unknown_group_takes_the_default(&["unknown"], Some(UserRole::InternalUserViewer))]
#[case::no_groups_take_the_default(&[], Some(UserRole::InternalUserViewer))]
fn determines_the_highest_privilege_role(
    #[case] groups: &[&str],
    #[case] expected: Option<UserRole>,
) {
    assert_eq!(
        determine_role_from_groups(&strings(groups), &ranked()),
        expected
    );
}

#[test]
fn an_unranked_role_never_matches_a_group() {
    let unranked = mappings(
        Some(UserRole::Customer),
        &[(UserRole::Team, &["team-group"])],
    );
    assert_eq!(
        determine_role_from_groups(&strings(&["team-group"]), &unranked),
        Some(UserRole::Customer)
    );
}

#[test]
fn empty_roles_return_the_default_role() {
    let empty = mappings(Some(UserRole::InternalUser), &[]);
    assert_eq!(
        determine_role_from_groups(&strings(&["admins"]), &empty),
        Some(UserRole::InternalUser)
    );
}

#[test]
fn a_missing_default_role_returns_none() {
    let missing = mappings(None, &[(UserRole::ProxyAdmin, &["admins"])]);
    assert_eq!(
        determine_role_from_groups(&strings(&["unknown"]), &missing),
        None
    );
}

#[test]
fn a_missing_default_role_still_lets_a_group_match() {
    let missing = mappings(None, &[(UserRole::ProxyAdmin, &["admins"])]);
    assert_eq!(
        determine_role_from_groups(&strings(&["admins"]), &missing),
        Some(UserRole::ProxyAdmin)
    );
}
