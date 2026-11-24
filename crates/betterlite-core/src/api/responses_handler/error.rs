use std::error::Error as StdError;

use thiserror::Error;

use super::state::InteractionsStreamStateError;
use crate::api::responses_transform::error::ResponsesTransformError;

// The handler's failures: the request or reply could not be translated, or the Responses operation itself failed.
#[derive(Debug, Error)]
pub enum InteractionsHandlerError<CallError>
where
    CallError: StdError + 'static,
{
    #[error("the interaction request or response could not be translated")]
    Transform(#[source] ResponsesTransformError),
    #[error("the responses call failed")]
    Call(#[source] CallError),
}

// An iterator step's failures: the provider stream failed, or the state refused a chunk it could not keep bounded.
#[derive(Debug, Error)]
pub enum InteractionsStreamError<Error>
where
    Error: StdError + 'static,
{
    #[error("the responses stream failed")]
    Upstream(#[source] Error),
    #[error("the interaction stream could not keep the synthesized event bounded")]
    State(#[source] InteractionsStreamStateError),
}
