#![cfg(test)]

use axum::http::StatusCode;
use betterlite_lens::models::job::Lens;
use chrono::{DateTime, TimeDelta, Utc};

use crate::lens_access::tests::support::now;
use crate::lens_worker_routes::tests::support::{
    Harness, authorized, base_lens, progress_body, running_job, worker_call, worker_fixture,
};

const NOT_OWNED: &str = "This worker no longer owns the job";

fn shifted(delta: TimeDelta) -> DateTime<Utc> {
    now()
        .checked_add_signed(delta)
        .expect("a representable instant")
}

fn live_lens(lease: DateTime<Utc>) -> Lens {
    let mut stored = base_lens("l1");
    stored.jobs = vec![running_job("j1", "w1", lease)];
    stored
}

#[tokio::test]
async fn progress_renews_the_lease_and_records_stage_and_coverage() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    harness
        .database
        .seed_lens(live_lens(shifted(TimeDelta::minutes(1))));
    let reply = worker_call(
        harness.app(),
        authorized(
            "POST",
            "/lens/worker/l1/j1/progress",
            Some(&progress_body("Analyzing", 7)),
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, b"true");
    let stored = harness.database.lens("l1");
    assert_eq!(stored.jobs[0].stage, "Analyzing");
    assert_eq!(stored.jobs[0].coverage.screened, 7);
    assert_eq!(
        stored.jobs[0].lease_until,
        Some(shifted(TimeDelta::minutes(5)))
    );
    assert_eq!(harness.database.heartbeats(), ["2026-05-01T12:00:00+00:00"]);
}

#[tokio::test]
async fn progress_writes_the_heartbeat_after_the_update() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    harness
        .database
        .seed_lens(live_lens(shifted(TimeDelta::minutes(1))));
    let reply = worker_call(
        harness.app(),
        authorized(
            "POST",
            "/lens/worker/l1/j1/progress",
            Some(&progress_body("Analyzing", 7)),
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(harness.database.events(), ["update", "heartbeat"]);
}

#[tokio::test]
async fn progress_answers_true_when_the_job_moved_away() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    harness
        .database
        .seed_lens(live_lens(shifted(TimeDelta::minutes(1))));
    let mut replacement = base_lens("l1");
    replacement.jobs = vec![running_job("j1", "w2", shifted(TimeDelta::minutes(5)))];
    harness.database.moved_on_read(2, replacement);
    let reply = worker_call(
        harness.app(),
        authorized(
            "POST",
            "/lens/worker/l1/j1/progress",
            Some(&progress_body("Analyzing", 7)),
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, b"true");
    assert_eq!(harness.database.heartbeats().len(), 1);
}

#[tokio::test]
async fn progress_for_a_missing_lens_is_404() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    let reply = worker_call(
        harness.app(),
        authorized(
            "POST",
            "/lens/worker/missing/j1/progress",
            Some(&progress_body("Analyzing", 7)),
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.detail(), "Lens not found");
}

#[tokio::test]
async fn progress_for_another_workers_job_is_409_without_a_heartbeat() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    let mut stored = base_lens("l1");
    stored.jobs = vec![running_job("j1", "w2", shifted(TimeDelta::minutes(5)))];
    harness.database.seed_lens(stored);
    let reply = worker_call(
        harness.app(),
        authorized(
            "POST",
            "/lens/worker/l1/j1/progress",
            Some(&progress_body("Analyzing", 7)),
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.detail(), NOT_OWNED);
    assert!(harness.database.heartbeats().is_empty());
}

#[tokio::test]
async fn progress_without_a_stage_is_422() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    harness
        .database
        .seed_lens(live_lens(shifted(TimeDelta::minutes(1))));
    let reply = worker_call(
        harness.app(),
        authorized(
            "POST",
            "/lens/worker/l1/j1/progress",
            Some(r#"{"coverage":{"screened":7}}"#),
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let issues = reply.issues();
    assert_eq!(issues[0].kind, "missing");
    assert_eq!(issues[0].loc, ["body", "stage"]);
}

#[tokio::test]
async fn progress_with_a_too_long_stage_is_422() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    harness
        .database
        .seed_lens(live_lens(shifted(TimeDelta::minutes(1))));
    let long = "s".repeat(101);
    let body = format!(r#"{{"stage":"{long}","coverage":{{}}}}"#);
    let reply = worker_call(
        harness.app(),
        authorized("POST", "/lens/worker/l1/j1/progress", Some(&body)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let issues = reply.issues();
    assert_eq!(issues[0].kind, "value_error");
    assert_eq!(issues[0].loc, ["body", "stage"]);
}

#[tokio::test]
async fn heartbeat_renews_the_job_with_its_own_stage_and_coverage() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    let mut stored = base_lens("l1");
    let mut running = running_job("j1", "w1", shifted(TimeDelta::minutes(1)));
    running.stage = "Analyzing".to_owned();
    running.coverage.screened = 3;
    stored.jobs = vec![running];
    harness.database.seed_lens(stored);
    let reply = worker_call(
        harness.app(),
        authorized("POST", "/lens/worker/l1/j1/heartbeat", None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, b"true");
    let stored = harness.database.lens("l1");
    assert_eq!(stored.jobs[0].stage, "Analyzing");
    assert_eq!(stored.jobs[0].coverage.screened, 3);
    assert_eq!(
        stored.jobs[0].lease_until,
        Some(shifted(TimeDelta::minutes(5)))
    );
    assert_eq!(harness.database.heartbeats().len(), 1);
}

#[tokio::test]
async fn heartbeat_runs_assigned_twice() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    harness
        .database
        .seed_lens(live_lens(shifted(TimeDelta::minutes(1))));
    let mut replacement = base_lens("l1");
    replacement.jobs = vec![running_job("j1", "w2", shifted(TimeDelta::minutes(5)))];
    harness.database.moved_on_read(2, replacement);
    let reply = worker_call(
        harness.app(),
        authorized("POST", "/lens/worker/l1/j1/heartbeat", None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.detail(), NOT_OWNED);
    assert!(harness.database.heartbeats().is_empty());
}
