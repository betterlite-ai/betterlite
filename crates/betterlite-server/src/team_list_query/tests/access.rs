use std::sync::Arc;

use betterlite_auth::auth_user_lookup::lookup::UserLookup;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_db::team_rows::team::LiteLLMTeamTable;

use crate::team_list_query::access::{V2Access, authorize_and_filter_teams, enforce_v2_access};
use crate::team_list_query::conditions::{TeamListScope, build_where_conditions};

use super::support::{
    Fixture, RecordingCache, auth, database, lookup, membership_insert, organization_insert,
    rejected, team_filters, team_insert, user_insert, user_store,
};

fn team_ids(teams: &[LiteLLMTeamTable]) -> Vec<String> {
    let mut ids: Vec<String> = teams
        .iter()
        .map(|team| team.fields.base.team_id.clone())
        .collect();
    ids.sort();
    ids
}

fn org_admin_seed() -> String {
    [
        organization_insert("org-1"),
        organization_insert("org-2"),
        user_insert("u1", &["t-own"]),
        membership_insert("u1", "org-1", "org_admin"),
        user_insert("u2", &["shared", "outside"]),
        team_insert("shared", "shared", Some("org-1"), &["u2"], &["u2"]),
        team_insert("other", "other", Some("org-1"), &[], &[]),
        team_insert(
            "outside",
            "outside",
            Some("org-2"),
            &["u1", "u2"],
            &["u1", "u2"],
        ),
        team_insert("t-own", "own", Some("org-2"), &["u1"], &["u1"]),
    ]
    .concat()
}

fn admin_seed() -> String {
    [
        team_insert("t1", "one", None, &["u2"], &["u2"]),
        team_insert("t2", "two", None, &[], &[]),
    ]
    .concat()
}

