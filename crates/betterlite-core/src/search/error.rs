use std::error::Error;
use std::time::TryFromFloatSecsError;

use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::ErrorClass;
use betterlite_error::error::details::ResponseDetails;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use super::query::QueryError;
use crate::api::search_http::SearchHttpError;

#[derive(Debug, Error)]
pub enum SearchCallError<HookError>
where
    HookError: Error + 'static,
{
    #[error(transparent)]
    Query(#[from] QueryError),
    #[error("Search is not supported for provider: {provider}")]
    Unsupported { provider: String },
    #[error("{0}")]
    Url(#[source] HookError),
    #[error("a search option cannot be encoded as JSON")]
    Encode(#[source] serde_json::Error),
    #[error("the request timeout of {seconds} seconds is not a valid duration")]
    Timeout {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    #[error(transparent)]
    Http(#[from] SearchHttpError<HookError>),
}

#[derive(Debug, Error)]
#[error("{error}")]
pub struct SearchFailure<HookError>
where
    HookError: Error + 'static,
{
    #[source]
    pub error: SearchCallError<HookError>,
    pub class: ErrorClass,
    // exception_type copies the provider's response headers onto the mapped exception as litellm_response_headers
    pub response_headers: Vec<(String, String)>,
}

// litellm.exception_type(model=f"{provider}/search", custom_llm_provider=provider, original_exception=e)
pub(super) fn failure<HookError>(
    error: SearchCallError<HookError>,
    provider: &str,
    redactor: &Redactor,
) -> SearchFailure<HookError>
where
    HookError: Error + 'static,
{
    let model = format!("{provider}/search");
    // a wrong query or an unmapped provider is the caller's mistake, where LiteLLM's ValueError maps to a 500
    if matches!(
        error,
        SearchCallError::Query(_) | SearchCallError::Unsupported { .. }
    ) {
        return SearchFailure {
            class: ErrorClass::BadRequest(Box::new(ResponseDetails::for_provider(
                error.to_string(),
                Some(model),
                provider,
            ))),
            error,
            response_headers: Vec::new(),
        };
    }
    let (raw, response_headers) = raw_exception(&error, model, provider);
    let class = dispatch(&DispatchInput::Raw(Box::new(raw)), redactor);
    SearchFailure {
        error,
        class,
        response_headers,
    }
}

fn raw_exception<HookError>(
    error: &SearchCallError<HookError>,
    model: String,
    provider: &str,
) -> (RawException, Vec<(String, String)>)
where
    HookError: Error + 'static,
{
    let mut raw = RawException {
        model,
        custom_llm_provider: provider.to_owned(),
        error_str: error.to_string(),
        status_code: None,
        status_code_is_synthesized: false,
        message: None,
        body: None,
        exception_type: "ValueError".to_owned(),
    };
    let SearchCallError::Http(SearchHttpError::Provider {
        provider: exception,
    }) = error
    else {
        return (raw, Vec::new());
    };
    raw.status_code = Some(exception.status_code);
    raw.status_code_is_synthesized = exception.status_code_is_synthesized;
    raw.message = Some(exception.message.clone());
    raw.exception_type = String::new();
    (raw, exception.headers.clone().unwrap_or_default())
}
