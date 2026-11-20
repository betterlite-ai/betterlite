use std::error::Error as StdError;

use betterlite_http::interactions::base::{
    CallFailure, ClientError, ErrorClasses, RequestFailure, ResponseHeaders, handle_error,
};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use thiserror::Error;

use super::config::CharacterConfig;

// what a character call raises: setup and request transforms stay typed, and the send's failures take the shape _handle_error gives them
#[derive(Debug, Error)]
pub enum CharacterError<ProviderError>
where
    ProviderError: StdError + 'static,
{
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error(transparent)]
    Logging(#[from] PrePostError),
    #[error(transparent)]
    Multipart(TransportError),
    #[error("{0}")]
    Provider(#[source] ProviderError),
    #[error(transparent)]
    Transport(RequestFailure),
}

pub type CharacterFault<Config> = CharacterError<<Config as ErrorClasses>::Error>;

// _handle_error: a failing status becomes the provider's own error class, everything else keeps its typed reason
pub(crate) fn converted<C>(config: &C, failure: RequestFailure) -> CharacterFault<C>
where
    C: CharacterConfig,
{
    match handle_error(config, failure) {
        CallFailure::Provider(error) => CharacterError::Provider(error),
        CallFailure::PassedThrough(passed) => CharacterError::Transport(passed),
    }
}

// _handle_error reads a caught exception without a response as a synthesized 500 whose text is the exception
pub(crate) fn synthesized<C>(config: &C, error: &C::Error) -> CharacterFault<C>
where
    C: CharacterConfig,
{
    let failure = RequestFailure::Status {
        status_code: 500,
        headers: ResponseHeaders::new(),
        body: error.to_string(),
    };
    converted(config, failure)
}
