#![cfg(test)]

use betterlite_auth::auth_identity::roles::Role;
use rstest::rstest;

use crate::roi_settings::auth::WRITE_FORBIDDEN;
use crate::roi_settings::auth::write_admin;
use crate::roi_settings::error::RoiSettingsError;
use crate::roi_settings::support::caller;

#[rstest]
#[case::master_key("master_key", Vec::new(), true)]
#[case::proxy_admin("user", vec![Role::PlatformAdmin], true)]
#[case::admin_and_viewer("user", vec![Role::PlatformAdmin, Role::PlatformViewer], true)]
#[case::proxy_admin_viewer("user", vec![Role::PlatformViewer], false)]
#[case::org_admin("user", vec![Role::OrgAdmin], false)]
#[case::team_member("user", vec![Role::TeamMember], false)]
#[case::no_roles("user", Vec::new(), false)]
fn write_admin_accepts_only_the_proxy_admin(
    #[case] subject: &str,
    #[case] roles: Vec<Role>,
    #[case] allowed: bool,
) {
    let context = caller(subject, roles);
    assert_eq!(write_admin(Some(&context)).is_ok(), allowed);
}

#[test]
fn write_admin_without_a_caller_is_unauthenticated() {
    assert!(matches!(
        write_admin(None),
        Err(RoiSettingsError::Unauthenticated)
    ));
}

#[test]
fn write_admin_refuses_with_litellms_detail() {
    let context = caller("user", vec![Role::PlatformViewer]);
    assert!(matches!(
        write_admin(Some(&context)),
        Err(RoiSettingsError::Forbidden(WRITE_FORBIDDEN))
    ));
}
