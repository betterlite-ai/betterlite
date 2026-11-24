#![cfg(test)]

use axum::http::StatusCode;
use betterlite_lens::models::job::{JobStatus, Lens, LensList};
use betterlite_lens::sources::activity_availability::ActivityAvailability;

use crate::lens_access::tests::support::{
    FakeStorage, Harness, admin, call, job, lens, lens_in_team, member, minutes_after, now,
    request, settings_json, worker,
};

const WRITE_DENIED: &str = "Only proxy admins can configure or run Lens";
const UNKNOWN_MODEL: &str = "Choose a model configured on this LiteLLM instance";

#[tokio::test]
async fn list_lenses_returns_lenses_workers_and_tracing() {
    let harness = Harness::new();
    harness.database.seed_lens(lens("l1"));
    harness.database.seed_lens(lens_in_team("l2", "team-1"));
    harness.database.seed_worker(worker("w1"));
    let reply = call(harness.app(), request("GET", "/lens", Some(&admin()), None)).await;
    assert_eq!(reply.status, StatusCode::OK);
    let list: LensList = reply.json();
    assert_eq!(list.lenses.len(), 2);
    assert_eq!(list.workers.len(), 1);
    assert_eq!(list.workers[0].id, "w1");
    assert!(list.tracing_enabled);
}

#[tokio::test]
async fn without_storage_the_list_reports_tracing_disabled() {
    let harness = Harness::new().without_storage();
    let reply = call(harness.app(), request("GET", "/lens", Some(&admin()), None)).await;
    let list: LensList = reply.json();
    assert!(!list.tracing_enabled);
}

#[tokio::test]
async fn a_repository_failure_is_a_500_that_keeps_its_text() {
    let harness = Harness::new();
    harness.database.corrupted();
    let reply = call(harness.app(), request("GET", "/lens", Some(&admin()), None)).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!reply.detail().is_empty());
}

#[tokio::test]
async fn the_lens_list_is_read_before_the_worker_list() {
    let harness = Harness::new();
    harness.database.seed_worker(worker("w1"));
    harness.database.seed_lens(lens("l1"));
    let reply = call(harness.app(), request("GET", "/lens", Some(&admin()), None)).await;
    assert_eq!(reply.status, StatusCode::OK);
    let queries = harness.database.queries();
    let lenses = queries
        .iter()
        .position(|query| query.contains("ORDER BY id"))
        .expect("the lens query runs");
    let workers = queries
        .iter()
        .position(|query| query.contains("\"LiteLLM_LensWorker\""))
        .expect("the worker query runs");
    assert!(lenses < workers);
}

