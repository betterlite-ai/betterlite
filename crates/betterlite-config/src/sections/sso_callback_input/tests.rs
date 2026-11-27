#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::Settings;
use crate::loader::{ConfigError, ConfigLoader};

fn settings(source: &str, variables: &[(&str, &str)]) -> Settings {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .expect("the config loads")
        .sections
        .sso_callback_input
}

fn load_error(source: &str, variables: &[(&str, &str)]) -> ConfigError {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .expect_err("the config must not load")
}

#[rstest]
#[case::absent("general_settings: {}\n", false, None)]
#[case::null_mode("general_settings:\n  ui_access_mode: null\n", false, None)]
#[case::admin_only_text("general_settings:\n  ui_access_mode: admin_only\n", false, None)]
#[case::all_text("general_settings:\n  ui_access_mode: all\n", false, None)]
#[case::other_text(
    "general_settings:\n  ui_access_mode: restricted_sso_group\n",
    false,
    None
)]
#[case::mapping_with_the_field(
    "general_settings:\n  ui_access_mode:\n    sso_group_jwt_field: groups\n",
    true,
    Some("groups")
)]
#[case::mapping_without_the_field(
    "general_settings:\n  ui_access_mode:\n    type: restricted_sso_group\n    restricted_sso_group: admins\n",
    true,
    None
)]
#[case::mapping_with_a_null_field(
    "general_settings:\n  ui_access_mode:\n    type: restricted_sso_group\n    restricted_sso_group: admins\n    sso_group_jwt_field: null\n",
    true,
    None
)]
fn loads_callback_group_claim(
    #[case] source: &str,
    #[case] has_projection: bool,
    #[case] claim_field: Option<&str>,
) {
    let projection = settings(source, &[]).group_projection;
    if !has_projection {
        assert_eq!(projection, None);
        return;
    }
    let projection = projection.expect("a mapping yields a group projection");
    assert_eq!(projection.sso_group_jwt_field.as_deref(), claim_field);
}

#[rstest]
#[case::list_mode(
    "general_settings:\n  ui_access_mode:\n    - admin_only\n",
    "general_settings.ui_access_mode"
)]
#[case::number_mode(
    "general_settings:\n  ui_access_mode: 3\n",
    "general_settings.ui_access_mode"
)]
#[case::boolean_mode(
    "general_settings:\n  ui_access_mode: true\n",
    "general_settings.ui_access_mode"
)]
#[case::number_field(
    "general_settings:\n  ui_access_mode:\n    sso_group_jwt_field: 3\n",
    "general_settings.ui_access_mode.sso_group_jwt_field"
)]
#[case::list_field(
    "general_settings:\n  ui_access_mode:\n    sso_group_jwt_field: [groups]\n",
    "general_settings.ui_access_mode.sso_group_jwt_field"
)]
fn rejects_a_mode_of_another_type(#[case] source: &str, #[case] path: &str) {
    let error = load_error(source, &[]);
    let ConfigError::InvalidScalar {
        path: found, value, ..
    } = &error
    else {
        panic!("expected an invalid scalar for {source}, got {error:?}");
    };
    assert_eq!(found.as_str(), path);
    assert!(!value);
}

#[test]
fn resolves_an_environment_reference_in_the_claim_field() {
    let projection = settings(
        "general_settings:\n  ui_access_mode:\n    sso_group_jwt_field: os.environ/GROUP_CLAIM\n",
        &[("GROUP_CLAIM", "groups")],
    )
    .group_projection
    .expect("a mapping yields a group projection");
    assert_eq!(projection.sso_group_jwt_field.as_deref(), Some("groups"));
}

#[test]
fn an_unset_environment_reference_fails_the_load() {
    let error = load_error(
        "general_settings:\n  ui_access_mode:\n    sso_group_jwt_field: os.environ/GROUP_CLAIM\n",
        &[],
    );
    let ConfigError::MissingEnvironment { variable, path } = &error else {
        panic!("expected a missing environment variable, got {error:?}");
    };
    assert_eq!(variable, "GROUP_CLAIM");
    assert_eq!(path, "general_settings.ui_access_mode.sso_group_jwt_field");
}
