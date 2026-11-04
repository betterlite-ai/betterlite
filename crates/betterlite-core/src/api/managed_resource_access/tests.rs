#![cfg(test)]

use super::{
    OwnerFilter, ResourceRecord, build_list_page, build_owner_filter, can_access_resource,
    resolve_resource_owner_id,
};
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey, TeamIdentity, UserIdentity,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, Role, TeamRole};

fn principal(user_id: Option<&str>, team_id: Option<&str>) -> Principal {
    Principal {
        principal_type: PrincipalType::Human,
        subject: String::new(),
        issuer: None,
        audience: Vec::new(),
        user: user_id.map(|id| UserIdentity {
            id: id.to_owned(),
            external_id: None,
            user_name: None,
            email: None,
            display_name: None,
        }),
        organization: None,
        teams: team_id
            .map(|id| {
                vec![TeamIdentity {
                    id: id.to_owned(),
                    name: None,
                    role: TeamRole::Member,
                }]
            })
            .unwrap_or_default(),
        project: None,
        end_user: None,
        roles: Vec::new(),
        scopes: Vec::new(),
        auth_method: AuthMethod::ApiKey,
        credential_ref: CredentialRef::default(),
        network: NetworkContext::default(),
        source_key: SourceKey::default(),
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
            team_id: "t".to_owned()
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
    caller.credential_ref.token_id = Some("k".to_owned());
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
    caller.credential_ref.token_id = Some("hash_a".to_owned());
    caller.credential_ref.key_id = Some("hash_b".to_owned());
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
    caller.credential_ref.token_id = Some(String::new());
    assert_eq!(
        resolve_resource_owner_id(&caller).as_deref(),
        Some("key:hash_b")
    );
    caller.credential_ref.key_id = Some(String::new());
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
#[case::proxy_admin(Role::PlatformAdmin)]
#[case::proxy_admin_viewer(Role::PlatformViewer)]
fn admin_view_permissions(#[case] role: Role) {
    let mut caller = principal(None, None);
    caller.roles.push(role);
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
