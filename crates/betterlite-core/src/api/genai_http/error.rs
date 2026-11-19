use std::error::Error;

use betterlite_logging::logging_pre_post::error::PrePostError;
use thiserror::Error;

use crate::api::vector_store_http_write::error::{
    BaseLlmException, ClientError, FailureFacts, handle_error,
};

use super::config::GenerateContentHttpConfig;

#[derive(Debug, Error)]
pub enum GenAiHttpError<Source>
where
    Source: Error + 'static,
{
    #[error(transparent)]
    Provider(#[from] BaseLlmException),
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error(transparent)]
    Logging(#[from] PrePostError),
    #[error("the provider config could not authenticate the request")]
    Auth(#[source] Source),
    #[error("the provider config could not build the request body")]
    Transform(#[source] Source),
    #[error("the provider config built a request body that is not an object")]
    BodyNotObject,
    #[error("the transport answered a stream to a request that did not stream")]
    UnexpectedStream,
}

// _handle_error normalizes the status, text and headers, then the provider config names the class
pub fn classed<C>(config: &C, facts: &FailureFacts) -> BaseLlmException
where
    C: GenerateContentHttpConfig,
{
    let BaseLlmException {
        status_code,
        message,
        headers,
        status_code_is_synthesized,
    } = handle_error(facts, None);
    let mut provider = config.error_class(&message, status_code, headers);
    provider.status_code_is_synthesized = status_code_is_synthesized;
    provider
}

// A request the wire cannot carry fails inside LiteLLM's try block too, so it gets the same synthesized 500
pub fn wire_failure<C>(config: &C, display: String) -> BaseLlmException
where
    C: GenerateContentHttpConfig,
{
    classed(
        config,
        &FailureFacts {
            response_status: None,
            status: None,
            text: None,
            headers: None,
            response: None,
            display,
        },
    )
}
