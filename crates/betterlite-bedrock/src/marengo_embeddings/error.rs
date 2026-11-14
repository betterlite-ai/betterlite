use thiserror::Error;

use crate::marengo_three::error::Marengo3Error;

#[derive(Debug, Error)]
pub enum MarengoEmbeddingsError {
    #[error(
        "Input type '{input_type}' requires async_invoke route. Use model format: 'bedrock/async_invoke/model_id'"
    )]
    AsyncRouteRequired { input_type: String },
    #[error("output_s3_uri cannot be empty for async invoke requests")]
    EmptyOutputS3Uri,
    #[error("embedding usage counts overflow")]
    UsageOverflow,
    #[error(transparent)]
    Marengo3(#[from] Marengo3Error),
    #[error("invalid Marengo parameters")]
    Parameters {
        #[source]
        source: serde_json::Error,
    },
}
