#![cfg(test)]

use axum::http::StatusCode;
use betterlite_lens::models::execution::Sample;
use betterlite_lens::models::job::{Job, Lens};
use chrono::{DateTime, TimeDelta, Utc};

use crate::lens_access::tests::support::now;
use crate::lens_worker_routes::tests::support::{
    Harness, WorkerReply, WorkerStorage, authorized, base_lens, page, running_job, sample_of,
    trace_execution, worker_call, worker_fixture,
};

const TRACING_DISABLED: &str =
    "Agent tracing is not enabled. Set `tracing:` in general_settings and CLICKHOUSE_URL.";
const MOVED: &str = "Job was cancelled or reassigned";

fn later(millis: i64) -> DateTime<Utc> {
    now()
        .checked_add_signed(TimeDelta::milliseconds(millis))
        .expect("a representable instant")
}

fn live_job() -> Job {
    running_job("j1", "w1", later(300_000))
}

fn lens_with(job: Job) -> Lens {
    let mut stored = base_lens("l1");
    stored.jobs = vec![job];
    stored
}

async fn get_sample(harness: &Harness) -> WorkerReply {
    worker_call(
        harness.app(),
        authorized("GET", "/lens/worker/l1/j1/sample", None),
    )
    .await
}

#[tokio::test]
async fn a_stored_sample_returns_without_storage() {
    let harness = Harness::new().without_storage();
    harness.database.seed_worker(worker_fixture());
    let mut job = live_job();
    job.sample = Some(sample_of(vec![trace_execution(0)]));
    harness.database.seed_lens(lens_with(job));
    let reply = get_sample(&harness).await;
    assert_eq!(reply.status, StatusCode::OK);
    let sample: Sample = reply.json();
    assert_eq!(sample, sample_of(vec![trace_execution(0)]));
}

#[tokio::test]
async fn the_sample_pages_by_cursor_until_the_selected_count() {
    let mut harness = Harness::new();
    harness.storage = Some(WorkerStorage::with_pages(&[
        page(100, 0, 500, 150),
        page(50, 100, 500, 150),
    ]));
    harness.database.seed_worker(worker_fixture());
    harness.database.seed_lens(lens_with(live_job()));
    let reply = get_sample(&harness).await;
    assert_eq!(reply.status, StatusCode::OK);
    let calls = harness
        .storage
        .as_ref()
        .expect("the fake storage")
        .sample_calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].limit, 100);
    assert_eq!(calls[0].offset, 0);
    assert_eq!(calls[0].after, "");
    assert!(!calls[0].preview);
    assert_eq!(calls[1].after, "k99");
    let sample: Sample = reply.json();
    assert_eq!(sample.eligible, 500);
    assert_eq!(sample.selected, 150);
    assert_eq!(sample.executions.len(), 150);
    assert_eq!(sample.executions[0].trace_id, "t0");
    assert_eq!(sample.executions[149].trace_id, "t149");
    assert_eq!(sample.next_cursor, None);
    assert_eq!(sample.next_offset, None);
}

#[tokio::test]
async fn the_sample_stops_when_a_full_page_reaches_the_selected_count() {
    let mut harness = Harness::new();
    harness.storage = Some(WorkerStorage::with_pages(&[
        page(100, 0, 500, 120),
        page(100, 100, 500, 120),
    ]));
    harness.database.seed_worker(worker_fixture());
    harness.database.seed_lens(lens_with(live_job()));
    let reply = get_sample(&harness).await;
    assert_eq!(reply.status, StatusCode::OK);
    let calls = harness
        .storage
        .as_ref()
        .expect("the fake storage")
        .sample_calls();
    assert_eq!(calls.len(), 2);
    let sample: Sample = reply.json();
    assert_eq!(sample.eligible, 500);
    assert_eq!(sample.selected, 200);
    assert_eq!(sample.executions.len(), 200);
}

