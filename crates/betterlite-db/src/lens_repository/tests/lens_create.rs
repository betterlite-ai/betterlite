#![cfg(test)]

use crate::lens_repository::support::{
    LENS_ID, database, job_json, lens, lens_with_jobs, repository, stored_lens,
};

#[tokio::test]
async fn create_inserts_version_zero_and_returns_the_lens() {
    let db = database().await;
    let input = lens_with_jobs(
        LENS_ID,
        &format!("[{}]", job_json("job-a", "2026-01-01T00:00:00Z", 1)),
    );
    let created = repository(&db.pool)
        .await
        .create(input.clone())
        .await
        .unwrap();
    assert_eq!(created, input);
    let (version, stored) = stored_lens(&db.pool, LENS_ID).await;
    assert_eq!(version, 0);
    assert_eq!(stored, input);
    db.close().await;
}

#[tokio::test]
async fn create_keeps_zero_in_the_version_column() {
    let db = database().await;
    let mut input = lens(LENS_ID);
    input.version = 7;
    repository(&db.pool)
        .await
        .create(input.clone())
        .await
        .unwrap();
    let (version, stored) = stored_lens(&db.pool, LENS_ID).await;
    assert_eq!(version, 0);
    assert_eq!(stored, input);
    db.close().await;
}
