use betterlite_error::error::ErrorClass;
use betterlite_error::error::details::ResponseDetails;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::evals::error::EvalsError;
use thiserror::Error;

// Everything a handler can fail with: the provider config's own error, a pre-call that stopped the call, and a body the pre-call could not record
#[derive(Debug, Error)]
pub enum EvalHttpError {
    #[error(transparent)]
    Api(#[from] EvalsError),
    #[error(transparent)]
    Logging(#[from] PrePostError),
    #[error("the eval request could not be recorded")]
    Record {
        #[source]
        source: serde_json::Error,
    },
}

// the provider name is the caller's input and the call can never succeed, so LiteLLM's ValueError becomes a 400
pub(crate) fn unsupported_provider(message: String, provider: &str) -> ErrorClass {
    ErrorClass::BadRequest(Box::new(ResponseDetails::for_provider(
        message,
        Some(String::new()),
        provider,
    )))
}
