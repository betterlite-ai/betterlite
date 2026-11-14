#![cfg(test)]

use betterlite_lens::models::job::Lens;

use crate::lens_repository::support::{
    LENS_ID, database, insert_lens, insert_run, job, job_json, lens_with_jobs, repository,
    stored_runs,
};

fn two_jobs() -> String {
    format!(
        "[{},{}]",
        job_json("job-a", "2026-01-01T00:00:00Z", 1),
        job_json("job-b", "2026-01-02T00:00:00Z", 1)
    )
}

#[tokio::test]
async fn a_removed_job_is_archived_and_a_retained_job_is_not() {
    let db = database().await;
    insert_lens(&db.pool, &lens_with_jobs(LENS_ID, &two_jobs()), 0).await;
    let updated = repository(&db.pool)
        .await
        .update(
            LENS_ID,
            |mut record| {
                record.jobs.retain(|job| job.id != "job-b");
                Ok::<Lens, &str>(record)
            },
            8,
            false,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.jobs.len(), 1);
    let archived = stored_runs(&db.pool, LENS_ID).await;
    assert_eq!(archived.len(), 1);
    assert_eq!(archived[0].id, "job-b");
    db.close().await;
}

#[tokio::test]
async fn an_already_archived_job_keeps_its_row() {
    let db = database().await;
    insert_lens(&db.pool, &lens_with_jobs(LENS_ID, &two_jobs()), 0).await;
    insert_run(&db.pool, LENS_ID, &job("job-b", "2026-01-05T00:00:00Z")).await;
    repository(&db.pool)
        .await
        .update(
            LENS_ID,
            |mut record| {
                record.jobs.retain(|job| job.id != "job-b");
                Ok::<Lens, &str>(record)
            },
            8,
            false,
        )
        .await
        .unwrap()
        .unwrap();
    let archived = stored_runs(&db.pool, LENS_ID).await;
    assert_eq!(archived.len(), 1);
    assert_eq!(
        archived[0].created_at,
        job("job-b", "2026-01-05T00:00:00Z").created_at
    );
    db.close().await;
}
