#![cfg(test)]

use rstest::rstest;

use super::*;

#[test]
fn ordered_owner_scopes() {
    let principal = OwnerPrincipal {
        user_id: Some("u".to_owned()),
        team_id: Some("t".to_owned()),
        org_id: Some("o".to_owned()),
        hashed_api_key: Some("a".to_owned()),
        token: Some("b".to_owned()),
        user_role: None,
    };
    assert_eq!(
        get_resource_owner_scopes(Some(&principal)),
        ["u", "user:u", "team:t", "org:o", "key:a", "key:b"]
    );
    assert_eq!(
        get_primary_owner_scope(Some(&principal)).as_deref(),
        Some("u")
    );
    assert!(
        get_resource_owner_scopes(Some(&principal))
            .iter()
            .all(|scope| user_can_access_resource_owner(Some(scope), Some(&principal)))
    );
    assert!(!user_can_access_resource_owner(
        Some("user:v"),
        Some(&principal)
    ));
}

#[test]
fn primary_scope_fallback() {
    let mut principal = OwnerPrincipal {
        user_id: Some("u".to_owned()),
        team_id: Some("t".to_owned()),
        org_id: Some("o".to_owned()),
        hashed_api_key: Some("a".to_owned()),
        token: Some("b".to_owned()),
        user_role: None,
    };
    principal.user_id = None;
    assert_eq!(
        get_primary_owner_scope(Some(&principal)).as_deref(),
        Some("team:t")
    );
    principal.team_id = None;
    assert_eq!(
        get_primary_owner_scope(Some(&principal)).as_deref(),
        Some("org:o")
    );
    principal.org_id = None;
    assert_eq!(
        get_primary_owner_scope(Some(&principal)).as_deref(),
        Some("key:a")
    );
    principal.hashed_api_key = None;
    assert_eq!(
        get_primary_owner_scope(Some(&principal)).as_deref(),
        Some("key:b")
    );
    principal.token = None;
    assert_eq!(get_primary_owner_scope(Some(&principal)), None);
    assert!(get_resource_owner_scopes(Some(&principal)).is_empty());
}

#[test]
fn duplicate_key_scopes() {
    let key_and_token_principal = OwnerPrincipal {
        user_id: None,
        team_id: None,
        org_id: None,
        hashed_api_key: Some("a".to_owned()),
        token: Some("a".to_owned()),
        user_role: None,
    };
    assert_eq!(
        get_resource_owner_scopes(Some(&key_and_token_principal)),
        ["key:a"]
    );
    let prefixed_user_principal = OwnerPrincipal {
        user_id: Some("user:u".to_owned()),
        team_id: None,
        org_id: None,
        hashed_api_key: None,
        token: None,
        user_role: None,
    };
    assert_eq!(
        get_resource_owner_scopes(Some(&prefixed_user_principal)),
        ["user:u", "user:user:u"]
    );
}

#[test]
fn missing_owner_context() {
    assert!(!is_proxy_admin(None));
    assert!(get_resource_owner_scopes(None).is_empty());
    assert_eq!(get_primary_owner_scope(None), None);
    assert!(user_can_access_resource_owner(None, None));
    let principal = OwnerPrincipal {
        user_id: None,
        team_id: None,
        org_id: None,
        hashed_api_key: None,
        token: None,
        user_role: Some("internal_user".to_owned()),
    };
    assert!(!user_can_access_resource_owner(None, Some(&principal)));
    assert!(!user_can_access_resource_owner(Some(""), Some(&principal)));
    assert!(!user_can_access_resource_owner(
        Some("foreign"),
        Some(&principal)
    ));
}

#[rstest]
#[case::proxy_admin("proxy_admin", true)]
#[case::proxy_admin_viewer("proxy_admin_viewer", false)]
#[case::internal_user("internal_user", false)]
fn admin_role_boundary(#[case] role: &str, #[case] is_admin: bool) {
    let principal = OwnerPrincipal {
        user_id: Some("u".to_owned()),
        team_id: None,
        org_id: None,
        hashed_api_key: None,
        token: None,
        user_role: Some(role.to_owned()),
    };
    assert_eq!(is_proxy_admin(Some(&principal)), is_admin);
    assert_eq!(
        user_can_access_resource_owner(None, Some(&principal)),
        is_admin
    );
    assert_eq!(
        user_can_access_resource_owner(Some("other"), Some(&principal)),
        is_admin
    );
    assert!(user_can_access_resource_owner(Some("u"), Some(&principal)));
    assert!(user_can_access_resource_owner(
        Some("user:u"),
        Some(&principal)
    ));
}
