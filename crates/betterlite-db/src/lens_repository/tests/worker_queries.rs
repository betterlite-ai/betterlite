#![cfg(test)]

use crate::lens_repository::support::{database, insert_worker, repository, worker};

#[tokio::test]
async fn workers_returns_every_worker() {
    let db = database().await;
    insert_worker(
        &db.pool,
        &worker("worker-a", "2026-01-01T00:00:00Z"),
        "hash-a",
    )
    .await;
    insert_worker(
        &db.pool,
        &worker("worker-b", "2026-01-01T00:00:00Z"),
        "hash-b",
    )
    .await;
    let mut found = repository(&db.pool).await.workers().await.unwrap();
    found.sort_by(|left, right| left.id.cmp(&right.id));
    let ids: Vec<&str> = found.iter().map(|record| record.id.as_str()).collect();
    assert_eq!(ids, ["worker-a", "worker-b"]);
    db.close().await;
}

#[tokio::test]
async fn worker_reads_by_token_hash() {
    let db = database().await;
    insert_worker(
        &db.pool,
        &worker("worker-a", "2026-01-01T00:00:00Z"),
        "hash-a",
    )
    .await;
    let repository = repository(&db.pool).await;
    assert_eq!(
        repository.worker("hash-a").await.unwrap().unwrap().id,
        "worker-a"
    );
    assert!(repository.worker("hash-missing").await.unwrap().is_none());
    db.close().await;
}
