#![cfg(test)]

use crate::lens_repository::support::{
    database, insert_worker, repository, stored_workers, worker,
};

#[tokio::test]
async fn revoke_worker_flips_the_flag_in_the_stored_record() {
    let db = database().await;
    let original = worker("worker-a", "2026-01-01T00:00:00Z");
    insert_worker(&db.pool, &original, "hash-a").await;
    repository(&db.pool)
        .await
        .revoke_worker("worker-a")
        .await
        .unwrap();
    let stored = stored_workers(&db.pool).await;
    assert!(stored[0].1.revoked);
    assert_eq!(stored[0].1.last_seen, original.last_seen);
    db.close().await;
}

#[tokio::test]
async fn revoking_a_missing_worker_writes_nothing() {
    let db = database().await;
    repository(&db.pool)
        .await
        .revoke_worker("worker-x")
        .await
        .unwrap();
    assert!(stored_workers(&db.pool).await.is_empty());
    db.close().await;
}
