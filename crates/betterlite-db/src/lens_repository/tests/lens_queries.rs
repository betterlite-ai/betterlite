#![cfg(test)]

use crate::lens_repository::support::{LENS_ID, database, insert_lens, lens, repository};

#[tokio::test]
async fn lenses_returns_every_lens_ordered_by_id() {
    let db = database().await;
    insert_lens(&db.pool, &lens("lens-b"), 0).await;
    insert_lens(&db.pool, &lens(LENS_ID), 0).await;
    let found = repository(&db.pool).await.lenses().await.unwrap();
    let ids: Vec<&str> = found.iter().map(|record| record.id.as_str()).collect();
    assert_eq!(ids, ["lens-a", "lens-b"]);
    db.close().await;
}

#[tokio::test]
async fn get_returns_one_lens_or_none() {
    let db = database().await;
    insert_lens(&db.pool, &lens(LENS_ID), 0).await;
    let repository = repository(&db.pool).await;
    assert_eq!(repository.get(LENS_ID).await.unwrap(), Some(lens(LENS_ID)));
    assert!(repository.get("missing").await.unwrap().is_none());
    db.close().await;
}
