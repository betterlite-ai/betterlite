use betterlite_models::call_helpers::expected_error::{ClientErrorFacts, is_expected_client_error};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use crate::fields::payload::records::{AdditionalHeaders, ErrorInformation};
use crate::logging_metadata::error_info::{
    ExceptionLogging, PayloadErrorRequest, payload_error_information,
};
use crate::logging_metadata::exception::ExceptionFacts;
use crate::logging_metadata::headers::get_additional_headers;
use crate::logging_metadata::params::MetadataMap;
use crate::payload::compose::{PayloadComposition, PayloadRequest};
use crate::payload::error::PayloadError;

// _get_response_headers: exception.headers, then response.headers, then the litellm response headers
pub(crate) fn exception_headers(exception: Option<&ExceptionFacts>) -> Option<AdditionalHeaders> {
    let facts = exception?;
    let candidates = facts
        .headers
        .as_ref()
        .or(facts.response_headers.as_ref())
        .or(facts.litellm_response_headers.as_ref());
    let headers = candidates?;
    let record: IndexMap<String, Json> = headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect();
    get_additional_headers(Some(&record))
}

// The proxy's own 4xx is an expected client error; anything the provider originated is not.
pub(crate) fn expected_client_error(exception: Option<&ExceptionFacts>) -> bool {
    let facts = ClientErrorFacts {
        code: exception
            .and_then(|facts| facts.code.clone())
            .map(Json::Text),
        status_code: exception
            .and_then(|facts| facts.status_code)
            .map(|status| Json::Number(Number::from(status))),
        llm_provider: exception.and_then(|facts| facts.llm_provider.clone()),
        category: exception.and_then(|facts| facts.category.clone()),
        // ExceptionFacts carries no HTTPException or BaseLLMException marker; see docs/deviations/B190.yaml
        http_exception: false,
        base_llm_exception: false,
    };
    is_expected_client_error(Some(&facts))
}

pub(crate) fn build(
    request: &PayloadRequest<'_>,
    metadata: &MetadataMap,
    composition: &PayloadComposition<'_>,
) -> Result<(ErrorInformation, Option<String>), PayloadError> {
    let context = ExceptionLogging {
        exception: request.original_exception,
        log_client_error_tracebacks: composition.settings.log_client_error_tracebacks,
        expected_client_error: expected_client_error(request.original_exception),
        redactor: Some(composition.redactor),
        normalizer: composition.normalizer,
    };
    Ok(payload_error_information(&PayloadErrorRequest {
        context: &context,
        metadata,
        error_str: request.error_str.map(str::to_owned),
    })?)
}
