use std::collections::BTreeMap;

use betterlite_models::cost_usage::CostUsage;
use betterlite_vertex::endpoint_target::passthrough::get_vertex_location_from_url;
use betterlite_vertex::vertex_relay_metadata::types::{
    CompletionCost, CompletionCostRequest, CompletionResponse, CustomLlmProvider,
};
use rust_decimal::Decimal;

use super::VertexPredictError;

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

// An ImageResponse carries no model, no image count and a default ImageUsage of zeros, so the trait prices it without the images.
pub(super) fn image_response() -> CompletionResponse {
    CompletionResponse {
        id: None,
        model: None,
        usage: Some(CostUsage::default()),
    }
}

// LiteLLM calls completion_cost with the built response, the model, the provider and the URL's location, and a price it cannot find fails the dispatch.
pub(super) fn charge(
    cost_service: &dyn CompletionCost,
    response: &CompletionResponse,
    model: &str,
    custom_llm_provider: CustomLlmProvider,
    url_route: &str,
) -> Result<Decimal, VertexPredictError> {
    cost_service
        .completion_cost(&CompletionCostRequest {
            response,
            model,
            custom_llm_provider,
            vertex_location: get_vertex_location_from_url(url_route),
        })
        .ok_or_else(|| VertexPredictError::Cost {
            model: model.to_owned(),
        })
}
