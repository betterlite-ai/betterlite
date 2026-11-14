#![cfg(test)]

use std::sync::{Arc, Mutex};

use betterlite_lens::models::job::Lens;

use crate::lens_repository::database::WriterDatabase;
use crate::lens_repository::repository::LensRepository;
use crate::lens_repository::support::{
    LENS_ID, Recorder, database, insert_lens, lens, repository, stored_lens,
};

#[tokio::test]
async fn changed_only_true_returns_none_for_an_unchanged_lens() {
    let db = database().await;
    insert_lens(&db.pool, &lens(LENS_ID), 0).await;
    let queries = Arc::new(Mutex::new(Vec::new()));
    let repository_instance = LensRepository {
        database: Recorder {
            database: WriterDatabase {
                writer: db.pool.get().await.unwrap(),
            },
            queries: Arc::clone(&queries),
        },
    };
    let result = repository_instance
        .update(LENS_ID, Ok::<Lens, &str>, 8, true)
        .await
        .unwrap();
    assert!(result.is_none());
    assert!(
        queries
            .lock()
            .unwrap()
            .iter()
            .all(|query| !query.starts_with("WITH previous"))
    );
    let (version, _stored) = stored_lens(&db.pool, LENS_ID).await;
    assert_eq!(version, 0);
    db.close().await;
}

#[tokio::test]
async fn changed_only_false_returns_the_previous_lens_for_an_unchanged_one() {
    let db = database().await;
    let original = lens(LENS_ID);
    insert_lens(&db.pool, &original, 0).await;
    let result = repository(&db.pool)
        .await
        .update(LENS_ID, Ok::<Lens, &str>, 8, false)
        .await
        .unwrap();
    assert_eq!(result, Some(original));
    let (version, _stored) = stored_lens(&db.pool, LENS_ID).await;
    assert_eq!(version, 0);
    db.close().await;
}
