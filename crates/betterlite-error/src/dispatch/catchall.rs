use crate::dispatch::{RawException, exception_provider_label};
use crate::error::{ErrorClass, ErrorContext};
use crate::facts::ProviderErrorFacts;
use crate::status::status_error_class;

// the openai-python SDK bug exception_type() checks against str(original_exception), ahead of every status-based fallback
const BAD_REQUEST_SDK_BUG: &str =
    "BadRequestError.__init__() missing 1 required positional argument: 'param'";

// the unconditional raise at the end of exception_type()'s try block, reached once no provider branch raises
pub(super) fn catchall(raw: &RawException, error_str: &str) -> ErrorClass {
    if let Some(class) = bad_request_sdk_bug(raw) {
        return class;
    }
    status_or_api_connection(raw, error_str)
}

// str(original_exception), never the message-overridden error_str already passed into catchall
fn bad_request_sdk_bug(raw: &RawException) -> Option<ErrorClass> {
    if !raw.error_str.contains(BAD_REQUEST_SDK_BUG) {
        return None;
    }
    let exception_provider = exception_provider_label(&raw.custom_llm_provider);
    let context = ErrorContext {
        message: format!(
            "{exception_provider} BadRequestError : This can happen due to missing AZURE_API_VERSION: {}",
            raw.error_str
        ),
        model: Some(raw.model.clone()),
        provider: Some(raw.custom_llm_provider.clone()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    };
    Some(ErrorClass::BadRequest(Box::new(context)))
}

// _map_exception_by_status runs first; a synthetic status never outranks this, since it bails out early
fn status_or_api_connection(raw: &RawException, error_str: &str) -> ErrorClass {
    let exception_provider = exception_provider_label(&raw.custom_llm_provider);
    let facts = ProviderErrorFacts {
        status_code: raw.status_code,
        status_code_is_synthesized: raw.status_code_is_synthesized,
        message: None,
        body: None,
        headers: None,
        response: None,
        litellm_response_headers: None,
        request: None,
    };
    if let Some(class) = status_error_class(
        &facts,
        error_str,
        &exception_provider,
        Some(raw.model.clone()),
        Some(raw.custom_llm_provider.clone()),
    ) {
        return class;
    }
    let context = ErrorContext {
        message: format!("{exception_provider} - {error_str}"),
        model: Some(raw.model.clone()),
        provider: Some(raw.custom_llm_provider.clone()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    };
    ErrorClass::ApiConnection(Box::new(context))
}
