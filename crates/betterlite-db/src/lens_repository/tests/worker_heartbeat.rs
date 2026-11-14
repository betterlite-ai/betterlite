#![cfg(test)]

use betterlite_lens::models::job::Worker;

use crate::lens_repository::support::{
    database, insert_worker, repository, stored_worker_text, stored_workers, worker,
};

#[tokio::test]
async fn heartbeat_stores_the_given_timestamp_text_unchanged() {
    let db = database().await;
    insert_worker(
        &db.pool,
        &worker("worker-a", "2026-01-01T00:00:00Z"),
        "hash-a",
    )
    .await;
    let now = "2026-02-03T04:05:06.789+02:00";
    repository(&db.pool)
        .await
        .heartbeat("worker-a", now)
        .await
        .unwrap();
    assert_eq!(
        stored_worker_text(&db.pool, "worker-a", "last_seen").await,
        now
    );
    let stored = stored_workers(&db.pool).await;
    let expected: Worker = serde_json::from_str(&format!(
        r#"{{"id":"worker-a","name":"worker","scope":{{}},"last_seen":"{now}"}}"#
    ))
    .unwrap();
    assert_eq!(stored[0].1.last_seen, expected.last_seen);
    db.close().await;
}

#[tokio::test]
async fn heartbeat_on_a_missing_worker_writes_nothing() {
    let db = database().await;
    repository(&db.pool)
        .await
        .heartbeat("worker-x", "2026-02-03T04:05:06Z")
        .await
        .unwrap();
    assert!(stored_workers(&db.pool).await.is_empty());
    db.close().await;
}
