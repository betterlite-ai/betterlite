#![cfg(test)]

use std::sync::Arc;

use axum::http::StatusCode;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_lens::models::job::{JobStatus, Worker};
use sha2::{Digest as _, Sha256};

use crate::lens_access::tests::support::{
    Answer, Harness, Resolver, TOKEN_BYTES, admin, call, job, lens, request, worker,
};

const RETIRED: &str = "Register a new worker instead of updating revoked access";
const RUNNING: &str =
    "Wait for this worker's investigation to finish or cancel it before revoking access";
const INACTIVE: &str = "Choose an active virtual key for Lens analysis";

#[derive(serde::Deserialize)]
struct Registered {
    worker: Worker,
    token: String,
}

fn key_id() -> String {
    "a".repeat(64)
}

fn other_key() -> String {
    "b".repeat(64)
}

#[tokio::test]
async fn register_returns_the_token_once_and_stores_its_digest() {
    let harness = Harness::new();
    let body = format!(r#"{{"analysis_key_id":"{}"}}"#, key_id());
    let reply = call(
        harness.app(),
        request(
            "POST",
            "/lens/workers/register",
            Some(&admin()),
            Some(&body),
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let registered: Registered = reply.json();
    let expected = format!("lens-{}", URL_SAFE_NO_PAD.encode(vec![TOKEN_BYTES; 40]));
    assert_eq!(registered.token, expected);
    assert_eq!(registered.worker.id, "id-1");
    assert_eq!(registered.worker.name, "Lens worker");
    assert_eq!(
        registered.worker.analysis_key_id.as_deref(),
        Some(key_id().as_str())
    );
    assert_eq!(
        registered.worker.last_seen.to_rfc3339(),
        "1970-01-01T00:00:00+00:00"
    );
    assert!(!registered.worker.revoked);
    assert!(registered.worker.scope.all_teams);
    let digest = hex::encode(Sha256::digest(registered.token.as_bytes()));
    assert_eq!(harness.database.stored_token(), Some(digest));
    assert_eq!(harness.resolver.seen(), [key_id()]);
}

#[tokio::test]
async fn register_counts_the_name_in_characters() {
    let harness = Harness::new();
    let name = "\u{1f600}".repeat(100);
    let body = format!(r#"{{"analysis_key_id":"{}","name":"{name}"}}"#, key_id());
    let reply = call(
        harness.app(),
        request(
            "POST",
            "/lens/workers/register",
            Some(&admin()),
            Some(&body),
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let registered: Registered = reply.json();
    assert_eq!(registered.worker.name, name);
}

#[tokio::test]
async fn register_rejects_a_name_past_one_hundred_characters() {
    let harness = Harness::new();
    let name = "n".repeat(101);
    let body = format!(r#"{{"analysis_key_id":"{}","name":"{name}"}}"#, key_id());
    let reply = call(
        harness.app(),
        request(
            "POST",
            "/lens/workers/register",
            Some(&admin()),
            Some(&body),
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn register_rejects_an_empty_name() {
    let harness = Harness::new();
    let body = format!(r#"{{"analysis_key_id":"{}","name":""}}"#, key_id());
    let reply = call(
        harness.app(),
        request(
            "POST",
            "/lens/workers/register",
            Some(&admin()),
            Some(&body),
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn register_rejects_an_unknown_field() {
    let harness = Harness::new();
    let body = format!(r#"{{"analysis_key_id":"{}","extra":1}}"#, key_id());
    let reply = call(
        harness.app(),
        request(
            "POST",
            "/lens/workers/register",
            Some(&admin()),
            Some(&body),
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn register_rejects_a_key_that_is_not_sixty_four_hex_characters() {
    let harness = Harness::new();
    let body = r#"{"analysis_key_id":"not-a-key"}"#;
    let reply = call(
        harness.app(),
        request("POST", "/lens/workers/register", Some(&admin()), Some(body)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn register_rejects_a_blocked_key() {
    let mut harness = Harness::new();
    harness.resolver = Arc::new(Resolver::new().answering(Answer::Blocked));
    let body = format!(r#"{{"analysis_key_id":"{}"}}"#, key_id());
    let reply = call(
        harness.app(),
        request(
            "POST",
            "/lens/workers/register",
            Some(&admin()),
            Some(&body),
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.detail(), INACTIVE);
}

#[tokio::test]
async fn register_rejects_a_session_token() {
    let mut harness = Harness::new();
    harness.resolver = Arc::new(Resolver::new().answering(Answer::Session));
    let body = format!(r#"{{"analysis_key_id":"{}"}}"#, key_id());
    let reply = call(
        harness.app(),
        request(
            "POST",
            "/lens/workers/register",
            Some(&admin()),
            Some(&body),
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn register_keeps_the_resolvers_401_for_a_missing_key() {
    let mut harness = Harness::new();
    harness.resolver = Arc::new(Resolver::new().answering(Answer::Missing));
    let body = format!(r#"{{"analysis_key_id":"{}"}}"#, key_id());
    let reply = call(
        harness.app(),
        request(
            "POST",
            "/lens/workers/register",
            Some(&admin()),
            Some(&body),
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn billing_updates_the_worker() {
    let harness = Harness::new();
    harness.database.seed_worker(worker("w1"));
    let body = format!(r#"{{"analysis_key_id":"{}"}}"#, other_key());
    let reply = call(
        harness.app(),
        request(
            "PUT",
            "/lens/workers/w1/billing-key",
            Some(&admin()),
            Some(&body),
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let updated: Worker = reply.json();
    assert_eq!(updated.id, "w1");
    assert_eq!(
        updated.analysis_key_id.as_deref(),
        Some(other_key().as_str())
    );
    assert_eq!(
        harness.database.worker("w1").analysis_key_id.as_deref(),
        Some(other_key().as_str())
    );
}

#[tokio::test]
async fn billing_updates_only_the_named_worker() {
    let harness = Harness::new();
    harness.database.seed_worker(worker("w1"));
    harness.database.seed_worker(worker("w2"));
    let body = format!(r#"{{"analysis_key_id":"{}"}}"#, other_key());
    let reply = call(
        harness.app(),
        request(
            "PUT",
            "/lens/workers/w2/billing-key",
            Some(&admin()),
            Some(&body),
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        harness.database.worker("w2").analysis_key_id.as_deref(),
        Some(other_key().as_str())
    );
    assert!(harness.database.worker("w1").analysis_key_id.is_none());
}

#[tokio::test]
async fn billing_for_a_missing_worker_is_404() {
    let harness = Harness::new();
    let body = format!(r#"{{"analysis_key_id":"{}"}}"#, other_key());
    let reply = call(
        harness.app(),
        request(
            "PUT",
            "/lens/workers/missing/billing-key",
            Some(&admin()),
            Some(&body),
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.detail(), "Worker not found");
}

#[tokio::test]
async fn billing_for_a_revoked_worker_is_409() {
    let harness = Harness::new();
    let mut retired = worker("w1");
    retired.revoked = true;
    harness.database.seed_worker(retired);
    let body = format!(r#"{{"analysis_key_id":"{}"}}"#, other_key());
    let reply = call(
        harness.app(),
        request(
            "PUT",
            "/lens/workers/w1/billing-key",
            Some(&admin()),
            Some(&body),
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.detail(), RETIRED);
}

#[tokio::test]
async fn revoke_refuses_while_a_job_runs() {
    let harness = Harness::new();
    harness.database.seed_worker(worker("w1"));
    let mut running = job("j1", JobStatus::Running);
    running.worker_id = Some("w1".to_owned());
    let mut stored = lens("l1");
    stored.jobs = vec![running];
    harness.database.seed_lens(stored);
    let reply = call(
        harness.app(),
        request("DELETE", "/lens/workers/w1", Some(&admin()), None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.detail(), RUNNING);
    assert!(!harness.database.worker("w1").revoked);
}

#[tokio::test]
async fn a_queued_job_does_not_block_a_revoke() {
    let harness = Harness::new();
    harness.database.seed_worker(worker("w1"));
    let mut finished = job("j2", JobStatus::Completed);
    finished.worker_id = Some("w1".to_owned());
    let mut elsewhere = job("j3", JobStatus::Running);
    elsewhere.worker_id = Some("w2".to_owned());
    let mut stored = lens("l1");
    stored.jobs = vec![job("j1", JobStatus::Queued), finished, elsewhere];
    harness.database.seed_lens(stored);
    let reply = call(
        harness.app(),
        request("DELETE", "/lens/workers/w1", Some(&admin()), None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let revoked: bool = reply.json();
    assert!(revoked);
    assert!(harness.database.worker("w1").revoked);
}

#[tokio::test]
async fn revoking_a_missing_worker_is_404() {
    let harness = Harness::new();
    harness.database.seed_worker(worker("w1"));
    let reply = call(
        harness.app(),
        request("DELETE", "/lens/workers/missing", Some(&admin()), None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.detail(), "Worker not found");
}
