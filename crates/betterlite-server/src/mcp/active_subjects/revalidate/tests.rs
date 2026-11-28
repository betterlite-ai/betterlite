#![cfg(test)]

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_mcp::envelope::EnvelopeIdentity;
use betterlite_mcp::envelope::models::SubjectType;
use betterlite_mcp::gateway_dcr::ports::ReloadUserFailure;

use super::super::SubjectReadError;
use super::super::UserRowSource;
use super::super::tests::{FakeStore, KeyAnswer, UserAnswer, key_row, subjects, user_row};
use super::SubjectRevalidation as _;

const KEY_HASH: &str = "hashed-token";

fn owned_key() -> UserAPIKeyAuth {
    key_row(r#"{"token":"sk-owned","user_id":"user-1"}"#)
}

fn key_identity() -> EnvelopeIdentity {
    EnvelopeIdentity {
        server_id: "server-1".to_owned(),
        subject_type: SubjectType::KeyHash,
        subject: KEY_HASH.to_owned(),
    }
}

fn user_identity() -> EnvelopeIdentity {
    EnvelopeIdentity {
        server_id: "server-1".to_owned(),
        subject_type: SubjectType::UserId,
        subject: "user-1".to_owned(),
    }
}

#[tokio::test]
async fn revalidate_live_key_is_none() {
    let key = key_row(r#"{"token":"sk-active"}"#);
    let store = FakeStore {
        key: KeyAnswer::Found(Box::new(key)),
        ..FakeStore::new()
    };
    let calls = store.calls();
    let active = subjects(store);
    assert_eq!(active.revalidate_subject(&key_identity()).await, None);
    assert!(calls.lock().unwrap().users.is_empty());
}

#[tokio::test]
async fn revalidate_key_failure_returns_it() {
    let store = FakeStore {
        key: KeyAnswer::Failed(Box::new(SubjectReadError::Faulted)),
        ..FakeStore::new()
    };
    let active = subjects(store);
    assert_eq!(
        active.revalidate_subject(&key_identity()).await,
        Some(ReloadUserFailure::Faulted)
    );
}

#[tokio::test]
async fn revalidate_live_key_with_a_live_owner_is_none() {
    let store = FakeStore {
        key: KeyAnswer::Found(Box::new(owned_key())),
        user: UserAnswer::Found(Box::new(user_row(r#"{"user_id":"user-1"}"#))),
        ..FakeStore::new()
    };
    let calls = store.calls();
    let active = subjects(store);
    assert_eq!(active.revalidate_subject(&key_identity()).await, None);
    assert_eq!(
        calls.lock().unwrap().users,
        vec![("user-1".to_owned(), UserRowSource::Cache)]
    );
}

#[tokio::test]
async fn revalidate_revokes_a_deactivated_key_owner() {
    let deactivated = user_row(r#"{"user_id":"user-1","metadata":{"scim_active":false}}"#);
    let store = FakeStore {
        key: KeyAnswer::Found(Box::new(owned_key())),
        user: UserAnswer::Found(Box::new(deactivated)),
        ..FakeStore::new()
    };
    let active = subjects(store);
    assert_eq!(
        active.revalidate_subject(&key_identity()).await,
        Some(ReloadUserFailure::NoActiveKey)
    );
}

#[tokio::test]
async fn revalidate_live_user_is_none() {
    let store = FakeStore {
        user: UserAnswer::Found(Box::new(user_row(r#"{"user_id":"user-1"}"#))),
        ..FakeStore::new()
    };
    let calls = store.calls();
    let active = subjects(store);
    assert_eq!(active.revalidate_subject(&user_identity()).await, None);
    assert_eq!(
        calls.lock().unwrap().users,
        vec![("user-1".to_owned(), UserRowSource::Cache)]
    );
}

#[tokio::test]
async fn revalidate_user_failure_returns_it() {
    let store = FakeStore {
        user: UserAnswer::Failed(Box::new(SubjectReadError::Unavailable)),
        ..FakeStore::new()
    };
    let active = subjects(store);
    assert_eq!(
        active.revalidate_subject(&user_identity()).await,
        Some(ReloadUserFailure::Unavailable)
    );
}

#[tokio::test]
async fn revalidate_missing_user_is_no_active_key() {
    let store = FakeStore {
        user: UserAnswer::Missing,
        ..FakeStore::new()
    };
    let active = subjects(store);
    assert_eq!(
        active.revalidate_subject(&user_identity()).await,
        Some(ReloadUserFailure::NoActiveKey)
    );
}

#[tokio::test]
async fn revalidate_scim_deactivated_user_is_no_active_key() {
    let deactivated = user_row(r#"{"user_id":"user-1","metadata":{"scim_active":false}}"#);
    let store = FakeStore {
        user: UserAnswer::Found(Box::new(deactivated)),
        ..FakeStore::new()
    };
    let active = subjects(store);
    assert_eq!(
        active.revalidate_subject(&user_identity()).await,
        Some(ReloadUserFailure::NoActiveKey)
    );
}
