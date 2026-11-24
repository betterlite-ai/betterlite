use std::error::Error as StdError;

use thiserror::Error;

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