#[tokio::test]
async fn a_page_without_a_cursor_ends_the_gathering() {
    let mut harness = Harness::new();
    harness.storage = Some(WorkerStorage::with_pages(&[page(10, 0, 500, 500)]));
    harness.database.seed_worker(worker_fixture());
    harness.database.seed_lens(lens_with(live_job()));
    let reply = get_sample(&harness).await;
    assert_eq!(reply.status, StatusCode::OK);
    let calls = harness
        .storage
        .as_ref()
        .expect("the fake storage")
        .sample_calls();
    assert_eq!(calls.len(), 1);
    let sample: Sample = reply.json();
    assert_eq!(sample.eligible, 500);
    assert_eq!(sample.selected, 10);
}

#[tokio::test]
async fn the_frozen_sample_is_kept_for_the_next_call() {
    let mut harness = Harness::new();
    harness.storage = Some(WorkerStorage::with_pages(&[page(2, 0, 2, 2)]));
    harness.database.seed_worker(worker_fixture());
    harness.database.seed_lens(lens_with(live_job()));
    let first = get_sample(&harness).await;
    assert_eq!(first.status, StatusCode::OK);
    let second = get_sample(&harness).await;
    assert_eq!(second.status, StatusCode::OK);
    assert_eq!(first.body, second.body);
    let calls = harness
        .storage
        .as_ref()
        .expect("the fake storage")
        .sample_calls();
    assert_eq!(calls.len(), 1);
    let sample: Sample = first.json();
    assert_eq!(sample.selected, 2);
}

#[tokio::test]
async fn another_requests_frozen_sample_wins() {
    let mut harness = Harness::new();
    harness.storage = Some(WorkerStorage::with_pages(&[page(2, 0, 2, 2)]));
    harness.database.seed_worker(worker_fixture());
    harness.database.seed_lens(lens_with(live_job()));
    let mut job = live_job();
    job.sample = Some(sample_of(vec![trace_execution(9)]));
    harness.database.moved_on_read(2, lens_with(job));
    let reply = get_sample(&harness).await;
    assert_eq!(reply.status, StatusCode::OK);
    let sample: Sample = reply.json();
    assert_eq!(sample, sample_of(vec![trace_execution(9)]));
}

#[tokio::test]
async fn a_job_moved_before_the_freeze_is_409() {
    let mut harness = Harness::new();
    harness.storage = Some(WorkerStorage::with_pages(&[page(2, 0, 2, 2)]));
    harness.database.seed_worker(worker_fixture());
    harness.database.seed_lens(lens_with(live_job()));
    let job = running_job("j1", "w2", later(300_000));
    harness.database.moved_on_read(2, lens_with(job));
    let reply = get_sample(&harness).await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.detail(), MOVED);
}

#[tokio::test]
async fn sample_without_storage_is_501() {
    let harness = Harness::new().without_storage();
    harness.database.seed_worker(worker_fixture());
    harness.database.seed_lens(lens_with(live_job()));
    let reply = get_sample(&harness).await;
    assert_eq!(reply.status, StatusCode::NOT_IMPLEMENTED);
    assert_eq!(reply.detail(), TRACING_DISABLED);
    assert!(harness.database.lens("l1").jobs[0].sample.is_none());
}

#[tokio::test]
async fn sample_for_a_missing_lens_is_404() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    let reply = worker_call(
        harness.app(),
        authorized("GET", "/lens/worker/missing/j1/sample", None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.detail(), "Lens not found");
}

#[tokio::test]
async fn the_sample_window_is_in_integer_milliseconds() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    let mut job = live_job();
    job.start = "2026-04-30T12:00:00.123456Z".parse().expect("a job start");
    job.end = "2026-04-30T12:00:00.999999Z".parse().expect("a job end");
    harness.database.seed_lens(lens_with(job));
    let reply = get_sample(&harness).await;
    assert_eq!(reply.status, StatusCode::OK);
    let calls = harness
        .storage
        .as_ref()
        .expect("the fake storage")
        .sample_calls();
    assert_eq!(calls[0].start, 1_777_550_400_123);
    assert_eq!(calls[0].end, 1_777_550_400_999);
}
