use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use betterlite_auth::auth_user_lookup::error::UserLookupError;
use betterlite_auth::auth_user_lookup::store::UserStore;
use betterlite_auth::management_routes::key_routes::KeyManagementRoute;
use betterlite_auth::user_roles::UserRole;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use rstest::rstest;

use super::super::access::{TEAM_ADMIN_ONLY, TEAM_OR_ORG_ADMIN, TeamAccess, TeamRole};
use super::super::error::TeamAccessError;
use super::super::key_team::{can_execute_key_route, user_belongs_to_keys_team};
use super::support::{World, caller, team, teams, users};

const ROW_WITHOUT_CALLER: &str = r#"{"team_id":"team-1","members_with_roles":[{"user_id":"member-1","role":"user"}],"team_member_permissions":["/key/update"]}"#;
const STALE_ADMIN: &str =
    r#"{"team_id":"team-1","members_with_roles":[{"user_id":"stale-1","role":"admin"}]}"#;

fn world(cached_team: &str, team_row: &str) -> Arc<World> {
    let cached = serde_json::from_str::<JsonData>(cached_team).unwrap();
    Arc::new(World {
        cached_user: None,
        cached_team: Some(LiteLLMTeamTableCachedObj::try_from(cached).unwrap()),
        team_row: Some(serde_json::from_str(team_row).unwrap()),
        user_reads: AtomicUsize::new(0),
        user_creates: AtomicUsize::new(0),
        team_cache_reads: AtomicUsize::new(0),
        team_reads: Mutex::new(Vec::new()),
    })
}

fn org_access(world: &Arc<World>, with_store: bool) -> TeamAccess {
    let store: Option<Arc<dyn UserStore>> =
        with_store.then(|| -> Arc<dyn UserStore> { Arc::<World>::clone(world) });
    TeamAccess::from(Arc::new(users(world, store)))
}

