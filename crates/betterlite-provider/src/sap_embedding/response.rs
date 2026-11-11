use betterlite_openai::embedding::EmbeddingResponse;
use serde::Deserialize;

use super::error::SapEmbeddingError;

#[derive(Deserialize)]
struct EmbeddingReply {
    final_result: EmbeddingResponse,
}

pub fn transform_embedding_response(
    raw_response: &str,
) -> Result<EmbeddingResponse, SapEmbeddingError> {
    serde_json::from_str::<EmbeddingReply>(raw_response)
        .map(|reply| reply.final_result)
        .map_err(SapEmbeddingError::Response)
}
