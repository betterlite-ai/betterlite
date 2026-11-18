use serde_json::{Value, json};

use betterlite_bedrock::aws_endpoints::provider::BedrockEmbeddingProvider;
use betterlite_bedrock::bedrock_embedding_io::error::EmbeddingIoError;
use betterlite_bedrock::bedrock_embedding_io::response::transform_response;

use super::failure::Failure;
use super::input::{Provider, TransformRequest};

pub(crate) fn run(case: &TransformRequest) -> Result<Value, Failure> {
    match transform_response(
        &case.response_list,
        &case.model,
        provider(case.provider),
        case.is_async_invoke,
        case.batch_data.as_deref(),
    ) {
        Ok(reply) => Ok(serde_json::to_value(reply)?),
        Err(error @ EmbeddingIoError::Unmapped { .. }) => Ok(json!({
            "error": {"kind": "unmapped", "message": error.to_string()},
        })),
        Err(error) => Err(Failure::Embedding(error)),
    }
}

const fn provider(provider: Provider) -> BedrockEmbeddingProvider {
    match provider {
        Provider::Amazon => BedrockEmbeddingProvider::Amazon,
        Provider::Cohere => BedrockEmbeddingProvider::Cohere,
        Provider::Nova => BedrockEmbeddingProvider::Nova,
        Provider::Twelvelabs => BedrockEmbeddingProvider::Twelvelabs,
    }
}
