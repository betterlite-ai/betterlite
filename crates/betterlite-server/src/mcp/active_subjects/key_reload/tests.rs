#![cfg(test)]

use betterlite_auth::auth_key_lookup::key_identity;
use betterlite_auth::user_roles::UserRole;
use betterlite_mcp::gateway_dcr::ports::ReloadUserFailure;
use betterlite_models::key_digest::hashing::hash_token;
use http::HeaderMap;
use http::header::{AUTHORIZATION, HeaderValue};
use rstest::rstest;
use secrecy::SecretString;

use super::super::SubjectReadError;
use super::super::tests::{ADMIN_NAME, FakeStore, KeyAnswer, key_row, subjects};

const MASTER_KEY: &str = "master-secret";

fn master_subjects(store: FakeStore) -> super::super::ActiveSubjects<FakeStore> {
    let mut active = subjects(store);
    active.settings.master_key = Some(SecretString::from(MASTER_KEY));
    active
}

fn with_authorization(token: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, HeaderValue::from_str(token).unwrap());
    headers
}

#[tokio::test]
async fn reload_master_key_needs_no_database_and_reads_no_store() {
    let store = FakeStore {
        has_database: false,
        ..FakeStore::new()
    };
    let calls = store.calls();
    let active = master_subjects(store);
    let hash = hash_token(MASTER_KEY);
    let resolved = active.reload_active_key(&hash).await.unwrap();
    assert_eq!(resolved.key_hash, hash);
    let identity = key_identity(&resolved.key);
    assert_eq!(identity.user_id.as_deref(), Some(ADMIN_NAME));
    assert_eq!(identity.user_role, Some(UserRole::ProxyAdmin));
    assert!(calls.lock().unwrap().keys.is_empty());
}

#[tokio::test]
async fn reload_master_key_mismatch_without_a_database_is_unresolvable() {
    let store = FakeStore {
        has_database: false,
        ..FakeStore::new()
    };
    let active = master_subjects(store);
    let outcome = active.reload_active_key(&hash_token("another-key")).await;
    assert_eq!(outcome.unwrap_err(), ReloadUserFailure::Unresolvable);
}

#[tokio::test]
async fn reload_without_a_database_is_unresolvable() {
    let store = FakeStore {
        has_database: false,
        key: KeyAnswer::Found(key_row(r#"{"token":"sk-active","user_id":"user-1"}"#)),
        ..FakeStore::new()
    };
    let calls = store.calls();
    let active = subjects(store);
    let outcome = active.reload_active_key("hashed-token").await;
    assert_eq!(outcome.unwrap_err(), ReloadUserFailure::Unresolvable);
    assert!(calls.lock().unwrap().keys.is_empty());
}

#[rstest]
#[case::refused(SubjectReadError::Refused, ReloadUserFailure::NoActiveKey)]
#[case::unavailable(SubjectReadError::Unavailable, ReloadUserFailure::Unavailable)]
#[case::faulted(SubjectReadError::Faulted, ReloadUserFailure::Faulted)]
#[case::other(SubjectReadError::Other, ReloadUserFailure::Unresolvable)]
#[tokio::test]
async fn reload_key_failures_keep_their_names(
    #[case] answer: SubjectReadError,
    #[case] expected: ReloadUserFailure,
) {
    let store = FakeStore {
        key: KeyAnswer::Failed(answer),
        ..FakeStore::new()
    };
    let active = subjects(store);
    let outcome = active.reload_active_key("hashed-token").await;
    assert_eq!(outcome.unwrap_err(), expected);
}

#[tokio::test]
async fn reload_active_key_returns_it_with_the_stored_hash() {
    let key = key_row(r#"{"token":"sk-active","user_id":"user-1"}"#);
    let store = FakeStore {
        key: KeyAnswer::Found(key),
        ..FakeStore::new()
    };
    let calls = store.calls();
    let active = subjects(store);
    let resolved = active.reload_active_key("hashed-token").await.unwrap();
    assert_eq!(resolved.key_hash, "hashed-token");
    assert_eq!(
        key_identity(&resolved.key).user_id.as_deref(),
        Some("user-1")
    );
    assert_eq!(calls.lock().unwrap().keys, vec!["hashed-token"]);
}

#[tokio::test]
async fn reload_expired_key_is_no_active_key() {
    let key = key_row(r#"{"token":"sk-old","expires":"2000-01-01T00:00:00Z"}"#);
    let store = FakeStore {
        key: KeyAnswer::Found(key),
        ..FakeStore::new()
    };
    let active = subjects(store);
    let outcome = active.reload_active_key("hashed-token").await;
    assert_eq!(outcome.unwrap_err(), ReloadUserFailure::NoActiveKey);
}

#[tokio::test]
async fn reload_blocked_key_is_no_active_key() {
    let key = key_row(r#"{"token":"sk-blocked","blocked":true}"#);
    let store = FakeStore {
        key: KeyAnswer::Found(key),
        ..FakeStore::new()
    };
    let active = subjects(store);
    let outcome = active.reload_active_key("hashed-token").await;
    assert_eq!(outcome.unwrap_err(), ReloadUserFailure::NoActiveKey);
}

#[tokio::test]
async fn resolve_without_a_token_is_no_active_key() {
    let store = FakeStore::new();
    let calls = store.calls();
    let active = subjects(store);
    let outcome = active.resolve_active_key(&HeaderMap::new()).await;
    assert_eq!(outcome.unwrap_err(), ReloadUserFailure::NoActiveKey);
    assert!(calls.lock().unwrap().keys.is_empty());
}

#[rstest]
#[case::sk_token("Bearer sk-presented", "sk-presented")]
#[case::opaque_token("opaque-token", "opaque-token")]
#[tokio::test]
async fn resolve_hashes_the_presented_token_with_plain_sha256(
    #[case] header: &str,
    #[case] token: &str,
) {
    let key = key_row(r#"{"token":"sk-presented"}"#);
    let store = FakeStore {
        key: KeyAnswer::Found(key),
        ..FakeStore::new()
    };
    let calls = store.calls();
    let active = subjects(store);
    let resolved = active
        .resolve_active_key(&with_authorization(header))
        .await
        .unwrap();
    assert_eq!(resolved.key_hash, hash_token(token));
    assert_eq!(calls.lock().unwrap().keys, vec![hash_token(token)]);
}
