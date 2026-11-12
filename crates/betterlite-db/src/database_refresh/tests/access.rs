use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;
use tokio::task::yield_now;
use tracing::subscriber::set_default;

use crate::database_refresh::access::{self, BlockingAccessError};
use crate::database_refresh::endpoint::DatabaseUrlSlot;

use super::support::{
    FakeScript, capture_logs, create, lock, refresh, secret, set_url, token_options,
};

#[tokio::test]
async fn fresh_access_no_refresh() {
    let script = Arc::new(FakeScript::default());
    let refresh = Arc::new(refresh());
    let built = create(
        &script,
        token_options(DatabaseUrlSlot::default()),
        Some(Arc::clone(&refresh)),
        None,
        true,
    )
    .await;
    let handle = access::operation(&built).expect("a connected operation");
    assert_eq!(handle.handle.id, 0);
    assert_eq!(refresh.expired_checks.load(Ordering::SeqCst), 1);
    assert_eq!(refresh.refreshes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn async_expired_access_schedules_and_returns_stale() {
    let script = Arc::new(FakeScript::default());
    let gate = Arc::new(Notify::new());
    let fake = refresh();
    fake.expired.store(true, Ordering::SeqCst);
    *lock(&fake.gate) = Some(Arc::clone(&gate));
    let refresh = Arc::new(fake);
    let built = create(
        &script,
        token_options(DatabaseUrlSlot::default()),
        Some(Arc::clone(&refresh)),
        None,
        true,
    )
    .await;
    let (logs, subscriber) = capture_logs();
    let guard = set_default(subscriber);
    let handle = access::operation(&built).expect("the stale handle returns");
    yield_now().await;
    assert_eq!(handle.handle.id, 0);
    assert_eq!(refresh.refreshes.load(Ordering::SeqCst), 1);
    drop(guard);
    let captured = logs.contents();
    assert!(
        captured.contains("proactive refresh may have failed"),
        "{captured}"
    );
    assert!(captured.contains("Scheduling async refresh"), "{captured}");
    gate.notify_one();
    yield_now().await;
}

#[tokio::test]
async fn sync_expired_access_refetches_after_recovery() {
    let script = Arc::new(FakeScript::default());
    let slot = DatabaseUrlSlot::default();
    set_url(&slot, "postgres://initial");
    let mut fake = refresh();
    fake.slot = slot.clone();
    fake.expired.store(true, Ordering::SeqCst);
    fake.minted = Mutex::new(Some(secret("postgres://minted")));
    let refresh = Arc::new(fake);
    let built = create(
        &script,
        token_options(slot),
        Some(Arc::clone(&refresh)),
        None,
        true,
    )
    .await;
    let (logs, subscriber) = capture_logs();
    let guard = set_default(subscriber);
    let handle = access::operation_blocking(&built).expect("the refreshed handle");
    drop(guard);
    assert_eq!(handle.handle.id, 1);
    assert_eq!(built.observe().generation, 1);
    assert_eq!(
        script.build_urls().last().expect("a build"),
        "postgres://minted"
    );
    assert!(
        logs.contents()
            .contains("Synchronous token refresh completed successfully")
    );
}

#[tokio::test]
async fn sync_falsey_mint_exact_error() {
    let script = Arc::new(FakeScript::default());
    let slot = DatabaseUrlSlot::default();
    let mut fake = refresh();
    fake.slot = slot.clone();
    fake.expired.store(true, Ordering::SeqCst);
    let refresh = Arc::new(fake);
    let built = create(
        &script,
        token_options(slot),
        Some(Arc::clone(&refresh)),
        None,
        true,
    )
    .await;
    let missing = access::operation_blocking(&built)
        .err()
        .expect("a missing token fails");
    assert!(matches!(
        missing,
        BlockingAccessError::MissingToken {
            label: "RDS IAM token"
        }
    ));
    assert_eq!(missing.to_string(), "Failed to get RDS IAM token");
    *lock(&refresh.minted) = Some(secret(""));
    let empty = access::operation_blocking(&built)
        .err()
        .expect("an empty token fails");
    assert!(matches!(empty, BlockingAccessError::MissingToken { .. }));
    refresh.mint_fails.store(true, Ordering::SeqCst);
    let failed = access::operation_blocking(&built)
        .err()
        .expect("a mint failure propagates");
    assert!(matches!(failed, BlockingAccessError::Mint { .. }));
    assert_eq!(built.observe().generation, 0);
}
