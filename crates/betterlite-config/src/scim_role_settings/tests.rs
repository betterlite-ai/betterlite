#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{ScimUserRole, Settings, load_scim_role_settings};
use crate::loader::ConfigError;

fn lookup() -> BTreeMap<String, String> {
    BTreeMap::new()
}

fn load(source: &str) -> Settings {
    load_scim_role_settings(source, &lookup()).unwrap()
}

fn load_error(source: &str) -> ConfigError {
    load_scim_role_settings(source, &lookup()).unwrap_err()
}

#[test]
fn loads_scim_role_settings() {
    let defaults = load("model_list: []\n");
    assert!(defaults.scim_upsert_user);
    assert_eq!(defaults.scim_admin_group, None);
    assert_eq!(defaults.default_user_role, None);

    let disabled = load("litellm_settings:\n  scim_upsert_user: false\n");
    assert!(!disabled.scim_upsert_user);

    let string_false = load("litellm_settings:\n  scim_upsert_user: \"false\"\n");
    assert!(string_false.scim_upsert_user);

    let empty_group = load("litellm_settings:\n  scim_admin_group: \"\"\n");
    assert_eq!(empty_group.scim_admin_group, None);

    let grouped = load("litellm_settings:\n  scim_admin_group: SCIM Admins\n");
    assert_eq!(grouped.scim_admin_group.as_deref(), Some("SCIM Admins"));

    let custom_role =
        load("general_settings:\n  default_internal_user_params:\n    user_role: internal_user\n");
    assert_eq!(
        custom_role.default_user_role,
        Some(ScimUserRole::InternalUser)
    );

    let nulled = load("litellm_settings: null\ngeneral_settings: null\n");
    assert!(nulled.scim_upsert_user);
    assert_eq!(nulled.scim_admin_group, None);
    assert_eq!(nulled.default_user_role, None);
}

#[rstest]
#[case::boolean_true("true", true)]
#[case::boolean_false("false", false)]
#[case::zero("0", false)]
#[case::one("1", true)]
#[case::float_nonzero("1.5", true)]
#[case::float_zero("0.0", false)]
#[case::empty_text("\"\"", false)]
#[case::text_word("enabled", true)]
#[case::text_false("\"false\"", true)]
#[case::empty_list("[]", false)]
#[case::one_item_list("[a]", true)]
#[case::empty_mapping("{}", false)]
#[case::one_entry_mapping("{a: b}", true)]
fn loads_scim_role_settings_follows_python_truthiness(#[case] value: &str, #[case] expected: bool) {
    let source = format!("litellm_settings:\n  scim_upsert_user: {value}\n");
    assert_eq!(load(&source).scim_upsert_user, expected);
}

#[rstest]
#[case::unset("null", None)]
#[case::boolean_false("false", None)]
#[case::zero("0", None)]
#[case::empty_text("\"\"", None)]
#[case::empty_list("[]", None)]
#[case::empty_mapping("{}", None)]
#[case::nonempty_text("SCIM Admins", Some("SCIM Admins"))]
fn loads_scim_role_settings_disables_empty_admin_groups(
    #[case] value: &str,
    #[case] expected: Option<&str>,
) {
    let source = format!("litellm_settings:\n  scim_admin_group: {value}\n");
    assert_eq!(load(&source).scim_admin_group.as_deref(), expected);
}

#[test]
fn loads_scim_role_settings_rejects_a_non_string_admin_group() {
    let error = load_error("litellm_settings:\n  scim_admin_group: 12\n");
    assert!(matches!(
        error,
        ConfigError::InvalidScalar { path, .. } if path == "litellm_settings.scim_admin_group"
    ));
}

#[rstest]
#[case::proxy_admin("proxy_admin", ScimUserRole::ProxyAdmin)]
#[case::proxy_admin_viewer("proxy_admin_viewer", ScimUserRole::ProxyAdminViewer)]
#[case::internal_user("internal_user", ScimUserRole::InternalUser)]
#[case::internal_user_viewer("internal_user_viewer", ScimUserRole::InternalUserViewer)]
fn loads_scim_role_settings_accepts_every_scim_role(
    #[case] configured: &str,
    #[case] expected: ScimUserRole,
) {
    let source = format!(
        "general_settings:\n  default_internal_user_params:\n    user_role: {configured}\n"
    );
    assert_eq!(load(&source).default_user_role, Some(expected));
}

#[test]
fn loads_scim_role_settings_rejects_a_role_outside_the_scim_literal() {
    let error = load_error(
        "general_settings:\n  default_internal_user_params:\n    user_role: org_admin\n",
    );
    assert!(matches!(
        error,
        ConfigError::InvalidScalar { path, .. }
            if path == "general_settings.default_internal_user_params.user_role"
    ));
}

#[test]
fn loads_scim_role_settings_rejects_non_object_internal_user_params() {
    let error = load_error("general_settings:\n  default_internal_user_params: later\n");
    assert!(matches!(
        error,
        ConfigError::InvalidScalar { path, .. }
            if path == "general_settings.default_internal_user_params"
    ));
}
