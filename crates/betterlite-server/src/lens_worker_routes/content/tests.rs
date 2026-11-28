#![cfg(test)]

use axum::http::StatusCode;
use betterlite_lens::models::execution::{Execution, ExecutionContent};
use betterlite_lens::models::job::{Job, Lens};
use chrono::{DateTime, TimeDelta, Utc};

use crate::lens_access::tests::support::now;
use crate::lens_worker_routes::tests::support::{
    Harness, WorkerStorage, authorized, base_lens, running_job, sample_of, trace_execution,
    worker_call, worker_fixture,
};

const TRACING_DISABLED: &str =
    "Agent tracing is not enabled. Set `tracing:` in general_settings and CLICKHOUSE_URL.";
const OUTSIDE_SAMPLE: &str = "Execution is outside this job's sample";

fn part() -> &'static str {
    concat!(
        r#"[{"span_id":"s1","parent_span_id":"","name":"root","kind":"llm","#,
        r#""content":"boom","truncated":0}]"#
    )
}

fn live_job() -> Job {
    let lease: DateTime<Utc> = now()
        .checked_add_signed(TimeDelta::minutes(5))
        .expect("a representable instant");
    running_job("j1", "w1", lease)
}

fn lens_with_job(job: Job) -> Lens {
    let mut stored = base_lens("l1");
    stored.jobs = vec![job];
    stored
}

fn lens_with_sample(execution: &Execution) -> Lens {
    let mut job = live_job();
    job.sample = Some(sample_of(vec![execution.clone()]));
    lens_with_job(job)
}

fn content_uri(execution: &Execution) -> String {
    format!("/lens/worker/l1/j1/content?execution_id={}", execution.id)
}

