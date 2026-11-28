#![cfg(test)]

use betterlite_auth::auth_identity::roles::Role;
use rstest::rstest;

use crate::auth::auth_key_lookup::tests::{caller_through_key_stage, master_caller};
use crate::roi_settings::auth::WRITE_FORBIDDEN;
use crate::roi_settings::auth::WRITE_ROLES;
use crate::roi_settings::auth::require_admin;
use crate::roi_settings::error::RoiSettingsError;
use crate::roi_settings::support::caller;

#[rstest]
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
    assert_eq!(
        require_admin(Some(&context), WRITE_ROLES, WRITE_FORBIDDEN).is_ok(),
        allowed
    );
}

#[test]
fn write_admin_admits_the_master_key_record() {
    let context = master_caller();
    require_admin(Some(&context), WRITE_ROLES, WRITE_FORBIDDEN).unwrap();
}

#[test]
fn write_admin_refuses_a_key_whose_user_is_master_key() {
    let context = caller_through_key_stage(r#"{"user_id":"master_key"}"#);
    assert!(matches!(
        require_admin(Some(&context), WRITE_ROLES, WRITE_FORBIDDEN),
        Err(RoiSettingsError::Forbidden(WRITE_FORBIDDEN))
    ));
}

#[test]
fn write_admin_without_a_caller_is_unauthenticated() {
    assert!(matches!(
        require_admin(None, WRITE_ROLES, WRITE_FORBIDDEN),
        Err(RoiSettingsError::Unauthenticated)
    ));
}

#[test]
fn write_admin_refuses_with_litellms_detail() {
    let context = caller("user", vec![Role::PlatformViewer]);
    assert!(matches!(
        require_admin(Some(&context), WRITE_ROLES, WRITE_FORBIDDEN),
        Err(RoiSettingsError::Forbidden(WRITE_FORBIDDEN))
    ));
}
