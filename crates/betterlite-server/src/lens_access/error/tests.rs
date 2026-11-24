#![cfg(test)]

use axum::http::StatusCode;
use betterlite_auth::resolved_key::auth::AuthError;
use betterlite_db::lens_repository::repository::LensRepositoryError;

use crate::lens_access::error::{LensAccessError, failure_text};

#[test]
fn refusals_keep_litellms_statuses() {
    assert_eq!(LensAccessError::WriteDenied.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        LensAccessError::AccessDenied.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(LensAccessError::ModelDenied.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        LensAccessError::NoDatabase.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        LensAccessError::LensNotFound.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        LensAccessError::InvestigationNotFound.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        LensAccessError::WorkerNotFound.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        LensAccessError::ExecutionNotFound.status(),
        StatusCode::NOT_FOUND
    );
}

#[test]
fn worker_refusals_keep_litellms_statuses() {
    assert_eq!(
        LensAccessError::ConcurrentChange.status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        LensAccessError::WorkerRevoked.status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        LensAccessError::WorkerAccessRevoked.status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        LensAccessError::WorkerRunning.status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        LensAccessError::TracingDisabled.status(),
        StatusCode::NOT_IMPLEMENTED
    );
    assert_eq!(
        LensAccessError::InvalidSelection.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        LensAccessError::UnknownModel.status(),
        StatusCode::BAD_REQUEST
    );
}

#[test]
fn every_detail_is_the_litellm_text() {
    assert_eq!(
        failure_text(&LensAccessError::NoDatabase),
        "Lens needs a connected Postgres database"
    );
    assert_eq!(
        failure_text(&LensAccessError::TracingDisabled),
        "Agent tracing is not enabled. Set `tracing:` in general_settings and CLICKHOUSE_URL."
    );
    assert_eq!(
        failure_text(&LensAccessError::InvalidSelection),
        "Choose execution IDs returned by the activity preview"
    );
    assert_eq!(
        failure_text(&LensAccessError::UnknownModel),
        "Choose a model configured on this LiteLLM instance"
    );
    assert_eq!(
        failure_text(&LensAccessError::ModelDenied),
        "This key does not have access to the analysis model"
    );
    assert_eq!(
        failure_text(&LensAccessError::ConcurrentChange),
        "Lens changed concurrently; retry the operation"
    );
    assert_eq!(
        failure_text(&LensAccessError::WorkerRevoked),
        "Register a new worker instead of updating revoked access"
    );
    assert_eq!(
        failure_text(&LensAccessError::WorkerAccessRevoked),
        "Worker access was revoked"
    );
    assert_eq!(
        failure_text(&LensAccessError::WorkerRunning),
        "Wait for this worker's investigation to finish or cancel it before revoking access"
    );
}

#[test]
fn a_repository_failure_is_a_500_that_keeps_its_text() {
    let error = LensAccessError::Repository(LensRepositoryError::Version);
    assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(failure_text(&error), "the lens version would overflow");
}

#[test]
fn a_chained_repository_failure_names_its_cause() {
    let source = serde_json::from_str::<u8>("not json").expect_err("the fixture is not json");
    let error = LensAccessError::Repository(LensRepositoryError::Encode(source));
    assert!(failure_text(&error).starts_with("the record could not be encoded as JSON: "));
}

#[test]
fn an_unbuilt_caller_names_its_cause() {
    let error = LensAccessError::CallerKey(AuthError::NonStringApiKey);
    assert!(failure_text(&error).starts_with("the caller key could not be rebuilt: "));
}

#[test]
fn an_unreadable_caller_names_its_cause() {
    let source = serde_json::from_str::<u8>("not json").expect_err("the fixture is not json");
    let error = LensAccessError::Caller(source);
    assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(failure_text(&error).starts_with("the caller key could not be read: "));
}