#[rstest]
#[case::org_admin("org-1", "org_admin", Some(TeamRole::OrgAdmin))]
#[case::other_organization("org-2", "org_admin", None)]
#[case::other_role("org-1", "internal_user", None)]
#[tokio::test]
async fn organization_admin(
    #[case] organization: &str,
    #[case] role: &str,
    #[case] strongest: Option<TeamRole>,
) {
    let user = format!(
        r#"{{"user_id":"org-user","organization_memberships":[{{"user_id":"org-user","organization_id":"{organization}","user_role":"{role}","created_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z"}}]}}"#
    );
    let shared = Arc::new(World {
        cached_user: Some(serde_json::from_str(&user).unwrap()),
        cached_team: None,
        team_row: None,
        user_reads: AtomicUsize::new(0),
        user_creates: AtomicUsize::new(0),
        team_cache_reads: AtomicUsize::new(0),
        team_reads: Mutex::new(Vec::new()),
    });
    let access = org_access(&shared, false);
    let base = team(r#"{"team_id":"team-1","organization_id":"org-1"}"#);
    let outsider = caller(Some("org-user"), None);
    assert_eq!(
        access.strongest_role(&outsider, &base).await.unwrap(),
        strongest
    );
    assert_eq!(
        access
            .allows(&outsider, &base, &TEAM_OR_ORG_ADMIN)
            .await
            .unwrap(),
        strongest.is_some()
    );
    assert!(
        !access
            .allows(&outsider, &base, &TEAM_ADMIN_ONLY)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn org_read_no_upsert() {
    let shared = Arc::new(World::default());
    let access = org_access(&shared, true);
    let base = team(r#"{"team_id":"team-1","organization_id":"org-1"}"#);
    let error = access
        .allows(&caller(Some("new-user"), None), &base, &TEAM_OR_ORG_ADMIN)
        .await
        .unwrap_err();
    assert!(
        matches!(error, TeamAccessError::OrgRoles(cause) if matches!(*cause, UserLookupError::NotFound(_)))
    );
    assert_eq!(shared.user_reads.load(Ordering::SeqCst), 1);
    assert_eq!(shared.user_creates.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn org_read_error() {
    let shared = Arc::new(World::default());
    let access = org_access(&shared, false);
    let base = team(
        r#"{"team_id":"team-1","organization_id":"org-1","members_with_roles":[{"user_id":"admin-1","role":"admin"}]}"#,
    );
    let admin = caller(Some("admin-1"), None);
    assert!(
        access
            .allows(&admin, &base, &TEAM_OR_ORG_ADMIN)
            .await
            .unwrap()
    );
    let error = access.strongest_role(&admin, &base).await.unwrap_err();
    assert!(
        matches!(error, TeamAccessError::OrgRoles(cause) if matches!(*cause, UserLookupError::NoDatabase))
    );
}

#[tokio::test]
async fn nonmember_route_401() {
    let shared = world(STALE_ADMIN, ROW_WITHOUT_CALLER);
    let lookup = teams(&shared);
    let error = can_execute_key_route(
        &caller(Some("outsider"), None),
        KeyManagementRoute::KeyUpdate,
        Some("team-1"),
        &lookup,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(&error, TeamAccessError::NotTeamMember { team_id, .. } if team_id == "team-1")
    );
    assert_eq!(error.status(), Some(401));
    let exception = error.proxy_exception().unwrap();
    assert_eq!(exception.error_type, "team_member_permission_error");
    assert_eq!(exception.param.as_deref(), Some("/key/update"));
    assert_eq!(exception.code, "401");
    let member = caller(Some("member-1"), None);
    can_execute_key_route(
        &member,
        KeyManagementRoute::KeyUpdate,
        Some("team-1"),
        &lookup,
    )
    .await
    .unwrap();
    let denied = can_execute_key_route(
        &member,
        KeyManagementRoute::KeyDelete,
        Some("team-1"),
        &lookup,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(&denied, TeamAccessError::MemberRoute { route, .. } if route == "/key/delete")
    );
}

#[tokio::test]
async fn personal_key_bypass() {
    let shared = world(STALE_ADMIN, ROW_WITHOUT_CALLER);
    let lookup = teams(&shared);
    let outsider = caller(Some("outsider"), None);
    can_execute_key_route(&outsider, KeyManagementRoute::KeyDelete, None, &lookup)
        .await
        .unwrap();
    let proxy = caller(Some("outsider"), Some(UserRole::ProxyAdmin));
    can_execute_key_route(
        &proxy,
        KeyManagementRoute::KeyDelete,
        Some("team-1"),
        &lookup,
    )
    .await
    .unwrap();
    assert!(shared.team_reads.lock().unwrap().is_empty());
}

#[tokio::test]
async fn fresh_team_read() {
    let shared = world(STALE_ADMIN, ROW_WITHOUT_CALLER);
    let lookup = teams(&shared);
    let stale = caller(Some("stale-1"), None);
    let error = can_execute_key_route(
        &stale,
        KeyManagementRoute::KeyUpdate,
        Some("team-1"),
        &lookup,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, TeamAccessError::NotTeamMember { .. }));
    assert!(
        !user_belongs_to_keys_team(&stale, Some("team-1"), &lookup)
            .await
            .unwrap()
    );
    assert!(
        user_belongs_to_keys_team(&caller(Some("member-1"), None), Some("team-1"), &lookup)
            .await
            .unwrap()
    );
    assert_eq!(shared.team_cache_reads.load(Ordering::SeqCst), 0);
    assert_eq!(*shared.team_reads.lock().unwrap(), [true, true, true]);
}

#[tokio::test]
async fn personal_membership_false() {
    let shared = world(STALE_ADMIN, ROW_WITHOUT_CALLER);
    let lookup = teams(&shared);
    assert!(
        !user_belongs_to_keys_team(&caller(Some("member-1"), None), None, &lookup)
            .await
            .unwrap()
    );
    assert!(shared.team_reads.lock().unwrap().is_empty());
}
