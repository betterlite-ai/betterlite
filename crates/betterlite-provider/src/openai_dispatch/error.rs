use betterlite_openai::text_completion_policy::reply::ReplyError;
use betterlite_openai::text_completion_transport::error::TextCompletionFailure;
use http::header::InvalidHeaderName;
use thiserror::Error;

use crate::operation::OperationError;
use crate::prepared::deployment::DeploymentError;

#[derive(Debug, Error)]
pub enum OpenAiProviderError {
    #[error(
        "the openai provider composition runs only chat, embeddings and text-completion operations"
    )]
    UnsupportedOperation,
    #[error(transparent)]
    Operation(#[from] OperationError),
    #[error(transparent)]
    Deployment(#[from] DeploymentError),
    #[error(transparent)]
    Text(#[from] TextCompletionFailure),
    #[error("the text completion reply cannot be converted to a chat completion: {source}")]
    Reply {
        #[from]
        source: ReplyError,
    },
    #[error("the {field} field must be {expected}")]
    FieldType {
        field: &'static str,
        expected: &'static str,
    },
    #[error("the {name} header name HTTP cannot carry: {source}")]
    HeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
}
