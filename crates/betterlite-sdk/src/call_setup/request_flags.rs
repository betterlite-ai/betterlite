use betterlite_models::decisions::Json;

use crate::call_setup::RequestMap;

const ASYNC_FLAGS: [&str; 11] = [
    "acompletion",
    "aembedding",
    "aimg_generation",
    "amoderation",
    "atext_completion",
    "atranscription",
    "arerank",
    "_arealtime",
    "acreate_batch",
    "acreate_fine_tuning_job",
    "aresponses",
];

#[must_use]
pub fn is_async_request(request: &RequestMap, is_pass_through: bool) -> bool {
    is_pass_through
        || ASYNC_FLAGS
            .iter()
            .any(|flag| matches!(request.get(*flag), Some(Json::Bool(true))))
}

#[must_use]
pub fn is_streaming_request(request: &RequestMap, call_type: &str) -> bool {
    matches!(request.get("stream"), Some(Json::Bool(true)))
        || matches!(
            call_type,
            "generate_content_stream" | "agenerate_content_stream"
        )
}
