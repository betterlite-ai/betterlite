use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::request_body::DecodedJson;
use betterlite_oracle_helper_api::error::OrHelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_server::api::request_form::{RequestFormBody, RequestFormError, get_request_body};
use serde_json::json;

use super::projection::form_fields_value;
use super::{Input, failure};

pub(super) fn body_dispatch(input: &Input) -> HelperResult {
    match get_request_body(
        &input.method,
        &input.content_type,
        input.body.as_bytes(),
        &ApiLimits::default(),
    ) {
        Ok(RequestFormBody::Empty) => Ok(json!({})),
        Ok(RequestFormBody::Json(DecodedJson::Parsed { body, .. })) => {
            serde_json::to_value(body).or_value_error()
        }
        Ok(RequestFormBody::Json(DecodedJson::Unreadable(_error))) => Ok(json!({})),
        Ok(RequestFormBody::Form(fields)) => form_fields_value(&fields),
        Err(RequestFormError::UnsupportedContentType(content_type)) => Err(failure(
            "ValueError",
            format!("Unsupported content type: {content_type}"),
        )),
        Err(error) => Err(failure("ValueError", error.to_string())),
    }
}
