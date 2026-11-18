use std::error::Error as StdError;

use betterlite_anthropic::anthropic_http_stream::completion::{CompletionError, ParameterError};
use betterlite_anthropic::anthropic_http_stream::error::StreamOpenError;
use betterlite_anthropic::chat_call::error::ChatCallError;
use betterlite_anthropic::response::error::ResponseError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use thiserror::Error;

use super::environment::EnvironmentError;

#[derive(Debug, Error)]
pub enum ChatError<ClientFailure, CounterFailure>
where
    ClientFailure: StdError + Send + Sync + 'static,
    CounterFailure: StdError + Send + Sync + 'static,
{
    #[error(transparent)]
    Environment(#[from] EnvironmentError),
    #[error(transparent)]
    Call(#[from] ChatCallError<ClientFailure, CounterFailure>),
}

impl<ClientFailure, CounterFailure> ChatError<ClientFailure, CounterFailure>
where
    ClientFailure: StdError + Send + Sync + 'static,
    CounterFailure: StdError + Send + Sync + 'static,
{
    // The status LiteLLM puts on the exception this call raises, as far as the error carries one
    #[must_use]
    pub fn status_code(&self) -> Option<u16> {
        match self {
            Self::Environment(source) => source.status(),
            Self::Call(source) => source.status_code(),
        }
    }
}

impl<ClientFailure, CounterFailure> From<ParameterError>
    for ChatError<ClientFailure, CounterFailure>
where
    ClientFailure: StdError + Send + Sync + 'static,
    CounterFailure: StdError + Send + Sync + 'static,
{
    fn from(source: ParameterError) -> Self {
        Self::Call(ChatCallError::Parameter(source))
    }
}

impl<ClientFailure, CounterFailure> From<CompletionError<ClientFailure>>
    for ChatError<ClientFailure, CounterFailure>
where
    ClientFailure: StdError + Send + Sync + 'static,
    CounterFailure: StdError + Send + Sync + 'static,
{
    fn from(source: CompletionError<ClientFailure>) -> Self {
        Self::Call(ChatCallError::from(source))
    }
}

impl<ClientFailure, CounterFailure> From<StreamOpenError<ClientFailure>>
    for ChatError<ClientFailure, CounterFailure>
where
    ClientFailure: StdError + Send + Sync + 'static,
    CounterFailure: StdError + Send + Sync + 'static,
{
    fn from(source: StreamOpenError<ClientFailure>) -> Self {
        Self::Call(ChatCallError::from(source))
    }
}

impl<ClientFailure, CounterFailure> From<PrePostError> for ChatError<ClientFailure, CounterFailure>
where
    ClientFailure: StdError + Send + Sync + 'static,
    CounterFailure: StdError + Send + Sync + 'static,
{
    fn from(source: PrePostError) -> Self {
        Self::Call(ChatCallError::PreCall(source))
    }
}

impl<ClientFailure, CounterFailure> From<ResponseError<CounterFailure>>
    for ChatError<ClientFailure, CounterFailure>
where
    ClientFailure: StdError + Send + Sync + 'static,
    CounterFailure: StdError + Send + Sync + 'static,
{
    fn from(source: ResponseError<CounterFailure>) -> Self {
        Self::Call(ChatCallError::Response(source))
    }
}

impl<ClientFailure, CounterFailure> From<url::ParseError>
    for ChatError<ClientFailure, CounterFailure>
where
    ClientFailure: StdError + Send + Sync + 'static,
    CounterFailure: StdError + Send + Sync + 'static,
{
    fn from(source: url::ParseError) -> Self {
        Self::Call(ChatCallError::Endpoint(source))
    }
}
