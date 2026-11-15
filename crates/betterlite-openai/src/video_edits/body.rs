use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::openai_video_identifiers::ids::decode_video_id_with_provider;

// LiteLLM sends the provider's own video id, not the provider-encoded one, on both edit and extension
#[must_use]
pub fn video_reference(video_id: &str) -> Json {
    Json::Object(IndexMap::from([(
        String::from("id"),
        Json::Text(decode_video_id_with_provider(video_id).video_id),
    )]))
}
