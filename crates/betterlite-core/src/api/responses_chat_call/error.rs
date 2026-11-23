use std::error::Error;

use thiserror::Error;

use crate::api::chat_response_stream::error::ChatResponseStreamError;
use crate::api::responses_chat_options::error::ChatOptionsError;
use crate::api::responses_chat_output::error::ChatOutputError;
use crate::api::responses_session_history::error::SessionHistoryError;

// B2410 hoisting, the typed fields the handler reads from the caller's kwargs, and B908's mapping
#[derive(Debug, Error)]
pub enum PreparationError {
    #[error(transparent)]
    Options(#[from] ChatOptionsError),
    #[error("the {field} value could not be written as JSON: {source}")]
    Encode {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the {field} field must be {expected}")]
    FieldType {
        field: &'static str,
        expected: &'static str,
    },
}

// B914 and B924 take the completion result; the provider type a Python caller answered with is not one of them
#[derive(Debug, Error)]
pub enum ConversionError {
    #[error(transparent)]
    Output(#[from] ChatOutputError),
    #[error(transparent)]
    Stream(#[from] ChatResponseStreamError),
    #[error("the Responses request could not be read: {source}")]
    Request {
        #[source]
        source: serde_json::Error,
    },
    #[error("Unexpected response type: <class '{python_type}'>")]
    UnexpectedResponse { python_type: String },
}

#[derive(Debug, Error)]
pub enum CallError<Source>
where
    Source: Error + 'static,
{
    #[error(transparent)]
    Preparation(#[from] PreparationError),
    #[error(transparent)]
    Conversion(#[from] ConversionError),
    #[error(transparent)]
    Completion(Source),
}

// The async entry restores B915's history first, so its failure joins the call's
#[derive(Debug, Error)]
pub enum AsyncCallError<Source, Cold>
where
    Source: Error + 'static,
    Cold: Error + 'static,
{
    #[error(transparent)]
    Call(#[from] CallError<Source>),
    #[error(transparent)]
    Session(#[from] SessionHistoryError<Cold>),
}

impl<Source, Cold> From<PreparationError> for AsyncCallError<Source, Cold>
where
    Source: Error + 'static,
    Cold: Error + 'static,
{
    fn from(error: PreparationError) -> Self {
        Self::Call(CallError::Preparation(error))
    }
}
