#![cfg(test)]

use crate::lens_repository::support::{
    LENS_ID, database, insert_lens, insert_run, job, job_json, lens, lens_with_jobs, repository,
};

#[tokio::test]
async fn job_reads_an_archived_record() {
    let db = database().await;
    insert_lens(&db.pool, &lens(LENS_ID), 0).await;
    insert_run(&db.pool, LENS_ID, &job("job-r", "2026-01-01T00:00:00Z")).await;
    let found = repository(&db.pool)
        .await
        .job(LENS_ID, "job-r")
        .await
        .unwrap()
        .unwrap();
    assert_eq!((found.id.as_str(), found.revision), ("job-r", 1));
    db.close().await;
}

#[tokio::test]
async fn job_reads_an_active_record() {
    let db = database().await;
    let active = format!("[{}]", job_json("job-a", "2026-01-01T00:00:00Z", 3));
    insert_lens(&db.pool, &lens_with_jobs(LENS_ID, &active), 0).await;
    let found = repository(&db.pool)
        .await
        .job(LENS_ID, "job-a")
        .await
        .unwrap()
        .unwrap();
    assert_eq!((found.id.as_str(), found.revision), ("job-a", 3));
    db.close().await;
}

#[tokio::test]
async fn job_returns_none_when_neither_table_has_the_id() {
    let db = database().await;
    insert_lens(&db.pool, &lens(LENS_ID), 0).await;
    insert_run(&db.pool, LENS_ID, &job("job-r", "2026-01-01T00:00:00Z")).await;
    assert!(
        repository(&db.pool)
            .await
            .job(LENS_ID, "job-x")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        repository(&db.pool)
            .await
            .job("lens-x", "job-r")
            .await
            .unwrap()
            .is_none()
    );
    db.close().await;
}
