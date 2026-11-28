#![cfg(test)]

use axum::http::{StatusCode, header};

use crate::lens_access::tests::support::request;
use crate::lens_worker_routes::tests::support::{
    Harness, OTHER_TOKEN, TOKEN, TOKEN_DIGEST, with_header, with_token, worker_call, worker_fixture,
};

const UNAUTHENTICATED: &str = "Not authenticated";
const INVALID: &str = "Worker credential is invalid or revoked";
const NO_DATABASE: &str = "Lens needs a connected Postgres database";

fn claim_uri() -> &'static str {
    "/lens/worker/claim?protocol_version=2"
}

#[tokio::test]
async fn a_missing_header_is_not_authenticated() {
    let harness = Harness::new();
    let reply = worker_call(harness.app(), request("POST", claim_uri(), None, None)).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert_eq!(reply.detail(), UNAUTHENTICATED);
    assert_eq!(reply.header("www-authenticate").as_deref(), Some("Bearer"));
}

#[tokio::test]
async fn a_basic_scheme_is_not_authenticated() {
    let harness = Harness::new();
    let built = with_header(
        request("POST", claim_uri(), None, None),
        header::AUTHORIZATION,
        "Basic worker-token",
    );
    let reply = worker_call(harness.app(), built).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert_eq!(reply.detail(), UNAUTHENTICATED);
}

#[tokio::test]
async fn an_empty_scheme_is_not_authenticated() {
    let harness = Harness::new();
    let built = with_header(
        request("POST", claim_uri(), None, None),
        header::AUTHORIZATION,
        " worker-token",
    );
    let reply = worker_call(harness.app(), built).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert_eq!(reply.detail(), UNAUTHENTICATED);
}

#[tokio::test]
async fn empty_credentials_are_not_authenticated() {
    let harness = Harness::new();
    let built = with_header(
        request("POST", claim_uri(), None, None),
        header::AUTHORIZATION,
        "Bearer   ",
    );
    let reply = worker_call(harness.app(), built).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert_eq!(reply.detail(), UNAUTHENTICATED);
}

#[tokio::test]
async fn a_bare_bearer_scheme_is_not_authenticated() {
    let harness = Harness::new();
    let built = with_header(
        request("POST", claim_uri(), None, None),
        header::AUTHORIZATION,
        "Bearer",
    );
    let reply = worker_call(harness.app(), built).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert_eq!(reply.detail(), UNAUTHENTICATED);
}

#[tokio::test]
async fn the_scheme_comparison_ignores_case() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    let built = with_header(
        request("POST", claim_uri(), None, None),
        header::AUTHORIZATION,
        "bEaReR worker-token",
    );
    let reply = worker_call(harness.app(), built).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, b"null");
}

#[tokio::test]
async fn whitespace_around_the_token_is_trimmed() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    let built = with_header(
        request("POST", claim_uri(), None, None),
        header::AUTHORIZATION,
        "Bearer    worker-token   ",
    );
    let reply = worker_call(harness.app(), built).await;
    assert_eq!(reply.status, StatusCode::OK);
}

#[tokio::test]
async fn an_unknown_token_is_an_invalid_credential() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    let built = with_token("POST", claim_uri(), None, OTHER_TOKEN);
    let reply = worker_call(harness.app(), built).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert_eq!(reply.detail(), INVALID);
    assert_eq!(reply.header("www-authenticate"), None);
}

#[tokio::test]
async fn a_revoked_worker_is_an_invalid_credential() {
    let harness = Harness::new();
    let mut retired = worker_fixture();
    retired.revoked = true;
    harness.database.seed_worker(retired);
    let reply = worker_call(harness.app(), with_token("POST", claim_uri(), None, TOKEN)).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert_eq!(reply.detail(), INVALID);
}

#[tokio::test]
async fn the_lookup_hashes_the_token_and_never_compares_it_raw() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    let reply = worker_call(harness.app(), with_token("POST", claim_uri(), None, TOKEN)).await;
    assert_eq!(reply.status, StatusCode::OK);
    let digest_header = with_token("POST", claim_uri(), None, TOKEN_DIGEST);
    let refused = worker_call(harness.app(), digest_header).await;
    assert_eq!(refused.status, StatusCode::UNAUTHORIZED);
    assert_eq!(refused.detail(), INVALID);
}

#[tokio::test]
async fn a_valid_bearer_without_a_database_is_503() {
    let harness = Harness::new().without_database();
    let reply = worker_call(harness.app(), with_token("POST", claim_uri(), None, TOKEN)).await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.detail(), NO_DATABASE);
}

#[tokio::test]
async fn a_missing_header_without_a_database_is_still_401() {
    let harness = Harness::new().without_database();
    let reply = worker_call(harness.app(), request("POST", claim_uri(), None, None)).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert_eq!(reply.detail(), UNAUTHENTICATED);
}
