#![cfg(test)]

use crate::lens_repository::support::{
    LENS_ID, database, insert_lens, insert_run, insert_run_with, job, job_json, lens,
    lens_with_jobs, repository,
};

fn seeded_active_jobs() -> String {
    format!(
        "[{},{}]",
        job_json("job-a1", "2026-01-03T00:00:00Z", 1),
        job_json("job-a2", "2026-01-04T00:00:00Z", 1)
    )
}

#[tokio::test]
async fn jobs_combines_archived_and_active_records_newest_first() {
    let db = database().await;
    insert_lens(&db.pool, &lens_with_jobs(LENS_ID, &seeded_active_jobs()), 0).await;
    insert_run(&db.pool, LENS_ID, &job("job-r1", "2026-01-02T00:00:00Z")).await;
    insert_run(&db.pool, LENS_ID, &job("job-r2", "2026-01-01T00:00:00Z")).await;
    let found = repository(&db.pool).await.jobs(LENS_ID, 0).await.unwrap();
    let ids: Vec<&str> = found.iter().map(|record| record.id.as_str()).collect();
    assert_eq!(ids, ["job-a2", "job-a1", "job-r1", "job-r2"]);
    db.close().await;
}

#[tokio::test]
async fn jobs_honours_the_offset() {
    let db = database().await;
    insert_lens(&db.pool, &lens_with_jobs(LENS_ID, &seeded_active_jobs()), 0).await;
    insert_run(&db.pool, LENS_ID, &job("job-r1", "2026-01-02T00:00:00Z")).await;
    insert_run(&db.pool, LENS_ID, &job("job-r2", "2026-01-01T00:00:00Z")).await;
    let found = repository(&db.pool).await.jobs(LENS_ID, 1).await.unwrap();
    let ids: Vec<&str> = found.iter().map(|record| record.id.as_str()).collect();
    assert_eq!(ids, ["job-a1", "job-r1", "job-r2"]);
    db.close().await;
}

#[tokio::test]
async fn jobs_returns_fifty_rows_at_most() {
    let db = database().await;
    insert_lens(&db.pool, &lens(LENS_ID), 0).await;
    let client = db.pool.get().await.unwrap();
    for index in 0..51 {
        let id = format!("job-{index:02}");
        let created = format!("2026-01-01T00:{index:02}:00Z");
        insert_run_with(&client, LENS_ID, &job(&id, &created)).await;
    }
    drop(client);
    let found = repository(&db.pool).await.jobs(LENS_ID, 0).await.unwrap();
    assert_eq!(found.len(), 50);
    assert_eq!(found.first().unwrap().id, "job-50");
    assert_eq!(found.last().unwrap().id, "job-01");
    db.close().await;
}
