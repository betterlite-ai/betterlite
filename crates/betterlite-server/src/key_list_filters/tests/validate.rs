use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_auth::user_roles::UserRole;
use betterlite_db::identity_rows::user::LiteLLMUserTable;

use super::support::{FakeStore, caller, key_row, strings, team_lookup, user_row};
use crate::key_list_filters::error::KeyListFiltersError;
use crate::key_list_filters::selectors::KeyListSelectors;
use crate::key_list_filters::validate::validate_key_list_check;

// u1 belongs to teams t1 and t2 and to organization o1
const USER_U1: &str = r#"{"user_id":"u1","teams":["t1","t2"],"organization_memberships":[{"user_id":"u1","organization_id":"o1","user_role":"member","created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"}]}"#;

fn selectors<'query>(
    user_id: Option<&'query str>,
    team_id: Option<&'query str>,
    organization_id: Option<&'query str>,
    key_alias: Option<&'query str>,
    key_hash: Option<&'query str>,
) -> KeyListSelectors<'query> {
    KeyListSelectors {
        user_id,
        team_id,
        organization_id,
        key_alias,
        key_hash,
    }
}

fn none() -> KeyListSelectors<'static> {
    selectors(None, None, None, None, None)
}

async fn check(
    auth: &UserAPIKeyAuth,
    wanted: &KeyListSelectors<'_>,
    store: &FakeStore,
) -> Result<Option<LiteLLMUserTable>, KeyListFiltersError> {
    let (teams, _reads) = team_lookup();
    validate_key_list_check(auth, wanted, store, &teams).await
}

fn denied(
    outcome: Result<Option<LiteLLMUserTable>, KeyListFiltersError>,
    param: &str,
) -> KeyListFiltersError {
    let Err(error) = outcome else {
        panic!("expected a denial");
    };
    let exception = error.proxy_exception().expect("a ProxyException denial");
    assert_eq!(error.status(), Some(403_u16));
    assert_eq!(exception.error_type, "bad_request_error");
    assert_eq!(exception.param.as_deref(), Some(param));
    assert_eq!(exception.code, "403");
    error
}

#[tokio::test]
async fn scope_admin_bypass() {
    let store = FakeStore::with_user(user_row(USER_U1));
    let wanted = selectors(Some("u2"), Some("t9"), Some("o9"), None, Some("sk-missing"));
    let admin = check(&caller(None, Some(UserRole::ProxyAdmin)), &wanted, &store).await;
    assert!(admin.unwrap().is_none());
    let viewer = caller(None, Some(UserRole::ProxyAdminViewer));
    assert!(check(&viewer, &wanted, &store).await.unwrap().is_none());
    assert!(store.reads().is_empty());
}

#[tokio::test]
async fn scope_missing_identity() {
    let store = FakeStore::with_user(user_row(USER_U1));
    let auth = caller(None, Some(UserRole::InternalUser));
    let error = denied(check(&auth, &none(), &store).await, "user_id");
    assert!(matches!(error, KeyListFiltersError::NoCallerRow));
    assert!(store.reads().is_empty());
}

#[tokio::test]
async fn scope_missing_user_row() {
    let store = FakeStore::default();
    let auth = caller(Some("u1"), Some(UserRole::InternalUser));
    let error = denied(check(&auth, &none(), &store).await, "user_id");
    assert!(matches!(error, KeyListFiltersError::NoCallerRow));
    assert_eq!(store.reads(), strings(&["user u1"]));
}

#[tokio::test]
async fn scope_foreign_user() {
    let store = FakeStore::with_user(user_row(USER_U1));
    let auth = caller(Some("u1"), Some(UserRole::InternalUser));
    let foreign = selectors(Some("u2"), None, None, None, None);
    let error = denied(check(&auth, &foreign, &store).await, "user_id");
    assert!(matches!(error, KeyListFiltersError::ForeignUser));
    let own = selectors(Some("u1"), None, None, None, None);
    let returned = check(&auth, &own, &store).await.unwrap().unwrap();
    assert_eq!(returned.user_id, "u1");
}

#[tokio::test]
async fn scope_team_membership() {
    let store = FakeStore::with_user(user_row(USER_U1));
    let auth = caller(Some("u1"), Some(UserRole::InternalUser));
    let member = selectors(None, Some("t2"), None, None, None);
    check(&auth, &member, &store).await.unwrap().unwrap();
    let foreign = selectors(None, Some("t9"), None, None, None);
    let error = denied(check(&auth, &foreign, &store).await, "team_id");
    assert!(matches!(error, KeyListFiltersError::ForeignTeam));
}

