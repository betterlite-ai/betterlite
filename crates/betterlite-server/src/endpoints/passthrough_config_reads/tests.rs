#![cfg(test)]

use betterlite_auth::auth_identity::roles::Role;
use rstest::rstest;

use crate::auth::auth_key_lookup::tests::master_caller;
use crate::endpoints::passthrough_config_reads::caller::CallerAccess;
use crate::endpoints::passthrough_config_reads::caller::access_of;
use crate::endpoints::passthrough_config_reads::support::caller;

#[rstest]
#[case::platform_admin("user", vec![Role::PlatformAdmin], CallerAccess::FullAdmin)]
#[case::admin_and_viewer("user", vec![Role::PlatformAdmin, Role::PlatformViewer], CallerAccess::FullAdmin)]
#[case::platform_viewer("user", vec![Role::PlatformViewer], CallerAccess::AdminView)]
#[case::team_admin("user", vec![Role::TeamAdmin], CallerAccess::Limited)]
#[case::team_member("user", vec![Role::TeamMember], CallerAccess::Limited)]
#[case::org_admin("user", vec![Role::OrgAdmin], CallerAccess::Limited)]
#[case::no_roles("user", Vec::new(), CallerAccess::Limited)]
fn the_principal_role_picks_the_general_settings_access(
    #[case] subject: &str,
    #[case] roles: Vec<Role>,
    #[case] access: CallerAccess,
) {
    assert_eq!(access_of(&caller(subject, roles)), access);
}

#[test]
fn the_master_key_record_has_full_admin() {
    let context = master_caller();
    assert_eq!(access_of(&context), CallerAccess::FullAdmin);
}
