#![cfg(test)]

use super::{
    AuthenticatedPrincipal, OwnerFilter, ResourceRecord, build_list_page, build_owner_filter,
    can_access_resource, resolve_resource_owner_id,
};

fn principal(user_id: Option<&str>, team_id: Option<&str>) -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        user_id: user_id.map(str::to_owned),
        team_id: team_id.map(str::to_owned),
        token: None,
        api_key: None,
        role: "internal_user".to_owned(),
    }
}

#[test]
fn user_and_team_visibility() {
    let caller = principal(Some("u"), Some("t"));
    assert_eq!(resolve_resource_owner_id(&caller).as_deref(), Some("u"));
    assert_eq!(
        build_owner_filter(&caller),
        Some(OwnerFilter::UserOrTeam {
            user_id: "u".to_owned(),
            team_id: "t".to_owned(),
        })
    );
    assert!(can_access_resource(&caller, Some("u"), Some("x")));
    assert!(can_access_resource(&caller, Some("v"), Some("t")));
    assert!(!can_access_resource(&caller, Some("v"), Some("x")));
    assert_eq!(
        build_owner_filter(&principal(Some("u"), None)),
        Some(OwnerFilter::CreatedBy {
            created_by: "u".to_owned()
        })
    );
}

#[test]
fn service_account_scope() {
    let mut caller = principal(None, Some("t"));
    caller.token = Some(secrecy::SecretString::new(Box::<str>::from("k")));
    assert_eq!(resolve_resource_owner_id(&caller), None);
    assert_eq!(
        build_owner_filter(&caller),
        Some(OwnerFilter::TeamId {
            team_id: "t".to_owned()
        })
    );
    assert!(!can_access_resource(&caller, Some("key:k"), Some("x")));
    assert!(can_access_resource(&caller, None, Some("t")));
}

#[test]
fn empty_identity_fields() {
    let user = principal(Some(""), None);
    assert_eq!(resolve_resource_owner_id(&user).as_deref(), Some(""));
    assert_eq!(
        build_owner_filter(&user),
        Some(OwnerFilter::CreatedBy {
            created_by: String::new()
        })
    );
    assert!(can_access_resource(&user, Some(""), None));
    let team = principal(None, Some(""));
    assert_eq!(
        build_owner_filter(&team),
        Some(OwnerFilter::TeamId {
            team_id: String::new()
        })
    );
    assert!(can_access_resource(&team, None, Some("")));
}

#[test]
fn token_owner_precedence() {
    let mut caller = principal(None, None);
    caller.token = Some(secrecy::SecretString::new(Box::<str>::from("hash_a")));
    caller.api_key = Some(secrecy::SecretString::new(Box::<str>::from("hash_b")));
    assert_eq!(
        resolve_resource_owner_id(&caller).as_deref(),
        Some("key:hash_a")
    );
    assert_eq!(
        build_owner_filter(&caller),
        Some(OwnerFilter::CreatedBy {
            created_by: "key:hash_a".to_owned()
        })
    );
    assert!(can_access_resource(&caller, Some("key:hash_a"), None));
    assert!(!can_access_resource(&caller, Some("key:hash_b"), None));
    caller.token = Some(secrecy::SecretString::new(Box::<str>::from("")));
    assert_eq!(
        resolve_resource_owner_id(&caller).as_deref(),
        Some("key:hash_b")
    );
    caller.api_key = Some(secrecy::SecretString::new(Box::<str>::from("")));
    assert_eq!(resolve_resource_owner_id(&caller), None);
    assert_eq!(build_owner_filter(&caller), None);
    assert!(!can_access_resource(&caller, None, None));
}

#[test]
fn identityless_denial() {
    let caller = principal(None, None);
    assert_eq!(build_owner_filter(&caller), None);
    assert!(!can_access_resource(&caller, None, None));
}

#[rstest::rstest]
#[case::proxy_admin("proxy_admin")]
#[case::proxy_admin_viewer("proxy_admin_viewer")]
fn admin_view_permissions(#[case] role: &str) {
    let mut caller = principal(None, None);
    caller.role = role.to_owned();
    assert_eq!(build_owner_filter(&caller), Some(OwnerFilter::Admin));
    assert!(can_access_resource(&caller, None, None));
    assert!(can_access_resource(&caller, Some("foreign"), None));
    assert_eq!(resolve_resource_owner_id(&caller), None);
    assert_eq!(build_owner_filter(&principal(None, None)), None);
}

#[test]
fn list_page_boundaries() {
    let empty = build_list_page(Vec::new(), false);
    assert_eq!(empty.object, "list");
    assert!(empty.data.is_empty());
    assert_eq!(empty.first_id, None);
    assert_eq!(empty.last_id, None);
    assert!(!empty.has_more);
    let rows = vec![
        ResourceRecord {
            id: "a".to_owned(),
            created_by: None,
            team_id: None,
        },
        ResourceRecord {
            id: "b".to_owned(),
            created_by: None,
            team_id: None,
        },
    ];
    let page = build_list_page(rows.clone(), false);
    assert_eq!(page.data, rows);
    assert_eq!(page.first_id.as_deref(), Some("a"));
    assert_eq!(page.last_id.as_deref(), Some("b"));
    assert!(build_list_page(Vec::new(), true).has_more);
}