#[tokio::test]
async fn scope_organization_membership() {
    let store = FakeStore::with_user(user_row(USER_U1));
    let auth = caller(Some("u1"), Some(UserRole::InternalUser));
    let member = selectors(None, None, Some("o1"), None, None);
    check(&auth, &member, &store).await.unwrap().unwrap();
    let foreign = selectors(None, None, Some("o9"), None, None);
    let error = denied(check(&auth, &foreign, &store).await, "organization_id");
    assert!(matches!(error, KeyListFiltersError::ForeignOrganization));
    // a row whose membership list is absent reaches no organization
    let bare = FakeStore::with_user(user_row(r#"{"user_id":"u1","teams":[]}"#));
    let error = denied(check(&auth, &member, &bare).await, "organization_id");
    assert!(matches!(error, KeyListFiltersError::ForeignOrganization));
}

#[tokio::test]
async fn scope_empty_selectors() {
    let store = FakeStore::with_user(user_row(USER_U1));
    let auth = caller(Some("u1"), Some(UserRole::InternalUser));
    let blank = selectors(Some(""), Some(""), Some(""), Some("alias"), Some(""));
    check(&auth, &blank, &store).await.unwrap().unwrap();
    assert_eq!(store.reads(), strings(&["user u1"]));
}

#[tokio::test]
async fn scope_alias_unchecked() {
    let store = FakeStore::with_user(user_row(USER_U1));
    let auth = caller(Some("u1"), Some(UserRole::InternalUser));
    let aliased = selectors(None, None, None, Some("someone-elses-alias"), None);
    check(&auth, &aliased, &store).await.unwrap().unwrap();
    assert_eq!(store.reads(), strings(&["user u1"]));
}

#[tokio::test]
async fn scope_hash_exact() {
    let store = FakeStore::with_user(user_row(USER_U1))
        .with_key(key_row(r#"{"token":"sk-hash-1","user_id":"u1"}"#));
    let auth = caller(Some("u1"), Some(UserRole::InternalUser));
    let wanted = selectors(None, None, None, None, Some("sk-hash-1"));
    check(&auth, &wanted, &store).await.unwrap().unwrap();
    assert_eq!(store.reads(), strings(&["user u1", "key"]));
    let cased = FakeStore::with_user(user_row(USER_U1))
        .with_key(key_row(r#"{"token":"SK-HASH-1","user_id":"u1"}"#));
    let error = denied(check(&auth, &wanted, &cased).await, "key_hash");
    assert!(matches!(error, KeyListFiltersError::KeyHashNotFound));
}

#[tokio::test]
async fn scope_hash_database_error_hidden() {
    let store = FakeStore::with_user(user_row(USER_U1)).failing_key();
    let auth = caller(Some("u1"), Some(UserRole::InternalUser));
    let wanted = selectors(None, None, None, None, Some("sk-hash-1"));
    let error = denied(check(&auth, &wanted, &store).await, "key_hash");
    assert!(matches!(error, KeyListFiltersError::KeyHashNotFound));
    assert_eq!(store.reads(), strings(&["user u1", "key"]));
}

#[tokio::test]
async fn scope_hash_missing() {
    let store = FakeStore::with_user(user_row(USER_U1));
    let auth = caller(Some("u1"), Some(UserRole::InternalUser));
    let wanted = selectors(None, None, None, None, Some("sk-hash-1"));
    let error = denied(check(&auth, &wanted, &store).await, "key_hash");
    assert!(matches!(error, KeyListFiltersError::KeyHashNotFound));
    assert_eq!(store.reads(), strings(&["user u1", "key"]));
}

#[tokio::test]
async fn scope_hash_forbidden() {
    let store = FakeStore::with_user(user_row(USER_U1))
        .with_key(key_row(r#"{"token":"sk-hash-1","user_id":"u2"}"#));
    let auth = caller(Some("u1"), Some(UserRole::InternalUser));
    let wanted = selectors(None, None, None, None, Some("sk-hash-1"));
    let outcome = check(&auth, &wanted, &store).await;
    let Err(error) = outcome else {
        panic!("expected the key-info denial");
    };
    assert!(matches!(
        error,
        KeyListFiltersError::KeyInfoForbidden { .. }
    ));
    assert_eq!(error.status(), Some(403_u16));
    assert!(error.proxy_exception().is_none());
    assert!(error.to_string().contains("internal_user"));
}

#[tokio::test]
async fn scope_returns_user() {
    let store = FakeStore::with_user(user_row(USER_U1))
        .with_key(key_row(r#"{"token":"sk-hash-1","user_id":"u1"}"#));
    let auth = caller(Some("u1"), Some(UserRole::InternalUser));
    let wanted = selectors(None, None, None, None, Some("sk-hash-1"));
    let returned = check(&auth, &wanted, &store).await.unwrap().unwrap();
    assert_eq!(returned.user_id, "u1");
    assert_eq!(returned.teams, strings(&["t1", "t2"]));
    let memberships = returned.organization_memberships.unwrap();
    assert_eq!(memberships.len(), 1);
    assert_eq!(memberships[0].organization_id, "o1");
}
