use betterlite_paths::error::PathError;

use crate::jwt_claims::claims::ClaimError;
use crate::sso_claims::claims::{claim_groups, claim_text};
use crate::sso_claims::roles::resolve_role;
use crate::user_roles::UserRole;

use super::support::{document, strings};

#[test]
fn a_missing_path_is_absent() {
    let response = document(r#"{"email":"user@example.com"}"#);
    assert_eq!(claim_text(&response, "nickname").unwrap(), None);
    assert!(claim_groups(&response, "nickname").unwrap().is_empty());
    assert_eq!(resolve_role(&response, None, "nickname").unwrap(), None);
}

#[test]
fn a_null_value_is_absent() {
    let response = document(r#"{"email":null,"groups":null,"role":null}"#);
    assert_eq!(claim_text(&response, "email").unwrap(), None);
    assert!(claim_groups(&response, "groups").unwrap().is_empty());
    assert_eq!(resolve_role(&response, None, "role").unwrap(), None);
}

#[test]
fn a_non_text_value_names_the_path() {
    let response = document(r#"{"email":42}"#);
    let error = claim_text(&response, "email").unwrap_err();
    assert!(
        matches!(&error, ClaimError::Type { claim, .. } if claim == "email"),
        "{error:?}"
    );
}

#[test]
fn the_metadata_prefix_is_stripped() {
    let response = document(r#"{"tier":"gold","metadata":{"tier":"silver"}}"#);
    assert_eq!(
        claim_text(&response, "metadata.tier").unwrap(),
        Some("gold".to_owned())
    );
}

#[test]
fn a_group_list_keeps_its_string_members() {
    let response = document(r#"{"groups":["admins","staff",7,null,["nested"]]}"#);
    assert_eq!(
        claim_groups(&response, "groups").unwrap(),
        strings(&["admins", "staff"])
    );
}

#[test]
fn a_comma_separated_group_string_is_split_and_trimmed() {
    let response = document(r#"{"groups":" admins , staff ,,"}"#);
    assert_eq!(
        claim_groups(&response, "groups").unwrap(),
        strings(&["admins", "staff"])
    );
}

#[test]
fn a_scalar_group_string_names_one_group() {
    let response = document(r#"{"groups":"admins"}"#);
    assert_eq!(
        claim_groups(&response, "groups").unwrap(),
        strings(&["admins"])
    );
}

#[test]
fn a_non_text_group_scalar_has_no_groups() {
    let response = document(r#"{"groups":7}"#);
    assert!(claim_groups(&response, "groups").unwrap().is_empty());
}

#[test]
fn a_role_list_resolves_to_its_highest_privilege_member() {
    let response = document(r#"{"role":["internal_user","proxy_admin","nonsense"]}"#);
    assert_eq!(
        resolve_role(&response, None, "role").unwrap(),
        Some(UserRole::ProxyAdmin)
    );
}

#[test]
fn a_role_string_ignores_case() {
    let response = document(r#"{"role":"Proxy_Admin_Viewer"}"#);
    assert_eq!(
        resolve_role(&response, None, "role").unwrap(),
        Some(UserRole::ProxyAdminViewer)
    );
}

#[test]
fn an_unknown_role_name_is_absent() {
    let response = document(r#"{"role":"superuser"}"#);
    assert_eq!(resolve_role(&response, None, "role").unwrap(), None);
}

#[test]
fn a_path_over_the_length_limit_is_a_path_error() {
    let response = document(r#"{"email":"user@example.com"}"#);
    let path = "a".repeat(8193);
    let error = claim_text(&response, &path).unwrap_err();
    assert!(
        matches!(&error, ClaimError::Path { claim, source } if claim == &path && matches!(source, PathError::TooLong { .. })),
        "{error:?}"
    );
}

#[test]
fn a_path_over_the_depth_limit_is_a_path_error() {
    let response = document(r#"{"email":"user@example.com"}"#);
    let path = vec!["a"; 257].join(".");
    let error = resolve_role(&response, None, &path).unwrap_err();
    assert!(
        matches!(&error, ClaimError::Path { claim, source } if claim == &path && matches!(source, PathError::TooDeep { .. })),
        "{error:?}"
    );
}
