use betterlite_logging::logging_pre_post::error::PrePostError;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use thiserror::Error;

use crate::api::responses_agentic_execution::error::AgenticExecutionError;
use crate::api::responses_compact_http::error::ConfigFailure;
use crate::api::responses_hook_policy::error::DeploymentHookError;
use crate::api::vector_store_http_write::error::{
    BaseLlmException, ClientError, FailureFacts, handle_error,
};

use super::config::CreateConfig;

// Which handler step a provider config failure came from: only the send failures reach _handle_error
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Url,
    Validate,
    Transform,
    Normalize,
    Sign,
    Response,
}

impl Stage {
    const fn text(self) -> &'static str {
        match self {
            Self::Url => "URL resolution",
            Self::Validate => "environment validation",
            Self::Transform => "request transform",
            Self::Normalize => "request normalization",
            Self::Sign => "request signing",
            Self::Response => "response transform",
        }
    }
}

#[derive(Debug, Error)]
pub enum ParamError {
    #[error("litellm_params.{field} must be {expected}, got {found}")]
    FieldType {
        field: String,
        expected: &'static str,
        found: &'static str,
    },
    #[error("the deployment hook returned an input that is not a string or a list of input items")]
    HookInput(#[source] serde_json::Error),
}

#[derive(Debug, Error)]
pub enum CreateError {
    #[error(transparent)]
    Provider(#[from] BaseLlmException),
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error(transparent)]
    Logging(#[from] PrePostError),
    #[error(transparent)]
    Hook(#[from] DeploymentHookError),
    #[error(transparent)]
    Agentic(#[from] AgenticExecutionError),
    #[error(transparent)]
    Params(#[from] ParamError),
    #[error("{} failed: {source}", .stage.text())]
    Step { stage: Stage, source: ConfigFailure },
    #[error("the request URL {url} is not a URL")]
    RequestUrl {
        url: String,
        #[source]
        source: url::ParseError,
    },
    #[error("the {name} header name is not a header name")]
    HeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the {name} header value is not a header value")]
    HeaderValue {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("optional_params.extra_headers must be an object, got {found}")]
    OptionalHeaders { found: &'static str },
    #[error("the {name} optional header must be a string, got {found}")]
    OptionalHeader { name: String, found: &'static str },
    #[error("the client streamed a request that asked for a buffered body")]
    StreamContract,
    #[error("the fake stream response did not decode as a Responses response")]
    NotTyped,
    #[error("the agentic result is a Messages stream, which the responses surface never produces")]
    AgenticStream,
    #[error("the request body could not be encoded")]
    Encode(#[source] serde_json::Error),
}

// _handle_error's funnel: an HTTP error keeps its status, text and masked headers; anything else becomes a synthesized 500
#[must_use]
pub fn provider_error<C>(facts: &FailureFacts, config: &C) -> BaseLlmException
where
    C: CreateConfig,
{
    let fallback = handle_error(facts, None);
    let mut error = config.error_class(&fallback.message, fallback.status_code, fallback.headers);
    if fallback.status_code_is_synthesized {
        error.status_code_is_synthesized = true;
    }
    error
}
