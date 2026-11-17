use betterlite_core::api::media_relay_logging::spend::MediaSpend;
use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_vertex::endpoint_target::passthrough::get_vertex_location_from_url;
use betterlite_vertex::gemini_embedding_records::input::{EmbeddingInput, ResolvedFiles};
use betterlite_vertex::gemini_embedding_records::response::{
    process_batch_reply, process_embed_content_reply,
};
use betterlite_vertex::vertex_relay_metadata::types::{CompletionCost, CompletionCostRequest};
use betterlite_vertex::vertex_relay_metadata::url_models::{
    custom_provider_from_url, extract_model_from_url,
};

use super::counter::CharacterCounter;
use super::input::extract_input;
use super::wire::{decode_body, json_of, object_fields};
use super::{VertexEmbedContentInput, VertexPredictError, cost, spend};

const BATCH_ROUTE: &str = "batchEmbedContents";

pub(super) fn embed_content(
    input: &VertexEmbedContentInput<'_>,
    cost_service: &dyn CompletionCost,
) -> Result<HandlerOutcome, VertexPredictError> {
    let body = decode_body(input.response_body)?;
    object_fields(&body)?;
    let model = extract_model_from_url(input.url_route);
    let batch = input.url_route.contains(BATCH_ROUTE);
    let embedding_input = EmbeddingInput::Text(extract_input(input.request_body, batch));
    let counter = CharacterCounter;
    let mut reply = if batch {
        process_batch_reply(&embedding_input, &model, &body, &counter)?
    } else {
        let files = ResolvedFiles::new();
        process_embed_content_reply(&embedding_input, &model, &body, &files, &counter)?
    };
    model.clone_into(&mut reply.model);
    let result = PassThroughResponse::Embedding(json_of(&reply)?);
    let provider = custom_provider_from_url(input.url_route);
    let cost_response =
        cost::embedding_response(&model, reply.usage.prompt_tokens, reply.usage.total_tokens);
    let response_cost = cost_service
        .completion_cost(&CompletionCostRequest {
            response: &cost_response,
            model: &model,
            custom_llm_provider: provider,
            vertex_location: get_vertex_location_from_url(input.url_route),
        })
        .ok_or_else(|| VertexPredictError::Cost {
            model: model.clone(),
        })?;
    let spend = MediaSpend {
        model,
        custom_llm_provider: provider.as_str(),
        response_cost: Some(response_cost),
    };
    Ok(spend::outcome(
        input.logging_obj,
        &spend,
        result,
        &input.kwargs,
    ))
}
