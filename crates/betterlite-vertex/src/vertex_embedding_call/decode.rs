use betterlite_models::decisions::Json;

use super::error::VertexEmbeddingCallError;

// The same cap the multimodal decoder uses, so an embedding answer never decodes without a bound
pub const RESPONSE_BODY_LIMIT: usize = 0x0200_0000;

pub fn decode_vertex_embedding(bytes: &[u8]) -> Result<Json, VertexEmbeddingCallError> {
    if bytes.len() > RESPONSE_BODY_LIMIT {
        return Err(VertexEmbeddingCallError::ResponseTooLarge {
            size: bytes.len(),
            limit: RESPONSE_BODY_LIMIT,
        });
    }
    serde_json::from_slice(bytes).map_err(|source| VertexEmbeddingCallError::Decode { source })
}
