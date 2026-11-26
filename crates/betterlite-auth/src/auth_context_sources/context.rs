use std::collections::BTreeMap;

use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use betterlite_models::model_registration::JsonObject;

use crate::auth_request_models::RequestedModels;
use crate::auth_request_models::catalog::{CatalogError, Collaborators};
use crate::auth_request_models::model::get_model_from_request;
use crate::auth_request_models::sources::{DispatchContext, RequestSnapshot};

use super::AUTH_ERROR_TYPE;

// _safe_get_request_headers/_safe_get_request_query_params: an unreadable value is dropped, never an error.
#[derive(Clone, Copy, Debug)]
pub struct ModelContext<'request> {
    pub route: &'request str,
    pub body: &'request JsonObject,
    pub headers: Option<&'request JsonObject>,
    pub query: Option<&'request JsonObject>,
    pub dispatch: DispatchContext<'request>,
    pub team_id: Option<&'request str>,
}

// _get_model_from_request_context delegates every source to B746 with the safe header and query values.
pub fn get_model_from_request_context(
    context: &ModelContext<'_>,
    collaborators: &Collaborators<'_>,
) -> Result<RequestedModels, CatalogError> {
    let snapshot = RequestSnapshot {
        route: context.route,
        body: context.body,
        headers: context.headers,
        query: context.query,
        dispatch: DispatchContext {
            endpoint_marker: context.dispatch.endpoint_marker,
            path_params: context.dispatch.path_params,
        },
        team_id: context.team_id,
    };
    get_model_from_request(&snapshot, collaborators)
}

// A bare auth-time exception becomes auth_error with param "None" and status 401.
#[must_use]
pub fn context_failure(error: &CatalogError) -> ProxyException {
    ProxyException::new(ProxyExceptionInit {
        message: format!("Authentication Error, {error}"),
        error_type: AUTH_ERROR_TYPE.to_owned(),
        param: Some("None".to_owned()),
        code: ExceptionCode::Status(401),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}
