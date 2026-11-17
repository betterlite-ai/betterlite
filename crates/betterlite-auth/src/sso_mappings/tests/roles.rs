use rstest::rstest;

use crate::sso_mappings::literal::LiteralError;
use crate::sso_mappings::roles::{RoleMappingError, decode_role_mapping_literal};
use crate::user_roles::UserRole;

use super::support::role_mapping;

#[test]
fn decodes_a_role_dictionary() {
    let decoded = decode_role_mapping_literal(
        b"{'proxy_admin': ['admins', 'ops'], \"internal_user_viewer\": [\"users\"]}",
        Some("groups"),
        Some("internal_user"),
    );
    assert_eq!(
        decoded,
        Ok(Some(role_mapping(
            "groups",
            Some(UserRole::InternalUser),
            vec![
                (UserRole::ProxyAdmin, vec!["admins", "ops"]),
                (UserRole::InternalUserViewer, vec!["users"]),
            ],
        )))
    );
}

#[test]
fn an_empty_dictionary_maps_no_roles() {
    let decoded = decode_role_mapping_literal(b"{}", Some("groups"), None);
    assert_eq!(decoded, Ok(Some(role_mapping("groups", None, Vec::new()))));
}

#[test]
fn a_repeated_role_keeps_the_last_groups() {
    let decoded = decode_role_mapping_literal(
        b"{'proxy_admin': ['first'], 'internal_user': ['users'], 'proxy_admin': ['last']}",
        Some("groups"),
        None,
    );
    assert_eq!(
        decoded,
        Ok(Some(role_mapping(
            "groups",
            None,
            vec![
                (UserRole::ProxyAdmin, vec!["last"]),
                (UserRole::InternalUser, vec!["users"]),
            ],
        )))
    );
}

#[rstest]
#[case::list(b"['proxy_admin']".as_slice())]
#[case::text(b"'proxy_admin'".as_slice())]
#[case::number(b"42".as_slice())]
#[case::none(b"None".as_slice())]
#[case::set(b"{'proxy_admin'}".as_slice())]
fn a_non_dictionary_literal_does_not_override(#[case] literal: &[u8]) {
    let decoded = decode_role_mapping_literal(literal, Some("groups"), Some("wizard"));
    assert_eq!(decoded, Ok(None));
}

#[rstest]
#[case::list_key(b"{[1]: ['admins']}".as_slice())]
#[case::set_value(b"{'proxy_admin': {[1]}}".as_slice())]
#[case::nested_list_key(b"{(1, [2]): ['admins']}".as_slice())]
fn an_unhashable_member_does_not_override(#[case] literal: &[u8]) {
    let decoded = decode_role_mapping_literal(literal, Some("groups"), None);
    assert_eq!(decoded, Ok(None));
}

#[test]
fn a_dictionary_without_the_group_claim_is_an_error() {
    let decoded = decode_role_mapping_literal(b"{'proxy_admin': ['admins']}", None, None);
    assert_eq!(decoded, Err(RoleMappingError::MissingGroupClaim));
}

#[test]
fn an_unknown_role_key_is_an_error() {
    let decoded = decode_role_mapping_literal(b"{'wizard': ['admins']}", Some("groups"), None);
    assert_eq!(
        decoded,
        Err(RoleMappingError::UnknownRole {
            role: "wizard".to_owned(),
        })
    );
}

#[test]
fn a_non_text_role_key_is_an_error() {
    let decoded = decode_role_mapping_literal(b"{1: ['admins']}", Some("groups"), None);
    assert_eq!(decoded, Err(RoleMappingError::KeyNotRole));
}

#[rstest]
#[case::text_value(b"{'proxy_admin': 'admins'}".as_slice())]
#[case::text_inside_the_list(b"{'proxy_admin': ['admins', 1]}".as_slice())]
#[case::nested_list(b"{'proxy_admin': [['admins']]}".as_slice())]
fn groups_that_are_not_a_list_of_names_are_an_error(#[case] literal: &[u8]) {
    let decoded = decode_role_mapping_literal(literal, Some("groups"), None);
    assert_eq!(
        decoded,
        Err(RoleMappingError::GroupsNotText {
            role: UserRole::ProxyAdmin,
        })
    );
}

#[rstest]
#[case::tuple(b"{'proxy_admin': ('admins',)}".as_slice())]
#[case::set(b"{'proxy_admin': {'admins'}}".as_slice())]
#[case::bytes(b"{'proxy_admin': [b'admins']}".as_slice())]
fn groups_pydantic_would_coerce_are_refused(#[case] literal: &[u8]) {
    let decoded = decode_role_mapping_literal(literal, Some("groups"), None);
    assert_eq!(
        decoded,
        Err(RoleMappingError::GroupsNotText {
            role: UserRole::ProxyAdmin,
        })
    );
}

#[test]
fn an_unknown_default_role_is_an_error() {
    let decoded = decode_role_mapping_literal(
        b"{'proxy_admin': ['admins']}",
        Some("groups"),
        Some("wizard"),
    );
    assert_eq!(
        decoded,
        Err(RoleMappingError::UnknownDefaultRole {
            role: "wizard".to_owned(),
        })
    );
}

#[test]
fn a_syntax_error_is_not_swallowed() {
    let decoded = decode_role_mapping_literal(b"{'proxy_admin': ['admins']", Some("groups"), None);
    assert_eq!(
        decoded,
        Err(RoleMappingError::Literal {
            source: LiteralError::Syntax,
        })
    );
}

#[test]
fn a_literal_that_is_not_utf8_is_an_error() {
    let decoded = decode_role_mapping_literal(&[0xff, 0x7b], Some("groups"), None);
    assert_eq!(
        decoded,
        Err(RoleMappingError::Literal {
            source: LiteralError::NotUtf8,
        })
    );
}
