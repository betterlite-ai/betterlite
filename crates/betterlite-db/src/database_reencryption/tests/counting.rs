use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use betterlite_secrets::secret_codec::Algorithm;
use deadpool_postgres::PoolError;
use secrecy::SecretString;
use tokio::sync::Notify;
use tokio::task::{yield_now, JoinSet};

use super::fixtures::{column, events, recorded, row, sealed, Events, FakeStorage, PREVIOUS_KEY};
use crate::database_reencryption::counting::{
    count_encrypted_or_none, count_values_encrypted_with,
};
use crate::database_reencryption::registry::SecretColumn;
use crate::database_reencryption::MigrationError;
use crate::postgres_store::diagnostics::PoolFailure;

fn credentials() -> SecretColumn {
    column("LiteLLM_CredentialsTable", "credential_values")
}

#[tokio::test]
async fn unreadable_count_returns_none_not_zero() {
    let events = events();
    let storage = FakeStorage::new(&events, vec![credentials()]);
    storage.script_unavailable();
    let key = SecretString::from(PREVIOUS_KEY);
    let connects = Arc::new(AtomicUsize::new(0));
    let handle = Arc::clone(&connects);
    let counted = count_encrypted_or_none(
        move || {
            handle.fetch_add(1, Ordering::SeqCst);
            Ok(storage)
        },
        &key,
        100,
    )
    .await;
    assert_eq!(counted, None, "a read failure is not zero");
    assert_eq!(connects.load(Ordering::SeqCst), 1, "connect runs once");
}

#[tokio::test]
async fn unreadable_connect_returns_none() {
    let key = SecretString::from(PREVIOUS_KEY);
    let counted = count_encrypted_or_none(
        || {
            Err::<FakeStorage, _>(MigrationError::Connect(PoolFailure {
                driver: PoolError::Closed,
            }))
        },
        &key,
        100,
    )
    .await;
    assert_eq!(counted, None);
}

#[tokio::test]
async fn readable_count_returns_some_zero() {
    let events = events();
    let storage = FakeStorage::new(&events, vec![credentials()]);
    storage.script_rows(vec![row("credential-1", "{\"api_key\":\"plain\"}")]);
    let key = SecretString::from(PREVIOUS_KEY);
    let counted = count_encrypted_or_none(move || Ok(storage), &key, 100).await;
    assert_eq!(counted, Some(0));
}

#[tokio::test]
async fn counting_reads_only_and_counts_leaves() {
    let events = events();
    let storage = FakeStorage::new(&events, vec![credentials()]);
    let legacy = sealed("hunter2", PREVIOUS_KEY, Algorithm::XSalsa20Poly1305);
    let aes = sealed("other", PREVIOUS_KEY, Algorithm::Aes256Gcm);
    storage.script_rows(vec![row(
        "credential-1",
        &format!("{{\"a\":\"{legacy}\",\"b\":[\"{aes}\",\"plain\"],\"c\":\"\"}}"),
    )]);
    let key = SecretString::from(PREVIOUS_KEY);
    let counted = count_values_encrypted_with(&storage, &key, 100)
        .await
        .expect("the count succeeds");
    assert_eq!(counted, 2, "legacy and AES ciphertexts count as leaves");
    assert!(storage.replaced().is_empty(), "counting never writes");
}

#[tokio::test]
async fn counting_respects_the_depth_limit() {
    let events = events();
    let storage = FakeStorage::new(&events, vec![credentials()]);
    let legacy = sealed("hunter2", PREVIOUS_KEY, Algorithm::XSalsa20Poly1305);
    storage.script_rows(vec![row("credential-1", &format!("[\"{legacy}\"]"))]);
    let key = SecretString::from(PREVIOUS_KEY);
    let counted = count_values_encrypted_with(&storage, &key, 0)
        .await
        .expect("the count succeeds");
    assert_eq!(counted, 0, "the child sits past a zero limit");
}

#[tokio::test]
async fn counting_cancels_without_an_outcome() {
    let events = events();
    let gate = Arc::new(Notify::new());
    let storage = FakeStorage::new(&events, vec![credentials()]).block_on_read(Arc::clone(&gate));
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        let key = SecretString::from(PREVIOUS_KEY);
        count_values_encrypted_with(&storage, &key, 100).await
    });
    let started = wait_for_read(&events).await;
    assert!(started, "the count blocks inside the read");
    tasks.abort_all();
    let joined = tasks
        .join_next()
        .await
        .expect("the task was spawned")
        .expect_err("cancellation propagates");
    assert!(joined.is_cancelled());
}

async fn wait_for_read(events: &Events) -> bool {
    for _ in 0_u16..10_000_u16 {
        if recorded(events)
            .iter()
            .any(|event| event.starts_with("rows:"))
        {
            return true;
        }
        yield_now().await;
    }
    false
}