#[tokio::test]
async fn create_lens_queues_the_first_job() {
    let harness = Harness::new();
    let body = settings_json("gpt-4o");
    let reply = call(
        harness.app(),
        request("POST", "/lens", Some(&admin()), Some(&body)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let created: Lens = reply.json();
    assert_eq!(created.id, "id-1");
    assert_eq!(created.revision, 1);
    assert_eq!(created.version, 0);
    assert_eq!(created.budget_month, "2026-05");
    assert_eq!(created.created_at, now());
    assert_eq!(created.next_run_at, now());
    assert!(created.scope.all_teams);
    let [only] = created.jobs.as_slice() else {
        panic!("a new lens carries one job");
    };
    assert_eq!(only.id, "id-2");
    assert_eq!(only.status, JobStatus::Queued);
    assert_eq!(only.start, minutes_after(now(), -1440));
    assert_eq!(only.end, minutes_after(now(), -2));
    assert_eq!(only.settings.name, "Nightly");
    assert_eq!(harness.database.lens("id-1").jobs.len(), 1);
}

#[tokio::test]
async fn create_lens_denies_a_non_admin() {
    let harness = Harness::new();
    let body = settings_json("gpt-4o");
    let reply = call(
        harness.app(),
        request("POST", "/lens", Some(&member()), Some(&body)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.detail(), WRITE_DENIED);
}

#[tokio::test]
async fn a_bad_body_is_422_before_the_role_check() {
    let harness = Harness::new();
    let reply = call(
        harness.app(),
        request("POST", "/lens", Some(&member()), Some("{}")),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let issues = reply.issues();
    assert_eq!(issues[0].loc[0], "body");
}

#[tokio::test]
async fn create_lens_rejects_an_unknown_model() {
    let harness = Harness::new();
    let body = settings_json("not-deployed");
    let reply = call(
        harness.app(),
        request("POST", "/lens", Some(&admin()), Some(&body)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.detail(), UNKNOWN_MODEL);
}

#[tokio::test]
async fn read_lens_returns_it() {
    let harness = Harness::new();
    harness.database.seed_lens(lens("l1"));
    let reply = call(
        harness.app(),
        request("GET", "/lens/l1", Some(&admin()), None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let read: Lens = reply.json();
    assert_eq!(read.id, "l1");
}

#[tokio::test]
async fn read_lens_missing_is_404() {
    let harness = Harness::new();
    let reply = call(
        harness.app(),
        request("GET", "/lens/missing", Some(&admin()), None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.detail(), "Lens not found");
}

#[tokio::test]
async fn update_lens_bumps_the_revision_and_keeps_the_jobs() {
    let harness = Harness::new();
    let mut seeded = lens("l1");
    seeded.jobs = vec![job("j1", JobStatus::Queued)];
    harness.database.seed_lens(seeded);
    let body = r#"{"name":"Updated","model":"gpt-4o","context":"Watch the checkout flow"}"#;
    let reply = call(
        harness.app(),
        request("PUT", "/lens/l1", Some(&admin()), Some(body)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let updated: Lens = reply.json();
    assert_eq!(updated.revision, 2);
    assert_eq!(updated.settings.name, "Updated");
    let [only] = updated.jobs.as_slice() else {
        panic!("the queued job stays");
    };
    assert_eq!(only.id, "j1");
    assert_eq!(only.settings.name, "Nightly");
    assert_eq!(harness.database.lens("l1").revision, 2);
}

#[tokio::test]
async fn update_lens_missing_is_404_before_the_model_check() {
    let harness = Harness::new();
    let body = settings_json("not-deployed");
    let reply = call(
        harness.app(),
        request("PUT", "/lens/missing", Some(&admin()), Some(&body)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.detail(), "Lens not found");
}

#[tokio::test]
async fn update_lens_denies_a_non_admin_writer() {
    let harness = Harness::new();
    harness.database.seed_lens(lens("l1"));
    let body = settings_json("gpt-4o");
    let reply = call(
        harness.app(),
        request("PUT", "/lens/l1", Some(&member()), Some(&body)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.detail(), WRITE_DENIED);
}

#[tokio::test]
async fn activity_available_without_tracing_is_false() {
    let harness = Harness::new().without_storage();
    let reply = call(
        harness.app(),
        request("GET", "/lens/activity/available", Some(&admin()), None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let available: ActivityAvailability = reply.json();
    assert!(!available.traces);
    assert!(!available.requests);
}

#[tokio::test]
async fn activity_available_reads_the_storage() {
    let mut harness = Harness::new();
    harness.storage = Some(
        FakeStorage::default().returning_availability(r#"[{"traces":true,"requests":false}]"#),
    );
    let reply = call(
        harness.app(),
        request("GET", "/lens/activity/available", Some(&admin()), None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let available: ActivityAvailability = reply.json();
    assert!(available.traces);
    assert!(!available.requests);
}

#[tokio::test]
async fn agents_without_tracing_is_empty() {
    let harness = Harness::new().without_storage();
    let reply = call(
        harness.app(),
        request("GET", "/lens/agents", Some(&admin()), None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let agents: Vec<String> = reply.json();
    assert!(agents.is_empty());
}

#[tokio::test]
async fn agents_with_tracing_reads_the_storage() {
    let mut harness = Harness::new();
    harness.storage = Some(
        FakeStorage::default()
            .returning_agents(r#"[{"agent_name":"helper"},{"agent_name":"writer"}]"#),
    );
    let reply = call(
        harness.app(),
        request("GET", "/lens/agents", Some(&admin()), None),
    )
    .await;
    let agents: Vec<String> = reply.json();
    assert_eq!(agents, ["helper", "writer"]);
}
