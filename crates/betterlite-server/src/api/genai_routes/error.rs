use std::collections::BTreeMap;

use axum::Json;
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse as _, Response};
use betterlite_core::api::request_body::RequestBodyError;
use betterlite_error::envelope::error_body_call_id;
use betterlite_error::proxy_exception::ProxyException;
use betterlite_error::proxy_exception::body::ProxyErrorBody;
use serde::Serialize;

const CALL_ID_HEADER: &str = "x-litellm-call-id";

// openai_exception_handler: {"error": with_call_id(exc.to_dict(), ..)} at int(exc.code), with exc.headers copied over
pub(crate) fn failure_response(
    exception: &ProxyException,
    include_call_id_in_error_body: bool,
) -> Response {
    let call_id = exception.headers.get(CALL_ID_HEADER).map(String::as_str);
    let mut response = (
        status_code(&exception.code),
        Json(ErrorEnvelope {
            error: ErrorBody {
                body: exception.to_body(),
                litellm_call_id: error_body_call_id(include_call_id_in_error_body, call_id)
                    .map(str::to_owned),
            },
        }),
    )
        .into_response();
    insert_headers(response.headers_mut(), &exception.headers);
    response
}

// B1100's facts carry the wire type, param and code as well as the status; the code is not always the status
pub(crate) fn request_body_failure(error: &RequestBodyError) -> Response {
    let facts = error.facts();
    let body = ErrorBody {
        body: ProxyErrorBody {
            message: error.to_string(),
            error_type: facts.error_type.to_owned(),
            param: Some(facts.param.to_owned()),
            code: facts.code.to_owned(),
            provider_specific_fields: BTreeMap::new(),
        },
        litellm_call_id: None,
    };
    let status = StatusCode::from_u16(facts.status).unwrap_or(StatusCode::BAD_REQUEST);
    (status, Json(ErrorEnvelope { error: body })).into_response()
}

#[derive(Serialize)]
struct ErrorBody {
    #[serde(flatten)]
    body: ProxyErrorBody,
    #[serde(skip_serializing_if = "Option::is_none")]
    litellm_call_id: Option<String>,
}

#[derive(Serialize)]
struct ErrorEnvelope {
    error: ErrorBody,
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
