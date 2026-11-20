#![cfg(test)]

use std::collections::BTreeMap;

use axum::http::StatusCode;
use betterlite_auth::user_roles::UserRole;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::org_admin_keys::Settings;
use rstest::rstest;

use super::{KEY_GENERATION_DISABLED, OrgAdminKeysError, check_org_admin_key_generation};

fn loaded(source: &str) -> Settings {
    let lookup: BTreeMap<String, String> = BTreeMap::new();
    let config = ConfigLoader { lookup }.load(source).unwrap();
    config.sections.org_admin_keys
}

#[test]
fn org_admin_passes_when_generation_is_allowed() {
    let settings = Settings {
        key_generation_disabled: false,
    };
    check_org_admin_key_generation(UserRole::OrgAdmin, &settings).unwrap();
}

#[test]
fn org_admin_is_refused_with_the_exact_detail() {
    let settings = Settings {
        key_generation_disabled: true,
    };
    let error = check_org_admin_key_generation(UserRole::OrgAdmin, &settings).unwrap_err();
    assert_eq!(error, OrgAdminKeysError::KeyGenerationDisabled);
    assert_eq!(error.status_code(), StatusCode::FORBIDDEN);
    assert_eq!(error.detail().error, KEY_GENERATION_DISABLED);
    assert_eq!(
        serde_json::to_string(&error.detail()).unwrap(),
        format!("{{\"error\":\"{KEY_GENERATION_DISABLED}\"}}")
    );
}

#[rstest]
#[case::proxy_admin(UserRole::ProxyAdmin)]
#[case::proxy_admin_viewer(UserRole::ProxyAdminViewer)]
#[case::internal_user(UserRole::InternalUser)]
#[case::internal_user_viewer(UserRole::InternalUserViewer)]
#[case::team(UserRole::Team)]
#[case::customer(UserRole::Customer)]
fn every_other_role_passes_with_generation_disabled(#[case] role: UserRole) {
    let settings = Settings {
        key_generation_disabled: true,
    };
    check_org_admin_key_generation(role, &settings).unwrap();
}

// an org-admin membership is a row this helper never receives: only the global role decides
#[test]
fn internal_user_with_org_admin_membership_passes() {
    let settings = Settings {
        key_generation_disabled: true,
    };
    check_org_admin_key_generation(UserRole::InternalUser, &settings).unwrap();
}

#[rstest]
#[case::switch_on(
    "general_settings:\n  disable_key_generate_for_org_admin: true\n",
    true
)]
#[case::switch_off(
    "general_settings:\n  disable_key_generate_for_org_admin: false\n",
    false
)]
#[case::switch_absent("general_settings: {}\n", false)]
fn the_loaded_section_reaches_the_guard(#[case] source: &str, #[case] refused: bool) {
    let outcome = check_org_admin_key_generation(UserRole::OrgAdmin, &loaded(source));
    assert_eq!(outcome.is_err(), refused);
}
