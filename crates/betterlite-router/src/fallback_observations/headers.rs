use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json;
use serde_json::Number;

use super::error_info::{
    FALLBACK_ERRORS_HEADER, FallbackErrorInfo, error_object, fallback_errors_from_headers,
};
use crate::router_core::headers::{
    Response, ensure_additional_headers, prepare_response, write_hidden_params,
};

// add_fallback_headers_to_response writes the attempted count, and the merged errors only when the caller passes some
pub const ATTEMPTED_FALLBACKS_HEADER: &str = "x-litellm-attempted-fallbacks";

#[must_use]
pub fn attach_fallback_headers(
    response: Response,
    attempted_fallbacks: u32,
    errors: Option<&[FallbackErrorInfo]>,
) -> Response {
    let Some(mut prepared) = prepare_response(response) else {
        return Response::Absent;
    };
    let headers = ensure_additional_headers(&mut prepared);
    headers.insert(
        ATTEMPTED_FALLBACKS_HEADER.to_owned(),
        Json::Number(Number::from(attempted_fallbacks)),
    );
    if let Some(new_errors) = errors {
        let merged = merged_errors(headers, new_errors);
        // Json always serializes; an impossible encoding failure must not fail the request
        let encoded = serde_json::to_string(&merged).unwrap_or_default();
        headers.insert(FALLBACK_ERRORS_HEADER.to_owned(), Json::Text(encoded));
    }
    write_hidden_params(prepared)
}

fn merged_errors(headers: &AdditionalHeaders, errors: &[FallbackErrorInfo]) -> Vec<Json> {
    let mut merged: Vec<Json> = fallback_errors_from_headers(headers)
        .into_iter()
        .map(Json::Object)
        .collect();
    merged.extend(errors.iter().map(|error| Json::Object(error_object(error))));
    merged
}
