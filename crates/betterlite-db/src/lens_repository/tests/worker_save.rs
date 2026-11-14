#![cfg(test)]

use crate::lens_repository::support::{database, repository, stored_workers, worker};

#[tokio::test]
async fn save_worker_inserts_a_worker_with_its_token_hash() {
    let db = database().await;
    let input = worker("worker-a", "2026-01-01T00:00:00Z");
    repository(&db.pool)
        .await
        .save_worker(&input, Some("hash-a"))
        .await
        .unwrap();
    let stored = stored_workers(&db.pool).await;
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].0, "hash-a");
    assert_eq!(stored[0].1, input);
    assert_eq!(
        repository(&db.pool)
            .await
            .worker("hash-a")
            .await
            .unwrap()
            .unwrap(),
        input
    );
    db.close().await;
}

#[tokio::test]
async fn save_worker_updates_an_existing_worker_by_id() {
    let db = database().await;
    let mut input = worker("worker-a", "2026-01-01T00:00:00Z");
    repository(&db.pool)
        .await
        .save_worker(&input, Some("hash-a"))
        .await
        .unwrap();
    input.name = "renamed".to_owned();
    repository(&db.pool)
        .await
        .save_worker(&input, None)
        .await
        .unwrap();
    let stored = stored_workers(&db.pool).await;
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].0, "hash-a");
    assert_eq!(stored[0].1.name, "renamed");
    db.close().await;
}

#[tokio::test]
async fn save_worker_treats_an_empty_token_hash_as_an_insert() {
    let db = database().await;
    let input = worker("worker-c", "2026-01-01T00:00:00Z");
    repository(&db.pool)
        .await
        .save_worker(&input, Some(""))
        .await
        .unwrap();
    let stored = stored_workers(&db.pool).await;
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].0, "");
    db.close().await;
}
