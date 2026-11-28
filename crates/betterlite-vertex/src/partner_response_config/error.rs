use std::convert::Infallible;
use std::error::Error as StdError;

use betterlite_anthropic::response::error::ResponseError;
use betterlite_openai::chat_choices::reply::TransformResponseError;
use thiserror::Error;

use crate::model_policy::error::VertexAiError;

// One variant per leaf transform's error, so a caller can tell which partner failed and keep its status.
#[derive(Debug, Error)]
pub enum PartnerResponseError<CounterFailure = Infallible>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    #[error(transparent)]
    Claude(#[from] ResponseError<CounterFailure>),
    #[error(transparent)]
    Ai21(#[from] TransformResponseError),
    #[error(transparent)]
    Llama(#[from] VertexAiError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl<CounterFailure> PartnerResponseError<CounterFailure>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    #[must_use]
    pub fn status_code(&self) -> Option<u16> {
        match self {
            Self::Claude(error) => match error {
                ResponseError::Anthropic { status, .. } => Some(*status),
                ResponseError::Decode(_)
                | ResponseError::MissingField(_)
                | ResponseError::MissingChoice
                | ResponseError::FieldType { .. }
                | ResponseError::Parameter { .. }
                | ResponseError::Content(_)
                | ResponseError::Usage { .. } => None,
            },
            Self::Ai21(error) => match error {
                TransformResponseError::Json(record) => Some(record.status_code),
                TransformResponseError::Projection(_) | TransformResponseError::Base { .. } => None,
            },
            Self::Llama(error) => Some(error.status_code),
            Self::Json(_) => None,
        }
    }
}
