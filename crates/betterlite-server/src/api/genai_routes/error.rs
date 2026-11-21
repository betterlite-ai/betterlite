use std::collections::BTreeMap;

use axum::Json;
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use betterlite_core::api::request_body::RequestBodyError;
use betterlite_error::proxy_exception::ProxyException;
use betterlite_error::proxy_exception::body::ProxyErrorBody;
use serde::Serialize;

// openai_exception_handler: {"error": exc.to_dict()} at int(exc.code), with exc.headers copied over
pub(crate) fn failure_response(exception: &ProxyException) -> Response {
    let mut response = (
        status_code(&exception.code),
        Json(ErrorEnvelope {
            error: exception.to_body(),
        }),
    )
        .into_response();
    insert_headers(response.headers_mut(), &exception.headers);
    response
}

// B1100's facts carry the wire type, param and code as well as the status; the code is not always the status
pub(crate) fn request_body_failure(error: &RequestBodyError) -> Response {
    let facts = error.facts();
    let body = ProxyErrorBody {
        message: error.to_string(),
        error_type: facts.error_type.to_owned(),
        param: Some(facts.param.to_owned()),
        code: facts.code.to_owned(),
        provider_specific_fields: BTreeMap::new(),
    };
    let status = StatusCode::from_u16(facts.status).unwrap_or(StatusCode::BAD_REQUEST);
    (status, Json(ErrorEnvelope { error: body })).into_response()
}

#[derive(Serialize)]
struct ErrorEnvelope {
    error: ProxyErrorBody,
}

// a code that is not an HTTP status has no response to name, so it answers the generic 500
fn status_code(code: &str) -> StatusCode {
    code.parse::<u16>()
        .ok()
        .and_then(|status| StatusCode::from_u16(status).ok())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

fn insert_headers(target: &mut HeaderMap, source: &BTreeMap<String, String>) {
    for (name, text) in source {
        let Ok(header_name) = HeaderName::try_from(name.as_str()) else {
            tracing::warn!(header = %name, "response header name cannot be forwarded");
            continue;
        };
        let Ok(header_value) = HeaderValue::from_str(text) else {
            tracing::warn!(header = %name, "response header value cannot be forwarded");
            continue;
        };
        target.insert(header_name, header_value);
    }
}
