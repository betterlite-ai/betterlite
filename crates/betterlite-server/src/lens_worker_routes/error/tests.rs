#![cfg(test)]

use axum::body::to_bytes;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse as _;
use betterlite_lens::models::messages::Progress;
use betterlite_lens::sources::source_reader::SourceError;

use crate::lens_access::error::LensAccessError;
use crate::lens_access::tests::support::TestIssue;
use crate::lens_admin_routes::body::decode;
use crate::lens_inference::analyze::AnalyzeError;
use crate::lens_worker_routes::error::{LensWorkerError, query_refusal};
use crate::lens_worker_routes::tests::support::TestError;

const UNAUTHENTICATED: &str = "Not authenticated";
const INVALID: &str = "Worker credential is invalid or revoked";
const NO_DATABASE: &str = "Lens needs a connected Postgres database";

type WorkerError = LensWorkerError<TestError>;

async fn answered(error: WorkerError) -> (StatusCode, HeaderMap, Vec<u8>) {
    let response = error.into_response();
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the error body reads");
    (status, headers, body.to_vec())
}

fn scheme(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::WWW_AUTHENTICATE)
        .map(|value| value.to_str().expect("an ascii header").to_owned())
}

#[tokio::test]
async fn the_missing_bearer_refusal_carries_the_scheme() {
    let (status, headers, body) = answered(WorkerError::NotAuthenticated).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(scheme(&headers), Some("Bearer".to_owned()));
    let text: DetailText = serde_json::from_slice(&body).expect("the error body decodes");
    assert_eq!(text.detail, UNAUTHENTICATED);
}

#[tokio::test]
async fn an_invalid_credential_has_no_scheme_header() {
    let (status, headers, body) = answered(WorkerError::InvalidCredential).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(scheme(&headers), None);
    let text: DetailText = serde_json::from_slice(&body).expect("the error body decodes");
    assert_eq!(text.detail, INVALID);
}

#[test]
fn the_unit_refusals_keep_their_statuses() {
    assert_eq!(WorkerError::NotOwned.status(), StatusCode::CONFLICT);
    assert_eq!(WorkerError::JobMoved.status(), StatusCode::CONFLICT);
    assert_eq!(WorkerError::SampleNotFrozen.status(), StatusCode::CONFLICT);
    assert_eq!(WorkerError::UpgradeWorker.status(), StatusCode::CONFLICT);
    assert_eq!(WorkerError::NoAnalysisKey.status(), StatusCode::CONFLICT);
    assert_eq!(WorkerError::OutsideSample.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        WorkerError::DuplicateRun.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        WorkerError::EvidenceMismatch.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        WorkerError::MissingRun.status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
}

#[test]
fn an_access_error_keeps_its_own_status_and_text() {
    let refused = WorkerError::from(LensAccessError::NoDatabase);
    assert_eq!(refused.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(refused.detail(), NO_DATABASE);
    let disabled = WorkerError::from(LensAccessError::TracingDisabled);
    assert_eq!(disabled.status(), StatusCode::NOT_IMPLEMENTED);
}

#[test]
fn an_analyze_error_keeps_its_own_status_and_text() {
    let refused = WorkerError::from(AnalyzeError::NoModels);
    assert_eq!(refused.status(), StatusCode::SERVICE_UNAVAILABLE);
    let no_key = WorkerError::from(AnalyzeError::NoAnalysisKey);
    assert_eq!(no_key.status(), StatusCode::CONFLICT);
    assert_eq!(
        no_key.detail(),
        "Assign an analysis key to this worker in Lens setup"
    );
    let no_deployments = WorkerError::from(AnalyzeError::NoDeployments);
    assert_eq!(no_deployments.status(), StatusCode::BAD_REQUEST);
    let no_choices = WorkerError::from(AnalyzeError::NoChoices);
    assert_eq!(no_choices.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[test]
fn a_source_error_names_the_whole_chain() {
    let refused = WorkerError::Source(SourceError::Query(TestError));
    assert_eq!(refused.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        refused.detail(),
        "lens storage query failed: the fake storage failed"
    );
}

#[tokio::test]
async fn a_query_refusal_is_the_fastapi_issue_shape() {
    let error = query_refusal::<TestError>("missing", "execution_id", "Field required");
    let (status, _, body) = answered(error).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let detail: Details = serde_json::from_slice(&body).expect("the issue body decodes");
    assert_eq!(detail.detail[0].kind, "missing");
    assert_eq!(detail.detail[0].loc, ["query", "execution_id"]);
}

#[tokio::test]
async fn a_body_failure_is_the_shared_422() {
    let failure = decode::<Progress>(b"{}").expect_err("the body misses its stage");
    let (status, _, body) = answered(WorkerError::from(failure)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let detail: Details = serde_json::from_slice(&body).expect("the issue body decodes");
    assert_eq!(detail.detail[0].kind, "missing");
    assert_eq!(detail.detail[0].loc, ["body", "stage"]);
}

#[derive(serde::Deserialize)]
struct DetailText {
    detail: String,
}

#[derive(serde::Deserialize)]
struct Details {
    detail: Vec<TestIssue>,
}
