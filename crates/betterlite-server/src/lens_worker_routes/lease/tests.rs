#![cfg(test)]

use axum::http::StatusCode;
use betterlite_lens::models::job::{JobStatus, Lens};
use chrono::{DateTime, TimeDelta, Utc};

use crate::lens_access::tests::support::{job, lens_in_team, now};
use crate::lens_worker_routes::lease::assigned;
use crate::lens_worker_routes::tests::support::{
    Harness, base_lens, running_job, team_worker, worker_fixture,
};

const NOT_OWNED: &str = "This worker no longer owns the job";

fn lens_with_lease(lease: DateTime<Utc>) -> Lens {
    let mut stored = base_lens("l1");
    stored.jobs = vec![running_job("j1", "w1", lease)];
    stored
}

fn later(millis: i64) -> DateTime<Utc> {
    now()
        .checked_add_signed(TimeDelta::milliseconds(millis))
        .expect("a representable instant")
}

#[tokio::test]
async fn a_live_lease_returns_the_job() {
    let harness = Harness::new();
    harness.database.seed_lens(lens_with_lease(later(300_000)));
    let state = harness.state();
    let (lens, found) = assigned(&state.routes, "l1", "j1", &worker_fixture(), now())
        .await
        .expect("the job is owned");
    assert_eq!(lens.id, "l1");
    assert_eq!(found.id, "j1");
}

#[tokio::test]
async fn a_missing_lens_is_404() {
    let harness = Harness::new();
    let state = harness.state();
    let error = assigned(&state.routes, "missing", "j1", &worker_fixture(), now())
        .await
        .expect_err("the lens is missing");
    assert_eq!(error.status(), StatusCode::NOT_FOUND);
    assert_eq!(error.detail(), "Lens not found");
}

#[tokio::test]
async fn a_lens_in_another_team_is_404() {
    let harness = Harness::new();
    harness.database.seed_lens(lens_with_lease(later(300_000)));
    let state = harness.state();
    let error = assigned(&state.routes, "l1", "j1", &team_worker(), now())
        .await
        .expect_err("the lens is out of reach");
    assert_eq!(error.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_lens_in_the_workers_own_team_is_visible() {
    let harness = Harness::new();
    let mut stored = lens_in_team("l1", "team-1");
    stored.jobs = vec![running_job("j1", "w1", later(300_000))];
    harness.database.seed_lens(stored);
    let state = harness.state();
    let (lens, _) = assigned(&state.routes, "l1", "j1", &team_worker(), now())
        .await
        .expect("the team lens is owned");
    assert_eq!(lens.id, "l1");
}

#[tokio::test]
async fn another_job_id_is_409() {
    let harness = Harness::new();
    harness.database.seed_lens(lens_with_lease(later(300_000)));
    let state = harness.state();
    let error = assigned(&state.routes, "l1", "j9", &worker_fixture(), now())
        .await
        .expect_err("another job is not owned");
    assert_eq!(error.status(), StatusCode::CONFLICT);
    assert_eq!(error.detail(), NOT_OWNED);
}

#[tokio::test]
async fn a_queued_job_is_409() {
    let harness = Harness::new();
    let mut stored = base_lens("l1");
    let mut queued = job("j1", JobStatus::Queued);
    queued.worker_id = Some("w1".to_owned());
    stored.jobs = vec![queued];
    harness.database.seed_lens(stored);
    let state = harness.state();
    let error = assigned(&state.routes, "l1", "j1", &worker_fixture(), now())
        .await
        .expect_err("a queued job is not owned");
    assert_eq!(error.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn a_finished_job_is_409() {
    let harness = Harness::new();
    let mut stored = base_lens("l1");
    let mut done = job("j1", JobStatus::Completed);
    done.worker_id = Some("w1".to_owned());
    stored.jobs = vec![done];
    harness.database.seed_lens(stored);
    let state = harness.state();
    let error = assigned(&state.routes, "l1", "j1", &worker_fixture(), now())
        .await
        .expect_err("a finished job is not owned");
    assert_eq!(error.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn another_workers_job_is_409() {
    let harness = Harness::new();
    let mut stored = base_lens("l1");
    stored.jobs = vec![running_job("j1", "w2", later(300_000))];
    harness.database.seed_lens(stored);
    let state = harness.state();
    let error = assigned(&state.routes, "l1", "j1", &worker_fixture(), now())
        .await
        .expect_err("another worker owns the job");
    assert_eq!(error.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn a_job_without_a_lease_is_409() {
    let harness = Harness::new();
    let mut stored = base_lens("l1");
    let mut unleased = running_job("j1", "w1", later(300_000));
    unleased.lease_until = None;
    stored.jobs = vec![unleased];
    harness.database.seed_lens(stored);
    let state = harness.state();
    let error = assigned(&state.routes, "l1", "j1", &worker_fixture(), now())
        .await
        .expect_err("a job without a lease is not owned");
    assert_eq!(error.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn a_lease_ending_exactly_now_is_expired() {
    let harness = Harness::new();
    harness.database.seed_lens(lens_with_lease(now()));
    let state = harness.state();
    let error = assigned(&state.routes, "l1", "j1", &worker_fixture(), now())
        .await
        .expect_err("a lease ending now is expired");
    assert_eq!(error.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn a_lease_ending_one_millisecond_later_is_live() {
    let harness = Harness::new();
    harness.database.seed_lens(lens_with_lease(later(1)));
    let state = harness.state();
    let (_, found) = assigned(&state.routes, "l1", "j1", &worker_fixture(), now())
        .await
        .expect("a lease one millisecond out is live");
    assert_eq!(found.id, "j1");
}

#[tokio::test]
async fn a_lens_without_a_current_job_is_409() {
    let harness = Harness::new();
    harness.database.seed_lens(base_lens("l1"));
    let state = harness.state();
    let error = assigned(&state.routes, "l1", "j1", &worker_fixture(), now())
        .await
        .expect_err("there is no job to own");
    assert_eq!(error.status(), StatusCode::CONFLICT);
}
