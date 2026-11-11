use betterlite_http::error::HttpError;
use thiserror::Error;

use crate::aws_endpoints::error::AwsEndpointError;
use crate::bedrock_settings::{BedrockError, HeaderError};
use crate::signing::error::SigningError;

// B682's BedrockError implements no Display or Error, so the derived wrapper carries the cause instead.
#[derive(Debug, Error)]
#[error("bedrock provider error")]
pub struct BedrockFailure {
    pub cause: Box<BedrockError>,
}

#[derive(Debug, Error)]
pub enum VectorStoreError {
    #[error(transparent)]
    VectorStoreId(#[from] HttpError),
    #[error(transparent)]
    Endpoint(#[from] AwsEndpointError),
    #[error("the filter must be an object")]
    FilterNotObject,
    #[error("the filter operator must be a string")]
    FilterOperatorNotString,
    #[error("the filter is missing {field}")]
    FilterMissingField { field: &'static str },
    #[error("the {field} compound filter must be an array")]
    FilterListNotArray { field: &'static str },
    #[error("retrievalConfiguration must be an object")]
    RetrievalConfigNotObject,
    #[error("retrievalConfiguration.vectorSearchConfiguration must be an object")]
    SearchConfigurationNotObject,
    #[error("the knowledge base reply cannot be encoded")]
    Encode {
        #[from]
        source: serde_json::Error,
    },
    #[error("bedrock provider error ({status})")]
    Provider {
        status: u16,
        #[source]
        error: BedrockFailure,
    },
    #[error("the response headers cannot be encoded")]
    Headers {
        #[source]
        source: HeaderError,
    },
    #[error(transparent)]
    Signing(#[from] SigningError),
    #[error("the Bedrock knowledge base create operation is not implemented")]
    CreateNotSupported,
}
