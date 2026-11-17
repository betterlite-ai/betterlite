use std::collections::BTreeMap;

use betterlite_models::cost_usage::CostUsage;
use betterlite_vertex::vertex_relay_metadata::types::CompletionResponse;

// LiteLLM prices the EmbeddingResponse it built, whose model and token counts are all the cost reads.
pub(super) fn embedding_response(
    model: &str,
    prompt_tokens: u32,
    total_tokens: u32,
) -> CompletionResponse {
    CompletionResponse {
        id: None,
        model: Some(model.to_owned()),
        usage: Some(CostUsage {
            prompt_tokens: i64::from(prompt_tokens),
            completion_tokens: 0,
            total_tokens: i64::from(total_tokens),
            prompt_details: None,
            completion_details: None,
            cache_read_tokens: None,
            numeric_extras: BTreeMap::new(),
        }),
    }
}

// An ImageResponse carries no model and its default ImageUsage is all zeros.
pub(super) fn image_response() -> CompletionResponse {
    CompletionResponse {
        id: None,
        model: None,
        usage: Some(CostUsage::default()),
    }
}
