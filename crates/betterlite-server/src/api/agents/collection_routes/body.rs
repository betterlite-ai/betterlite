use std::collections::BTreeMap;

use axum::Json;
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use betterlite_config::sections::request_body::Settings;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::request_body::{RequestBodyError, RequestBodyInput};
use betterlite_error::envelope::{ErrorObject, ErrorResponse, WireField};
use betterlite_openai::openai_base_config::Object as JsonMap;

use crate::api::request_body::read_request_body;

// _read_request_body picks its branch from the lowercased content-type header
fn headers(headers: &HeaderMap) -> BTreeMap<String, String> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|text| (name.as_str().to_ascii_lowercase(), text.to_owned()))
        })
        .collect()
}

pub(super) fn read_json_body(
    parts: &Parts,
    query_params: &BTreeMap<String, String>,
    bytes: &[u8],
    settings: &Settings,
    limits: &ApiLimits,
) -> Result<JsonMap, RequestBodyError> {
    let input = RequestBodyInput {
        method: parts.method.as_str().to_owned(),
        route_path: parts.uri.path().to_owned(),
        headers: headers(&parts.headers),
        query_params: query_params.clone(),
        body: bytes.to_vec(),
        client_requested_model: None,
        cached_body: None,
        form_fields: None,
    };
    let result = read_request_body(&input, settings, limits)?;
    Ok(result.body.into_iter().collect())
}

// a body ProxyException carries message, type, param and code, and its status comes from the same facts
pub(super) fn failure(error: &RequestBodyError) -> Response {
    let facts = error.facts();
    let body = ErrorResponse {
        error: ErrorObject {
            message: error.to_string(),
            error_type: WireField::Text(facts.error_type.to_owned()),
            param: WireField::Text(facts.param.to_owned()),
            code: WireField::Text(facts.code.to_owned()),
        },
    };
    let status = StatusCode::from_u16(facts.status).unwrap_or(StatusCode::BAD_REQUEST);
    (status, Json(body)).into_response()
}
