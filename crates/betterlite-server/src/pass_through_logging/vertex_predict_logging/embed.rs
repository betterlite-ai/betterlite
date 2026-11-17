use betterlite_core::api::media_relay_logging::spend::MediaSpend;
use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::decisions::Json;
use betterlite_vertex::gemini_embedding_records::input::{EmbeddingInput, ResolvedFiles};
use betterlite_vertex::gemini_embedding_records::response::{
    EmbeddingReply, process_batch_reply, process_embed_content_reply,
};
use betterlite_vertex::vertex_relay_metadata::types::CompletionCost;
use betterlite_vertex::vertex_relay_metadata::url_models::{
    custom_provider_from_url, extract_model_from_url,
};
use serde::Serialize;

use super::counter::Cl100kCounter;
use super::input::extract_input;
use super::wire::{json_of, object_fields};
use super::{VertexEmbedContentInput, VertexPredictError, cost, spend};

const BATCH_ROUTE: &str = "batchEmbedContents";
const LIST_OBJECT: &str = "list";

// LiteLLM's EmbeddingResponse constructor always sets object to "list"; B1340's reply holds only model, data and usage.
#[derive(Serialize)]
struct EmbeddingResult<'reply> {
    object: &'static str,
    #[serde(flatten)]
    reply: &'reply EmbeddingReply,
}

pub(super) fn embed_content(
    input: VertexEmbedContentInput<'_>,
    cost_service: &dyn CompletionCost,
    counter: Option<&Cl100kCounter>,
) -> Result<HandlerOutcome, VertexPredictError> {
    let VertexEmbedContentInput {
        logging_obj,
        url_route,
        response_body,
        request_body,
        kwargs,
        ..
    } = input;
    let body: Json = serde_json::from_slice(response_body)
        .map_err(|source| VertexPredictError::Decode { source })?;
    object_fields(&body)?;
    let cl100k = counter.ok_or(VertexPredictError::Tokenizer)?;
    let model = extract_model_from_url(url_route);
    let mut reply = build_reply(request_body, url_route, &body, &model, cl100k)?;
    model.clone_into(&mut reply.model);
    let result = PassThroughResponse::Embedding(json_of(&EmbeddingResult {
        object: LIST_OBJECT,
        reply: &reply,
    })?);
    let provider = custom_provider_from_url(url_route);
    let response_cost = cost::charge(
        cost_service,
        &cost::embedding_response(&model, reply.usage.prompt_tokens, reply.usage.total_tokens),
        &model,
        provider,
        url_route,
    )?;
    let spend = MediaSpend {
        model,
        custom_llm_provider: provider.as_str(),
        response_cost: Some(response_cost),
    };
    Ok(spend::outcome(logging_obj, &spend, result, &kwargs))
}

// Batch is the route substring, and the reply builder counts the joined text with the cl100k engine.
fn build_reply(
    request_body: Option<&Json>,
    url_route: &str,
    body: &Json,
    model: &str,
    counter: &Cl100kCounter,
) -> Result<EmbeddingReply, VertexPredictError> {
    let batch = url_route.contains(BATCH_ROUTE);
    let embedding_input = EmbeddingInput::Text(extract_input(request_body, batch));
    if batch {
        process_batch_reply(&embedding_input, model, body, counter)
            .map_err(VertexPredictError::from)
    } else {
        let files = ResolvedFiles::new();
        process_embed_content_reply(&embedding_input, model, body, &files, counter)
            .map_err(VertexPredictError::from)
    }
}
