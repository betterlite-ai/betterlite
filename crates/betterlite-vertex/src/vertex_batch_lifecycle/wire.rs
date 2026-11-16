use std::time::Duration;

use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_models::decisions::Json;
use http::Method;
use http::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use url::Url;

use crate::endpoint_target::region::DEFAULT_LOCATION;
use crate::token_cache::probe::AccessToken;

use super::error::VertexBatchLifecycleError;
use super::execution::BatchLifecycleRequest;

const CONTENT_TYPE_VALUE: &str = "application/json; charset=utf-8";

// LiteLLM sends `vertex_project or project_id` and `vertex_location or "us-central1"`
pub(super) fn project_and_location(
    request: &BatchLifecycleRequest<'_>,
    access: &AccessToken,
) -> (String, String) {
    let project = request
        .project
        .filter(|project| !project.is_empty())
        .unwrap_or(&access.project)
        .to_owned();
    let location = request
        .location
        .filter(|location| !location.is_empty())
        .unwrap_or(DEFAULT_LOCATION)
        .to_owned();
    (project, location)
}

pub(super) fn bearer_headers(access: &AccessToken) -> Result<HeaderMap, VertexBatchLifecycleError> {
    let mut authorization =
        HeaderValue::from_str(&format!("Bearer {}", access.token.expose_secret()))
            .map_err(|source| VertexBatchLifecycleError::Header { source })?;
    authorization.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(CONTENT_TYPE_VALUE));
    headers.insert(AUTHORIZATION, authorization);
    Ok(headers)
}

pub(super) fn get_request(
    target: &str,
    headers: HeaderMap,
    query: Vec<(String, String)>,
    deadline: Duration,
) -> Result<OutboundRequest, VertexBatchLifecycleError> {
    let url = Url::parse(target).map_err(|source| VertexBatchLifecycleError::Url { source })?;
    Ok(OutboundRequest {
        method: Method::GET,
        url,
        query,
        headers,
        body: RequestBody::Empty,
        deadline,
    })
}

// LiteLLM inserts limit_params before after_params, so pageSize precedes pageToken
pub(super) fn list_query(after: Option<&str>, limit: Option<i64>) -> Vec<(String, String)> {
    let mut query = Vec::with_capacity(2);
    if let Some(limit) = limit {
        query.push(("pageSize".to_owned(), limit.to_string()));
    }
    if let Some(after) = after {
        query.push(("pageToken".to_owned(), after.to_owned()));
    }
    query
}

pub(super) fn pre_call_args(target: &str, access: &AccessToken) -> AdditionalArgs {
    let mut headers = IndexMap::new();
    headers.insert(
        "Content-Type".to_owned(),
        Json::Text(CONTENT_TYPE_VALUE.to_owned()),
    );
    headers.insert(
        "Authorization".to_owned(),
        Json::Text(format!("Bearer {}", access.token.expose_secret())),
    );
    AdditionalArgs {
        api_base: Some(target.to_owned()),
        headers: Some(headers),
        complete_input_dict: Some(RawBody::default()),
        request_str: Some(format!(
            "\nGET Request Sent from LiteLLM:\ncurl -X GET \\\n{target} \\\n-H 'Authorization: Bearer ***REDACTED***' \\\n-H 'Content-Type: application/json; charset=utf-8'\n"
        )),
        openai_sdk: None,
        passthrough: IndexMap::new(),
    }
}
