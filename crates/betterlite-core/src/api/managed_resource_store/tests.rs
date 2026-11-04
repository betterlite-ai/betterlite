#![cfg(test)]

use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::Role;
use rstest::rstest;

use super::{
    get_primary_owner_scope, get_resource_owner_scopes, is_proxy_admin,
    user_can_access_resource_owner,
};

fn principal() -> Principal {
    serde_json::from_str(r#"{"principal_type":"human","subject":"u","auth_method":"api_key","user":{"id":"u"},"teams":[{"id":"t"}],"organization":{"id":"o"},"credential_ref":{"key_id":"a","token_id":"b"}}"#).unwrap()
}

#[rstest]
#[case::raw("u", true)]
#[case::user("user:u", true)]
#[case::team("team:t", true)]
#[case::org("org:o", true)]
#[case::api_key("key:a", true)]
#[case::token("key:b", true)]
#[case::foreign("user:v", false)]
fn ordered_owner_scopes(#[case] owner: &str, #[case] access: bool) {
    let identity = principal();
    let scopes = get_resource_owner_scopes(Some(&identity));
    assert_eq!(
        scopes
            .iter()
            .map(|scope| scope.0.as_str())
            .collect::<Vec<_>>(),
        ["u", "user:u", "team:t", "org:o", "key:a", "key:b"]
    );
    assert_eq!(get_primary_owner_scope(Some(&identity)).unwrap().0, "u");
    assert_eq!(
        user_can_access_resource_owner(Some(owner), Some(&identity)),
        access
    );
}

#[test]
fn primary_scope_fallback() {
    let mut identity = principal();
    identity.user = None;
    assert_eq!(
        get_primary_owner_scope(Some(&identity)).unwrap().0,
        "team:t"
    );
    identity.teams.clear();
    assert_eq!(get_primary_owner_scope(Some(&identity)).unwrap().0, "org:o");
    identity.organization = None;
    assert_eq!(get_primary_owner_scope(Some(&identity)).unwrap().0, "key:a");
    identity.credential_ref.key_id = None;
    assert_eq!(get_primary_owner_scope(Some(&identity)).unwrap().0, "key:b");
    identity.credential_ref.token_id = None;
    assert_eq!(get_primary_owner_scope(Some(&identity)), None);
    assert!(get_resource_owner_scopes(Some(&identity)).is_empty());
}

#[rstest]
#[case::duplicate_keys("user:u", "a", vec!["user:u", "user:user:u", "team:t", "org:o", "key:a"])]
#[case::raw_team_collision("team:t", "b", vec!["team:t", "user:team:t", "org:o", "key:a", "key:b"])]
#[case::raw_key_collision("key:b", "b", vec!["key:b", "user:key:b", "team:t", "org:o", "key:a"])]
fn duplicate_key_scopes(#[case] user: &str, #[case] token: &str, #[case] expected: Vec<&str>) {
    let mut identity = principal();
    identity.user.as_mut().unwrap().id = user.to_owned();
    identity.credential_ref.token_id = Some(token.to_owned());
    assert_eq!(
        get_resource_owner_scopes(Some(&identity))
            .iter()
            .map(|scope| scope.0.as_str())
            .collect::<Vec<_>>(),
        expected
    );
}

#[test]
fn missing_owner_context() {
    assert!(!is_proxy_admin(None));
    assert!(get_resource_owner_scopes(None).is_empty());
    assert_eq!(get_primary_owner_scope(None), None);
    assert!(user_can_access_resource_owner(None, None));
    assert!(user_can_access_resource_owner(Some("foreign"), None));
    let identity: Principal = serde_json::from_str(
        r#"{"principal_type":"human","subject":"not-an-owner","auth_method":"api_key"}"#,
    )
    .unwrap();
    assert!(!is_proxy_admin(Some(&identity)));
    assert!(get_resource_owner_scopes(Some(&identity)).is_empty());
    assert_eq!(get_primary_owner_scope(Some(&identity)), None);
    assert!(!user_can_access_resource_owner(None, Some(&identity)));
    assert!(!user_can_access_resource_owner(Some(""), Some(&identity)));
    assert!(!user_can_access_resource_owner(
        Some("foreign"),
        Some(&identity)
    ));
}

#[rstest]
#[case::proxy_admin(Role::PlatformAdmin, true)]
#[case::proxy_admin_viewer(Role::PlatformViewer, false)]
#[case::internal_user(Role::TeamMember, false)]
fn admin_role_boundary(#[case] role: Role, #[case] admin: bool) {
    let mut identity = principal();
    identity.roles = vec![role];
    assert_eq!(is_proxy_admin(Some(&identity)), admin);
    assert_eq!(user_can_access_resource_owner(None, Some(&identity)), admin);
    assert_eq!(
        user_can_access_resource_owner(Some("other"), Some(&identity)),
        admin
    );
    assert!(user_can_access_resource_owner(Some("u"), Some(&identity)));
    assert!(user_can_access_resource_owner(
        Some("user:u"),
        Some(&identity)
    ));
}

#[test]
fn empty_identities_have_no_shared_owner() {
    let identity: Principal = serde_json::from_str(r#"{"principal_type":"human","subject":"not-an-owner","auth_method":"api_key","user":{"id":""},"teams":[{"id":""}],"organization":{"id":""},"credential_ref":{"key_id":"","token_id":""}}"#).unwrap();
    assert!(get_resource_owner_scopes(Some(&identity)).is_empty());
    assert_eq!(get_primary_owner_scope(Some(&identity)), None);
    assert!(!user_can_access_resource_owner(Some(""), Some(&identity)));
    assert!(!user_can_access_resource_owner(
        Some("key:"),
        Some(&identity)
    ));
}
