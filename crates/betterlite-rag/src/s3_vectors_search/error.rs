use betterlite_bedrock::aws_endpoints::error::AwsEndpointError;
use betterlite_bedrock::signing::error::SigningError;
use betterlite_provider::api::vector_store_contracts::error::BaseLlmException;
use thiserror::Error;

use crate::s3_vectors_ingestion::S3VectorsInputError;

#[derive(Debug, Error)]
pub enum S3VectorsSearchError {
    #[error(transparent)]
    Target(#[from] S3VectorsInputError),
    #[error(transparent)]
    Region(#[from] AwsEndpointError),
    #[error("litellm_params.{field} must be a string")]
    ParamNotText { field: &'static str },
    #[error("queryVector[{index}] is not a finite number")]
    NonFiniteVector { index: usize },
    #[error(transparent)]
    Signing(#[from] SigningError),
    #[error(transparent)]
    Provider(#[from] BaseLlmException),
    #[error("S3 Vectors does not support creating a vector store")]
    CreateNotSupported,
}
