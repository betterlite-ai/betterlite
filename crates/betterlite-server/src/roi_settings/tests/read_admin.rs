#![cfg(test)]

use betterlite_auth::auth_identity::roles::Role;
use rstest::rstest;

use crate::roi_settings::auth::READ_FORBIDDEN;
use crate::roi_settings::auth::read_admin;
use crate::roi_settings::error::RoiSettingsError;
use crate::roi_settings::support::caller;

#[rstest]
#[case::master_key("master_key", Vec::new(), true)]
#[case::proxy_admin("user", vec![Role::PlatformAdmin], true)]
#[case::proxy_admin_viewer("user", vec![Role::PlatformViewer], true)]
#[case::admin_and_viewer("user", vec![Role::PlatformAdmin, Role::PlatformViewer], true)]
#[case::org_admin("user", vec![Role::OrgAdmin], false)]
#[case::org_viewer("user", vec![Role::OrgViewer], false)]
#[case::team_admin("user", vec![Role::TeamAdmin], false)]
#[case::team_member("user", vec![Role::TeamMember], false)]
#[case::no_roles("user", Vec::new(), false)]
fn read_admin_accepts_the_proxy_admin_roles(
    #[case] subject: &str,
    #[case] roles: Vec<Role>,
    #[case] allowed: bool,
) {
    let context = caller(subject, roles);
    assert_eq!(read_admin(Some(&context)).is_ok(), allowed);
}

#[test]
fn read_admin_without_a_caller_is_unauthenticated() {
    assert!(matches!(
        read_admin(None),
        Err(RoiSettingsError::Unauthenticated)
    ));
}

#[test]
fn read_admin_refuses_with_litellms_detail() {
    let context = caller("user", vec![Role::OrgAdmin]);
    assert!(matches!(
        read_admin(Some(&context)),
        Err(RoiSettingsError::Forbidden(READ_FORBIDDEN))
    ));
}