async fn admin_scope(users: &UserLookup, admin: &UserAPIKeyAuth) {
    let access = enforce_v2_access(users, admin, Some("u2"), Some("org-9"))
        .await
        .unwrap();
    assert_eq!(
        access,
        V2Access {
            user_id: Some("u2".to_owned()),
            org_admin_org_ids: None,
            own_team_ids: Vec::new(),
        }
    );
    let viewer = auth(r#"{"user_id":"watcher","user_role":"proxy_admin_viewer"}"#);
    let seen = enforce_v2_access(users, &viewer, None, None).await.unwrap();
    assert_eq!(
        seen,
        V2Access {
            user_id: None,
            org_admin_org_ids: None,
            own_team_ids: Vec::new(),
        }
    );
}

async fn admin_reads(users: &UserLookup, admin: &UserAPIKeyAuth, fixture: &Fixture) {
    let teams = authorize_and_filter_teams(users, admin, None, Some(fixture.client()))
        .await
        .unwrap();
    assert_eq!(team_ids(&teams), vec!["t1".to_owned(), "t2".to_owned()]);
    let filtered = authorize_and_filter_teams(users, admin, Some("u2"), Some(fixture.client()))
        .await
        .unwrap();
    assert_eq!(team_ids(&filtered), vec!["t1".to_owned()]);
}

#[tokio::test]
async fn admin_view_unrestricted() {
    let fixture = database(&admin_seed()).await;
    let cache = Arc::new(RecordingCache::default());
    // no store: an admin view must answer without a user lookup
    let users = lookup(&cache, None);
    let admin = auth(r#"{"user_id":"root","user_role":"proxy_admin"}"#);
    admin_scope(&users, &admin).await;
    admin_reads(&users, &admin, &fixture).await;
    fixture.close().await;
}

#[tokio::test]
async fn org_own_union() {
    let fixture = database(&org_admin_seed()).await;
    let cache = Arc::new(RecordingCache::default());
    let store = user_store(&fixture.store, &fixture.store);
    let users = lookup(&cache, Some(store));
    let caller = auth(r#"{"user_id":"u1","user_role":"internal_user"}"#);
    let expected = V2Access {
        user_id: None,
        org_admin_org_ids: Some(vec!["org-1".to_owned()]),
        own_team_ids: vec!["t-own".to_owned()],
    };
    let no_filter = enforce_v2_access(&users, &caller, None, None)
        .await
        .unwrap();
    assert_eq!(no_filter, expected);
    // an org admin's own query also carries the administered orgs and the caller's teams
    let own = enforce_v2_access(&users, &caller, Some("u1"), None)
        .await
        .unwrap();
    assert_eq!(own, expected);
    fixture.close().await;
}

#[tokio::test]
async fn org_other_user_intersection() {
    let fixture = database(&org_admin_seed()).await;
    let cache = Arc::new(RecordingCache::default());
    let store = user_store(&fixture.store, &fixture.store);
    let users = lookup(&cache, Some(store));
    let caller = auth(r#"{"user_id":"u1","user_role":"internal_user"}"#);
    let access = enforce_v2_access(&users, &caller, Some("u2"), None)
        .await
        .unwrap();
    assert_eq!(access.user_id, Some("u2".to_owned()));
    assert_eq!(access.org_admin_org_ids, Some(vec!["org-1".to_owned()]));
    // another user's teams never widen the caller's orgs
    assert!(access.own_team_ids.is_empty());
    let filters = team_filters(None, None, None, Some("u2"));
    let scope = TeamListScope {
        org_admin_org_ids: access.org_admin_org_ids.as_deref(),
        own_team_ids: &access.own_team_ids,
    };
    let clause = build_where_conditions(&users, &filters, scope)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        fixture.matching_team_ids(&clause, false).await,
        vec!["shared".to_owned()]
    );
    fixture.close().await;
}

#[tokio::test]
async fn foreign_org_403() {
    let fixture = database(&org_admin_seed()).await;
    let cache = Arc::new(RecordingCache::default());
    let store = user_store(&fixture.store, &fixture.store);
    let users = lookup(&cache, Some(store));
    let caller = auth(r#"{"user_id":"u1","user_role":"internal_user"}"#);
    let rejection = rejected(enforce_v2_access(&users, &caller, None, Some("org-2")).await);
    assert_eq!(rejection.status(), 403);
    assert_eq!(
        rejection.detail().error,
        "You can only view teams within your organizations."
    );
    // an empty organization_id is no filter in LiteLLM, so it never rejects
    let allowed = enforce_v2_access(&users, &caller, None, Some(""))
        .await
        .unwrap();
    assert_eq!(allowed.org_admin_org_ids, Some(vec!["org-1".to_owned()]));
    fixture.close().await;
}

#[tokio::test]
async fn regular_other_user_401() {
    let fixture = database(&user_insert("u2", &[])).await;
    let cache = Arc::new(RecordingCache::default());
    let store = user_store(&fixture.store, &fixture.store);
    let users = lookup(&cache, Some(store));
    let caller = auth(r#"{"user_id":"u2","user_role":"internal_user"}"#);
    let rejection = rejected(enforce_v2_access(&users, &caller, Some("u1"), None).await);
    assert_eq!(rejection.status(), 401);
    assert_eq!(
        rejection.detail().error,
        "Only admin users can query all teams/other teams. Your user role=internal_user"
    );
    // a caller the database does not know has no org admin role either
    let ghost = auth(r#"{"user_id":"ghost","user_role":"internal_user"}"#);
    let ghost_rejection = rejected(enforce_v2_access(&users, &ghost, Some("u1"), None).await);
    assert_eq!(ghost_rejection.status(), 401);
    let anonymous = auth(r#"{"user_role":"internal_user"}"#);
    let anonymous_rejection =
        rejected(enforce_v2_access(&users, &anonymous, Some("u1"), None).await);
    assert_eq!(anonymous_rejection.status(), 401);
    fixture.close().await;
}

#[tokio::test]
async fn regular_default_self() {
    let fixture = database(&user_insert("u2", &[])).await;
    let cache = Arc::new(RecordingCache::default());
    let store = user_store(&fixture.store, &fixture.store);
    let users = lookup(&cache, Some(store));
    let caller = auth(r#"{"user_id":"u2","user_role":"internal_user"}"#);
    let access = enforce_v2_access(&users, &caller, Some("u2"), None)
        .await
        .unwrap();
    assert_eq!(
        access,
        V2Access {
            user_id: Some("u2".to_owned()),
            org_admin_org_ids: None,
            own_team_ids: Vec::new(),
        }
    );
    // without a requested user_id the standard route check refuses the caller
    let rejection = rejected(enforce_v2_access(&users, &caller, None, None).await);
    assert_eq!(rejection.status(), 401);
    fixture.close().await;
}

#[tokio::test]
async fn legacy_explicit_self() {
    let fixture = database(&org_admin_seed()).await;
    let cache = Arc::new(RecordingCache::default());
    let store = user_store(&fixture.store, &fixture.store);
    let users = lookup(&cache, Some(store));
    let caller = auth(r#"{"user_id":"u1","user_role":"internal_user"}"#);
    let teams = authorize_and_filter_teams(&users, &caller, Some("u1"), Some(fixture.client()))
        .await
        .unwrap();
    // an own query reads every team and keeps the rosters holding the caller, not the org's teams
    assert_eq!(
        team_ids(&teams),
        vec!["outside".to_owned(), "t-own".to_owned()]
    );
    // a caller without an org admin role may still read their own teams
    let regular = auth(r#"{"user_id":"u2","user_role":"internal_user"}"#);
    let own = authorize_and_filter_teams(&users, &regular, Some("u2"), Some(fixture.client()))
        .await
        .unwrap();
    assert_eq!(
        team_ids(&own),
        vec!["outside".to_owned(), "shared".to_owned()]
    );
    fixture.close().await;
}

#[tokio::test]
async fn legacy_roster_filter() {
    let fixture = database(&org_admin_seed()).await;
    let cache = Arc::new(RecordingCache::default());
    let store = user_store(&fixture.store, &fixture.store);
    let users = lookup(&cache, Some(store));
    let caller = auth(r#"{"user_id":"u1","user_role":"internal_user"}"#);
    let scoped = authorize_and_filter_teams(&users, &caller, Some("u2"), Some(fixture.client()))
        .await
        .unwrap();
    assert_eq!(team_ids(&scoped), vec!["shared".to_owned()]);
    // an empty user_id is no filter in LiteLLM, so the caller's org teams all come back
    let all = authorize_and_filter_teams(&users, &caller, Some(""), Some(fixture.client()))
        .await
        .unwrap();
    assert_eq!(
        team_ids(&all),
        vec!["other".to_owned(), "shared".to_owned()]
    );
    // a caller without an org admin role cannot read another user's teams
    let regular = auth(r#"{"user_id":"u2","user_role":"internal_user"}"#);
    let rejection = rejected(
        authorize_and_filter_teams(&users, &regular, Some("u1"), Some(fixture.client())).await,
    );
    assert_eq!(rejection.status(), 401);
    fixture.close().await;
}
