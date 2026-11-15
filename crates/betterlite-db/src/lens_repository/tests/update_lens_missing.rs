#![cfg(test)]

use std::sync::{Arc, Mutex};

use betterlite_lens::models::job::Lens;

use crate::lens_repository::database::WriterDatabase;
use crate::lens_repository::repository::LensRepository;
use crate::lens_repository::support::{LENS_ID, Recorder, database, insert_lens, lens, repository};

#[tokio::test]
async fn a_missing_lens_ends_the_update_before_any_write() {
    let db = database().await;
    let queries = Arc::new(Mutex::new(Vec::new()));
    let repository_instance = LensRepository {
        database: Recorder {
            database: WriterDatabase {
                writer: db.pool.get().await.unwrap(),
            },
            queries: Arc::clone(&queries),
        },
    };
    let mut calls = 0_u32;
    let result = repository_instance
        .update(
            LENS_ID,
            |record| {
                calls = calls.saturating_add(1);
                Ok::<Lens, &str>(record)
            },
            8,
            false,
        )
        .await
        .unwrap();
    assert!(result.is_none());
    assert_eq!(calls, 0);
    let recorded = queries.lock().unwrap().clone();
    assert_eq!(recorded.len(), 1);
    assert!(
        recorded
            .iter()
            .all(|query| !query.starts_with("WITH previous"))
    );
    db.close().await;
}

#[tokio::test]
async fn another_lens_present_does_not_satisfy_the_missing_id() {
    let db = database().await;
    insert_lens(&db.pool, &lens("lens-b"), 0).await;
    let result = repository(&db.pool)
        .await
        .update(LENS_ID, Ok::<Lens, &str>, 8, false)
        .await
        .unwrap();
    assert!(result.is_none());
    db.close().await;
}
