#![cfg(test)]

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use betterlite_lens::models::job::Lens;

use crate::lens_repository::database::WriterDatabase;
use crate::lens_repository::repository::LensRepository;
use crate::lens_repository::support::{
    LENS_ID, Recorder, database, insert_lens, lens, racing, repository, stored_lens,
};
use crate::lens_repository::update::LensUpdateError;

#[tokio::test]
async fn a_changed_lens_is_written_on_the_first_attempt() {
    let db = database().await;
    insert_lens(&db.pool, &lens(LENS_ID), 0).await;
    let mut calls = 0_u32;
    let updated = repository(&db.pool)
        .await
        .update(
            LENS_ID,
            |mut record| {
                calls = calls.saturating_add(1);
                record.revision = 2;
                Ok::<Lens, &str>(record)
            },
            8,
            false,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!((calls, updated.revision, updated.version), (1, 2, 1));
    let (version, stored) = stored_lens(&db.pool, LENS_ID).await;
    assert_eq!((version, stored.revision), (1, 2));
    db.close().await;
}

#[tokio::test]
async fn a_failing_transform_ends_the_update_without_a_write() {
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
    let mut calls = 0_u32;
    let result = repository_instance
        .update(
            LENS_ID,
            |_record| {
                calls = calls.saturating_add(1);
                Err::<Lens, &str>("the transform failed")
            },
            8,
            false,
        )
        .await;
    assert!(matches!(
        result,
        Err(LensUpdateError::Transform("the transform failed"))
    ));
    assert_eq!(calls, 1);
    assert_eq!(queries.lock().unwrap().len(), 1);
    let (version, _stored) = stored_lens(&db.pool, LENS_ID).await;
    assert_eq!(version, 0);
    db.close().await;
}

#[tokio::test]
async fn every_lost_attempt_is_retried_until_the_attempts_run_out() {
    let db = database().await;
    insert_lens(&db.pool, &lens(LENS_ID), 0).await;
    let racing_repository = racing(&db.pool, LENS_ID, 8).await;
    let mut calls = 0_u32;
    let result = racing_repository
        .repository
        .update(
            LENS_ID,
            |mut record| {
                calls = calls.saturating_add(1);
                record.revision = 2;
                Ok::<Lens, &str>(record)
            },
            8,
            false,
        )
        .await
        .unwrap();
    assert!(result.is_none());
    assert_eq!(calls, 8);
    assert_eq!(racing_repository.races.load(Ordering::SeqCst), 0);
    let (version, stored) = stored_lens(&db.pool, LENS_ID).await;
    assert_eq!((version, stored.revision), (8, 1));
    db.close().await;
}

#[tokio::test]
async fn zero_attempts_returns_none_without_reading_the_lens() {
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
    let result = repository_instance
        .update(LENS_ID, |record| Ok::<Lens, &str>(record), 0, false)
        .await
        .unwrap();
    assert!(result.is_none());
    assert!(queries.lock().unwrap().is_empty());
    db.close().await;
}
