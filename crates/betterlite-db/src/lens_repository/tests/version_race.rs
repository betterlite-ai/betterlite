#![cfg(test)]

use betterlite_lens::models::job::Lens;

use crate::lens_repository::support::{
    LENS_ID, database, insert_lens, lens, racing, repository, stored_lens,
};

#[tokio::test]
async fn a_matching_version_completes_the_update_and_stores_it() {
    let db = database().await;
    insert_lens(&db.pool, &lens(LENS_ID), 0).await;
    let mut transform = |mut record: Lens| {
        record.revision = 2;
        Ok::<Lens, &str>(record)
    };
    let (completed, returned) = repository(&db.pool)
        .await
        .try_update(LENS_ID, &mut transform, false)
        .await
        .unwrap();
    assert!(completed);
    let updated = returned.unwrap();
    assert_eq!((updated.revision, updated.version), (2, 1));
    let (version, stored) = stored_lens(&db.pool, LENS_ID).await;
    assert_eq!((version, stored.revision), (1, 2));
    db.close().await;
}

#[tokio::test]
async fn a_lost_version_race_reports_incomplete_and_leaves_the_row() {
    let db = database().await;
    insert_lens(&db.pool, &lens(LENS_ID), 0).await;
    let racing_repository = racing(&db.pool, LENS_ID, 1).await;
    let mut transform = |mut record: Lens| {
        record.revision = 2;
        Ok::<Lens, &str>(record)
    };
    let (completed, returned) = racing_repository
        .repository
        .try_update(LENS_ID, &mut transform, false)
        .await
        .unwrap();
    assert!(!completed);
    let updated = returned.unwrap();
    assert_eq!((updated.revision, updated.version), (2, 1));
    let (version, stored) = stored_lens(&db.pool, LENS_ID).await;
    assert_eq!((version, stored.revision), (1, 1));
    db.close().await;
}
