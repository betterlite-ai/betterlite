use betterlite_models::decisions::Json;

use crate::embedding::response::EmbeddingResponse;

// LiteLLM's isinstance list in async_set_cache: the five response models whose json string is cached, plus anything else
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CacheableResult {
    Chat(Json),
    TextCompletion(Json),
    Embedding(EmbeddingResponse),
    Transcription(Json),
    Rerank(Json),
    Responses(Json),
    Other(Json),
}

impl CacheableResult {
    // result.model_dump_json() for a typed response; Other is cached raw instead and never comes through here
    pub(crate) fn json_text(&self) -> Result<String, serde_json::Error> {
        match self {
            Self::Embedding(response) => serde_json::to_string(response),
            Self::Chat(json)
            | Self::TextCompletion(json)
            | Self::Transcription(json)
            | Self::Rerank(json)
            | Self::Responses(json)
            | Self::Other(json) => serde_json::to_string(json),
        }
    }
}
