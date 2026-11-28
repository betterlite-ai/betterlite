use axum::Json;
use axum::body::Body;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use betterlite_core::api::api_boundaries::ApiLimitError;
use betterlite_error::envelope::{ErrorObject, ErrorResponse, WireField};
use betterlite_error::proxy_exception::ProxyException;

use crate::auth::auth_model_patterns::{ModelAccessRejection, rejection_response};
use crate::auth::auth_resource_grants::denial_response;
use crate::error_body::{AuthenticationFailure, authentication_error};

pub(crate) enum StageFailure {
    JwtKeys(Box<ProxyException>),
    Authentication(Box<AuthenticationFailure>),
    KeyLookup(Box<ProxyException>),
    ModelAccess(Box<ModelAccessRejection>),
    AuthModelGrants(Box<ProxyException>),
    EntityModelAccess(Box<ProxyException>),
    TrustedNetwork(Box<ProxyException>),
    AuthRequestPolicy(Box<ProxyException>),
    ResourceGrants(Box<ProxyException>),
    TagBudgets(Box<ProxyException>),
    RequestBodySafety(Box<ProxyException>),
    VirtualKeyRoutes(Box<ProxyException>),
    KeyBudgets(Box<ProxyException>),
    ModelBudgets(Box<ProxyException>),
    TeamBudgets(Box<ProxyException>),
    TenantBudgets(Box<ProxyException>),
    OrganizationRoutePolicy(Box<ProxyException>),
    AuthRoutePolicy(Box<ProxyException>),
    OrgLookup(Box<ProxyException>),
    EndUsers(Box<ProxyException>),
    AuthPolicySupport(Box<ProxyException>),
    ModelGrants(Box<ProxyException>),
    OauthTokens(Box<ProxyException>),
}

impl From<AuthenticationFailure> for StageFailure {
    fn from(failure: AuthenticationFailure) -> Self {
        Self::Authentication(Box::new(failure))
    }
}

impl From<ModelAccessRejection> for StageFailure {
    fn from(rejection: ModelAccessRejection) -> Self {
        Self::ModelAccess(Box::new(rejection))
    }
}

pub(crate) fn stage_failure_response(failure: &StageFailure) -> Response {
    match failure {
        StageFailure::Authentication(authentication) => authentication_error(authentication),
        StageFailure::ModelAccess(rejection) => rejection_response(rejection),
        StageFailure::KeyLookup(exception)
        | StageFailure::AuthModelGrants(exception)
        | StageFailure::JwtKeys(exception)
        | StageFailure::EntityModelAccess(exception)
        | StageFailure::AuthRequestPolicy(exception)
        | StageFailure::TrustedNetwork(exception)
        | StageFailure::ResourceGrants(exception)
        | StageFailure::TagBudgets(exception)
        | StageFailure::RequestBodySafety(exception)
        | StageFailure::VirtualKeyRoutes(exception)
        | StageFailure::KeyBudgets(exception)
        | StageFailure::ModelBudgets(exception)
        | StageFailure::TeamBudgets(exception)
        | StageFailure::TenantBudgets(exception)
        | StageFailure::OrganizationRoutePolicy(exception)
        | StageFailure::AuthRoutePolicy(exception)
        | StageFailure::OrgLookup(exception)
        | StageFailure::EndUsers(exception)
        | StageFailure::AuthPolicySupport(exception)
        | StageFailure::ModelGrants(exception)
        | StageFailure::OauthTokens(exception) => denial_response(exception),
    }
}

// the body stream itself failed, so there is nothing left to forward; the buffer rejection is a bare 400
pub(crate) fn unreadable_body_response() -> Response {
    let mut response = Response::new(Body::from("Failed to buffer the request body"));
    *response.status_mut() = StatusCode::BAD_REQUEST;
    response
}

// the same facts a route's JSON body limit answers: 413 invalid_request_error, param body, code request_too_large
pub(crate) fn body_too_large_response(observed: usize, limit: usize) -> Response {
    let error = ApiLimitError::JsonTooLarge { observed, limit };
    let fields = error.fields();
    let body = ErrorResponse {
        error: ErrorObject {
            message: error.to_string(),
            error_type: WireField::Text(fields.error_type.to_owned()),
            param: WireField::Text(fields.param.to_owned()),
            code: WireField::Text(fields.code.to_owned()),
        },
    };
    let status = StatusCode::from_u16(fields.status).unwrap_or(StatusCode::PAYLOAD_TOO_LARGE);
    (status, Json(body)).into_response()
}