#[tokio::test]
async fn content_returns_the_readers_page() {
    let mut harness = Harness::new();
    harness.storage = Some(WorkerStorage::with_pages(&[]).answering_content(part()));
    harness.database.seed_worker(worker_fixture());
    let execution = trace_execution(0);
    harness.database.seed_lens(lens_with_sample(&execution));
    let reply = worker_call(
        harness.app(),
        authorized("GET", &content_uri(&execution), None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: ExecutionContent = reply.json();
    assert_eq!(body.execution, execution);
    assert_eq!(body.parts.len(), 1);
    assert_eq!(body.parts[0].execution_id, execution.id);
    assert_eq!(body.parts[0].span_id, "s1");
    assert_eq!(body.parts[0].content, "boom");
    assert!(!body.parts[0].truncated);
    assert_eq!(body.next_cursor, None);
    assert!(body.partial);
    let calls = harness
        .storage
        .as_ref()
        .expect("the fake storage")
        .content_calls();
    assert_eq!(calls[0].id, "t0");
    assert_eq!(calls[0].trace_ref, "");
    assert_eq!(calls[0].record_team, "team-1");
    assert_eq!(calls[0].cursor, "");
    assert_eq!(calls[0].offset, 1);
}

#[tokio::test]
async fn content_passes_the_cursor_and_increments_the_offset() {
    let mut harness = Harness::new();
    harness.storage = Some(WorkerStorage::with_pages(&[]).answering_content(part()));
    harness.database.seed_worker(worker_fixture());
    let execution = trace_execution(0);
    harness.database.seed_lens(lens_with_sample(&execution));
    let uri = format!("{}&cursor=c1&offset=3", content_uri(&execution));
    let reply = worker_call(harness.app(), authorized("GET", &uri, None)).await;
    assert_eq!(reply.status, StatusCode::OK);
    let calls = harness
        .storage
        .as_ref()
        .expect("the fake storage")
        .content_calls();
    assert_eq!(calls[0].cursor, "c1");
    assert_eq!(calls[0].offset, 4);
}

#[tokio::test]
async fn content_without_an_execution_id_is_422_before_storage() {
    let harness = Harness::new().without_storage();
    harness.database.seed_worker(worker_fixture());
    let reply = worker_call(
        harness.app(),
        authorized("GET", "/lens/worker/l1/j1/content", None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let issues = reply.issues();
    assert_eq!(issues[0].kind, "missing");
    assert_eq!(issues[0].loc, ["query", "execution_id"]);
}

#[tokio::test]
async fn an_execution_outside_the_sample_is_404_before_storage() {
    let harness = Harness::new().without_storage();
    harness.database.seed_worker(worker_fixture());
    harness
        .database
        .seed_lens(lens_with_sample(&trace_execution(0)));
    let reply = worker_call(
        harness.app(),
        authorized("GET", &content_uri(&trace_execution(5)), None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.detail(), OUTSIDE_SAMPLE);
}

#[tokio::test]
async fn a_job_without_a_sample_has_no_selectable_execution() {
    let harness = Harness::new().without_storage();
    harness.database.seed_worker(worker_fixture());
    harness.database.seed_lens(lens_with_job(live_job()));
    let reply = worker_call(
        harness.app(),
        authorized("GET", &content_uri(&trace_execution(0)), None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.detail(), OUTSIDE_SAMPLE);
}

#[tokio::test]
async fn content_without_storage_is_501() {
    let harness = Harness::new().without_storage();
    harness.database.seed_worker(worker_fixture());
    let execution = trace_execution(0);
    harness.database.seed_lens(lens_with_sample(&execution));
    let reply = worker_call(
        harness.app(),
        authorized("GET", &content_uri(&execution), None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_IMPLEMENTED);
    assert_eq!(reply.detail(), TRACING_DISABLED);
}

#[tokio::test]
async fn a_non_integer_offset_is_422() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    let reply = worker_call(
        harness.app(),
        authorized(
            "GET",
            "/lens/worker/l1/j1/content?execution_id=e1&offset=abc",
            None,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let issues = reply.issues();
    assert_eq!(issues[0].kind, "int_parsing");
    assert_eq!(issues[0].loc, ["query", "offset"]);
}

#[tokio::test]
async fn a_negative_offset_is_422() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    let reply = worker_call(
        harness.app(),
        authorized(
            "GET",
            "/lens/worker/l1/j1/content?execution_id=e1&offset=-1",
            None,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let issues = reply.issues();
    assert_eq!(issues[0].kind, "greater_than_equal");
    assert_eq!(issues[0].loc, ["query", "offset"]);
}

#[tokio::test]
async fn a_repeated_execution_id_uses_the_last_value() {
    let mut harness = Harness::new();
    harness.storage = Some(WorkerStorage::with_pages(&[]).answering_content(part()));
    harness.database.seed_worker(worker_fixture());
    let execution = trace_execution(0);
    harness.database.seed_lens(lens_with_sample(&execution));
    let uri = format!(
        "/lens/worker/l1/j1/content?execution_id=other&execution_id={}",
        execution.id
    );
    let reply = worker_call(harness.app(), authorized("GET", &uri, None)).await;
    assert_eq!(reply.status, StatusCode::OK);
    let calls = harness
        .storage
        .as_ref()
        .expect("the fake storage")
        .content_calls();
    assert_eq!(calls[0].id, "t0");
}

#[tokio::test]
async fn content_for_a_missing_lens_is_404() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    let reply = worker_call(
        harness.app(),
        authorized("GET", &content_uri(&trace_execution(0)), None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.detail(), "Lens not found");
}

#[tokio::test]
async fn a_storage_failure_is_500() {
    let mut harness = Harness::new();
    harness.storage = Some(WorkerStorage::with_pages(&[]).failing());
    harness.database.seed_worker(worker_fixture());
    let execution = trace_execution(0);
    harness.database.seed_lens(lens_with_sample(&execution));
    let reply = worker_call(
        harness.app(),
        authorized("GET", &content_uri(&execution), None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(reply.detail().contains("the fake storage failed"));
}
