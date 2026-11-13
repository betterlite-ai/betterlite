#![cfg(test)]

use crate::lens_repository::support::{
    database, insert_worker, insert_worker_raw, repository, stored_workers, worker,
};

const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[tokio::test]
async fn set_worker_billing_returns_the_updated_worker() {
    let db = database().await;
    insert_worker(
        &db.pool,
        &worker("worker-a", "2026-01-01T00:00:00Z"),
        "hash-a",
    )
    .await;
    let updated = repository(&db.pool)
        .await
        .set_worker_billing("worker-a", KEY)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.analysis_key_id.as_deref(), Some(KEY));
    let stored = stored_workers(&db.pool).await;
    assert_eq!(stored[0].1.analysis_key_id.as_deref(), Some(KEY));
    db.close().await;
}

#[tokio::test]
async fn set_worker_billing_skips_a_revoked_worker() {
    let db = database().await;
    insert_worker(
        &db.pool,
        &worker("worker-a", "2026-01-01T00:00:00Z"),
        "hash-a",
    )
    .await;
    repository(&db.pool)
        .await
        .revoke_worker("worker-a")
        .await
        .unwrap();
    assert!(
        repository(&db.pool)
            .await
            .set_worker_billing("worker-a", KEY)
            .await
            .unwrap()
            .is_none()
    );
    let stored = stored_workers(&db.pool).await;
    assert_eq!(stored[0].1.analysis_key_id, None);
    assert!(stored[0].1.revoked);
    db.close().await;
}

#[tokio::test]
async fn set_worker_billing_returns_none_for_a_missing_worker() {
    let db = database().await;
    assert!(
        repository(&db.pool)
            .await
            .set_worker_billing("worker-x", KEY)
            .await
            .unwrap()
            .is_none()
    );
    db.close().await;
}

#[tokio::test]
async fn set_worker_billing_allows_a_record_without_the_revoked_field() {
    let db = database().await;
    insert_worker_raw(
        &db.pool,
        "worker-a",
        "hash-a",
        r#"{"id":"worker-a","name":"worker","scope":{},"last_seen":"2026-01-01T00:00:00Z"}"#,
    )
    .await;
    let updated = repository(&db.pool)
        .await
        .set_worker_billing("worker-a", KEY)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.analysis_key_id.as_deref(), Some(KEY));
    db.close().await;
}
