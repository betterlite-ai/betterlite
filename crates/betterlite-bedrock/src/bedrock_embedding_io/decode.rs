use betterlite_prompts::arguments::json::Object;

use super::error::EmbeddingIoError;

// LiteLLM calls response.json() and hands the dict on; a body that is not a JSON object is refused here
pub fn decode_embedding_response(body: &[u8]) -> Result<Object, EmbeddingIoError> {
    match serde_json::from_slice(body) {
        Ok(object) => Ok(object),
        Err(source) => Err(EmbeddingIoError::Decode { source }),
    }
}
