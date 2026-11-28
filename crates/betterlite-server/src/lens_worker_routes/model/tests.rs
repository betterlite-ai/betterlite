#![cfg(test)]

use axum::http::{HeaderName, StatusCode, header};
use betterlite_lens::models::job::Lens;
use betterlite_lens::models::messages::ModelResult;
use chrono::{DateTime, TimeDelta, Utc};
use rust_decimal::Decimal;

use crate::lens_access::tests::support::now;
use crate::lens_worker_routes::tests::support::{
    Harness, authorized, base_lens, model_body, running_job, with_header, worker_call,
    worker_fixture, worker_without_key,
};

const NO_MODELS: &str = "No analysis models are configured";
const NO_KEY: &str = "Assign an analysis key to this worker in Lens setup";
const NO_BUDGET: &str = "Monthly lens budget reached; increase it or wait for next month";

fn live_lens() -> Lens {
    let lease: DateTime<Utc> = now()
        .checked_add_signed(TimeDelta::minutes(5))
        .expect("a representable instant");
    let mut stored = base_lens("l1");
    stored.settings.monthly_budget = Decimal::from(10_000_u32);
    stored.jobs = vec![running_job("j1", "w1", lease)];
    stored
}

fn model_uri() -> &'static str {
    "/lens/worker/l1/j1/model"
}

#[tokio::test]
async fn model_returns_the_models_answer_and_cost() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    harness.database.seed_lens(live_lens());
    let reply = worker_call(
        harness.app(),
        authorized("POST", model_uri(), Some(&model_body("Summarize"))),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let result: ModelResult = reply.json();
    assert_eq!(result.content, r#"{"verdict":"ok"}"#);
    assert_eq!(result.cost, Decimal::new(25, 1));
    let stored = harness.database.lens("l1");
    assert_eq!(stored.spent, Decimal::new(25, 1));
    assert_eq!(stored.jobs[0].cost, Decimal::new(25, 1));
}

#[tokio::test]
async fn the_model_call_runs_under_the_lens_privacy_scope() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    harness.database.seed_lens(live_lens());
    let built = with_header(
        authorized("POST", model_uri(), Some(&model_body("Summarize"))),
        header::HOST,
        "lens.example.com",
    );
    let forwarded = with_header(built, HeaderName::from_static("x-forwarded-proto"), "https");
    let reply = worker_call(harness.app(), forwarded).await;
    assert_eq!(reply.status, StatusCode::OK);
    let calls = harness.calls();
    assert!(
        calls
            .iter()
            .any(|call| call == "run scheme=http client=None privacy=true")
    );
    assert!(calls.iter().any(|call| {
        call == "ip proto=Some(\"https\") host=Some(\"lens.example.com\") scheme=http"
    }));
}

#[tokio::test]
async fn model_without_a_router_is_503() {
    let mut harness = Harness::new();
    harness.without_router();
    harness.database.seed_worker(worker_fixture());
    harness.database.seed_lens(live_lens());
    let reply = worker_call(
        harness.app(),
        authorized("POST", model_uri(), Some(&model_body("Summarize"))),
    )
    .await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.detail(), NO_MODELS);
}

#[tokio::test]
async fn model_without_an_analysis_key_is_409() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_without_key());
    harness.database.seed_lens(live_lens());
    let reply = worker_call(
        harness.app(),
        authorized("POST", model_uri(), Some(&model_body("Summarize"))),
    )
    .await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.detail(), NO_KEY);
}

#[tokio::test]
async fn model_over_budget_is_402() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    let mut stored = live_lens();
    stored.spent = Decimal::from(9_000_u32);
    harness.database.seed_lens(stored);
    let reply = worker_call(
        harness.app(),
        authorized("POST", model_uri(), Some(&model_body("Summarize"))),
    )
    .await;
    assert_eq!(reply.status, StatusCode::PAYMENT_REQUIRED);
    assert_eq!(reply.detail(), NO_BUDGET);
    assert!(!harness.calls().iter().any(|call| call.starts_with("run ")));
}

#[tokio::test]
async fn an_upstream_failure_is_502() {
    let harness = Harness::with_billing(true, Some("2.5"));
    harness.database.seed_worker(worker_fixture());
    harness.database.seed_lens(live_lens());
    let reply = worker_call(
        harness.app(),
        authorized("POST", model_uri(), Some(&model_body("Summarize"))),
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_GATEWAY);
    assert_eq!(reply.detail(), "upstream failed");
    assert!(harness.calls().iter().any(|call| call == "release"));
}

#[tokio::test]
async fn model_for_another_workers_job_is_409() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    let mut stored = base_lens("l1");
    let lease = now()
        .checked_add_signed(TimeDelta::minutes(5))
        .expect("a representable instant");
    stored.jobs = vec![running_job("j1", "w2", lease)];
    harness.database.seed_lens(stored);
    let reply = worker_call(
        harness.app(),
        authorized("POST", model_uri(), Some(&model_body("Summarize"))),
    )
    .await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert!(harness.calls().is_empty());
}

#[tokio::test]
async fn model_without_a_prompt_is_422() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    let reply = worker_call(
        harness.app(),
        authorized("POST", model_uri(), Some(r#"{"purpose":"cluster"}"#)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let issues = reply.issues();
    assert_eq!(issues[0].kind, "missing");
    assert_eq!(issues[0].loc, ["body", "prompt"]);
}

#[tokio::test]
async fn model_for_a_missing_lens_is_404() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    let reply = worker_call(
        harness.app(),
        authorized(
            "POST",
            "/lens/worker/missing/j1/model",
            Some(&model_body("Summarize")),
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.detail(), "Lens not found");
}

#[tokio::test]
async fn model_for_an_idle_job_is_409() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    harness.database.seed_lens(base_lens("l1"));
    let reply = worker_call(
        harness.app(),
        authorized("POST", model_uri(), Some(&model_body("Summarize"))),
    )
    .await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.detail(), "This worker no longer owns the job");
}
