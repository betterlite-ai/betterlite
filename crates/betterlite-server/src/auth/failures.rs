use axum::body::Body;
use axum::http::StatusCode;
use axum::response::Response;

use crate::auth::auth_identity::StageFailure;
use crate::auth::auth_model_patterns::rejection_response;
use crate::auth::auth_resource_grants::denial_response;
use crate::error_body::authentication_error;

pub(crate) fn stage_failure_response(failure: &StageFailure) -> Response {
    match failure {
        StageFailure::Authentication(authentication) => authentication_error(authentication),
        StageFailure::ModelAccess(rejection) => rejection_response(rejection),
        StageFailure::ResourceGrants(exception)
        | StageFailure::TagBudgets(exception)
        | StageFailure::RequestBodySafety(exception) => denial_response(exception),
    }
}

// the body stream itself failed, so there is nothing left to forward; the buffer rejection is a bare 400
pub(crate) fn unreadable_body_response() -> Response {
    let mut response = Response::new(Body::from("Failed to buffer the request body"));
    *response.status_mut() = StatusCode::BAD_REQUEST;
    response
}
