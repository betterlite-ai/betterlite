#![cfg(test)]

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_mcp::gateway_dcr::ports::{ReloadUser as _, ReloadUserFailure};
use rstest::rstest;

use super::super::tests::{FakeStore, UserAnswer, key_row, subjects, user_row};
use super::super::{SubjectReadError, UserRowSource};
use super::ActiveUserLoad as _;

fn owned_key() -> UserAPIKeyAuth {
    key_row(r#"{"token":"sk-owned","user_id":"user-1"}"#)
}

fn active_user() -> LiteLLMUserTable {
    user_row(r#"{"user_id":"user-1"}"#)
}

#[tokio::test]
async fn load_without_a_database_is_unresolvable() {
    let store = FakeStore {
        has_database: false,
        ..FakeStore::new()
    };
    let calls = store.calls();
    let active = subjects(store);
    let outcome = active
        .load_active_user("user-1", UserRowSource::Cache)
        .await;
    assert_eq!(outcome.unwrap_err(), ReloadUserFailure::Unresolvable);
    assert!(calls.lock().unwrap().users.is_empty());
}

#[rstest]
#[case::refused(SubjectReadError::Refused, ReloadUserFailure::NoActiveKey)]
#[case::unavailable(SubjectReadError::Unavailable, ReloadUserFailure::Unavailable)]
#[case::faulted(SubjectReadError::Faulted, ReloadUserFailure::Faulted)]
#[case::other(SubjectReadError::Other, ReloadUserFailure::NoActiveKey)]
#[tokio::test]
async fn load_failures_keep_their_names(
    #[case] answer: SubjectReadError,
    #[case] expected: ReloadUserFailure,
) {
    let store = FakeStore {
        user: UserAnswer::Failed(Box::new(answer)),
        ..FakeStore::new()
    };
    let active = subjects(store);
    let outcome = active
        .load_active_user("user-1", UserRowSource::Cache)
        .await;
    assert_eq!(outcome.unwrap_err(), expected);
}

#[tokio::test]
async fn load_missing_row_is_no_active_key() {
    let store = FakeStore {
        user: UserAnswer::Missing,
        ..FakeStore::new()
    };
    let active = subjects(store);
    let outcome = active
        .load_active_user("user-1", UserRowSource::Cache)
        .await;
    assert_eq!(outcome.unwrap_err(), ReloadUserFailure::NoActiveKey);
}

#[tokio::test]
async fn load_passes_the_requested_source_to_the_store() {
    let store = FakeStore {
        user: UserAnswer::Found(Box::new(active_user())),
        ..FakeStore::new()
    };
    let calls = store.calls();
    let active = subjects(store);
    let loaded = active
        .load_active_user("user-1", UserRowSource::Database)
        .await
        .unwrap();
    assert_eq!(loaded.user_id, "user-1");
    assert_eq!(
        calls.lock().unwrap().users,
        vec![("user-1".to_owned(), UserRowSource::Database)]
    );
}

#[tokio::test]
async fn load_scim_deactivated_user_is_no_active_key() {
    let deactivated = user_row(r#"{"user_id":"user-1","metadata":{"scim_active":false}}"#);
    let store = FakeStore {
        user: UserAnswer::Found(Box::new(deactivated)),
        ..FakeStore::new()
    };
    let active = subjects(store);
    let outcome = active
        .load_active_user("user-1", UserRowSource::Cache)
        .await;
    assert_eq!(outcome.unwrap_err(), ReloadUserFailure::NoActiveKey);
}

#[tokio::test]
async fn owner_deactivated_needs_a_literal_scim_false() {
    let deactivated = user_row(r#"{"user_id":"user-1","metadata":{"scim_active":false}}"#);
    let store = FakeStore {
        user: UserAnswer::Found(Box::new(deactivated)),
        ..FakeStore::new()
    };
    let calls = store.calls();
    let active = subjects(store);
    assert!(active.key_owner_deactivated(&owned_key()).await);
    assert_eq!(
        calls.lock().unwrap().users,
        vec![("user-1".to_owned(), UserRowSource::Cache)]
    );
}

#[rstest]
#[case::active_user(r#"{"user_id":"user-1","metadata":{"scim_active":true}}"#)]
#[case::zero(r#"{"user_id":"user-1","metadata":{"scim_active":0}}"#)]
#[case::no_metadata(r#"{"user_id":"user-1"}"#)]
#[tokio::test]
async fn owner_deactivated_is_false_for_anything_but_scim_false(#[case] row: &str) {
    let store = FakeStore {
        user: UserAnswer::Found(Box::new(user_row(row))),
        ..FakeStore::new()
    };
    let active = subjects(store);
    assert!(!active.key_owner_deactivated(&owned_key()).await);
}

#[tokio::test]
async fn owner_without_a_user_id_reads_nothing() {
    let key = key_row(r#"{"token":"sk-team","team_id":"team-a"}"#);
    let store = FakeStore {
        user: UserAnswer::Found(Box::new(active_user())),
        ..FakeStore::new()
    };
    let calls = store.calls();
    let active = subjects(store);
    assert!(!active.key_owner_deactivated(&key).await);
    assert!(calls.lock().unwrap().users.is_empty());
}

#[tokio::test]
async fn owner_of_an_inactive_key_reads_nothing() {
    let key = key_row(r#"{"token":"sk-old","user_id":"user-1","expires":"2000-01-01T00:00:00Z"}"#);
    let store = FakeStore {
        user: UserAnswer::Found(Box::new(active_user())),
        ..FakeStore::new()
    };
    let calls = store.calls();
    let active = subjects(store);
    assert!(!active.key_owner_deactivated(&key).await);
    assert!(calls.lock().unwrap().users.is_empty());
}

#[tokio::test]
async fn owner_without_a_database_is_false() {
    let store = FakeStore {
        has_database: false,
        user: UserAnswer::Found(Box::new(active_user())),
        ..FakeStore::new()
    };
    let calls = store.calls();
    let active = subjects(store);
    assert!(!active.key_owner_deactivated(&owned_key()).await);
    assert!(calls.lock().unwrap().users.is_empty());
}

#[tokio::test]
async fn owner_read_error_fails_open() {
    let store = FakeStore {
        user: UserAnswer::Failed(Box::new(SubjectReadError::Faulted)),
        ..FakeStore::new()
    };
    let active = subjects(store);
    assert!(!active.key_owner_deactivated(&owned_key()).await);
}

#[tokio::test]
async fn owner_missing_row_fails_open() {
    let store = FakeStore {
        user: UserAnswer::Missing,
        ..FakeStore::new()
    };
    let active = subjects(store);
    assert!(!active.key_owner_deactivated(&owned_key()).await);
}

#[tokio::test]
async fn reload_user_keeps_only_the_failure() {
    let store = FakeStore {
        user: UserAnswer::Failed(Box::new(SubjectReadError::Unavailable)),
        ..FakeStore::new()
    };
    let active = subjects(store);
    assert_eq!(
        active.reload_user("user-1").await,
        Some(ReloadUserFailure::Unavailable)
    );
}

#[tokio::test]
async fn reload_user_is_none_for_a_live_user() {
    let store = FakeStore {
        user: UserAnswer::Found(Box::new(active_user())),
        ..FakeStore::new()
    };
    let calls = store.calls();
    let active = subjects(store);
    assert_eq!(active.reload_user("user-1").await, None);
    assert_eq!(
        calls.lock().unwrap().users,
        vec![("user-1".to_owned(), UserRowSource::Cache)]
    );
}
