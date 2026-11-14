use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::image_generation::reply::ImageUsage;

// LiteLLM's ImageResponse has no prompt or completion counts and dumps the SDK's output_tokens_details
#[must_use]
pub fn usage_identity(usage: ImageUsage) -> ImageUsage {
    if usage.extra.is_empty() {
        return usage;
    }
    let output_tokens_details = match usage.extra.get("output_tokens_details") {
        Some(Json::Object(fields)) => Json::Object(IndexMap::from([
            (
                String::from("image_tokens"),
                fields.get("image_tokens").cloned().unwrap_or(Json::Null),
            ),
            (
                String::from("text_tokens"),
                fields.get("text_tokens").cloned().unwrap_or(Json::Null),
            ),
        ])),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Json::Null
        }
    };
    ImageUsage {
        prompt_tokens: None,
        completion_tokens: None,
        extra: IndexMap::from([(String::from("output_tokens_details"), output_tokens_details)]),
        ..usage
    }
}
