mod assembly;
mod lines;
mod output;
mod runtime;
mod tests;
mod usage;

use betterlite_anthropic::stream_events::error::StreamEventError;
use betterlite_logging::logging_failure::partial_usage;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_streaming::passthrough_reconstruction::error::ReconstructionError;
use betterlite_streaming::passthrough_reconstruction::events::RawSseChunk;
use betterlite_streaming::passthrough_reconstruction::usage::{
    UsageOnlyRequest, build_usage_only_response,
};
use betterlite_streaming::stream_usage_assembly::UsageTokenCounter;
use betterlite_streaming::streamed_response_assembly::response::Response;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::engine::error::EngineError;
use thiserror::Error;
use uuid::Uuid;

use self::assembly::{AnthropicUsageCalculator, raw_chunk};
use self::runtime::{block_on, unix_seconds};
use self::usage::usage_of;
use super::response_costing::{
    ResponseCostError, ResponseCostService, cost_partial_stream_or_zero, cost_relevant_speed,
};
use super::stream_fields::{Chunk, StreamFieldError, resolve_logged_model};

// The event conversion fails either on a shape the Anthropic converter rejects or on the serde hand-off.
#[derive(Debug, Error)]
pub enum ConvertFailure {
    #[error(transparent)]
    Event(#[from] StreamEventError<EngineError>),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Error)]
pub enum InterruptedStreamUsageError {
    #[error(transparent)]
    Utf8(#[from] StreamFieldError),
    #[error("the buffered stream could not be reconstructed")]
    Reconstruction {
        #[source]
        source: Box<ReconstructionError<ConvertFailure>>,
    },
    #[error("the interrupted stream output could not be counted")]
    TokenCount {
        #[source]
        source: CountError,
    },
    #[error("the partial stream usage could not be costed")]
    Cost {
        #[source]
        source: ResponseCostError,
    },
}

// The collaborators the failure log passes in: B3630's cost service and B196's token count.
pub struct InterruptedStreamServices<'services, Costs, Counter>
where
    Costs: ResponseCostService,
    Counter: UsageTokenCounter,
{
    pub costs: &'services Costs,
    pub counter: &'services Counter,
}

// LiteLLM's _stream_was_interrupted: the first deciding event from the tail answers for the whole stream.
pub(crate) fn stream_was_interrupted(
    all_chunks: &[Chunk],
) -> Result<bool, InterruptedStreamUsageError> {
    for (index, chunk) in all_chunks.iter().enumerate().rev() {
        let text = lines::chunk_text(chunk, index)?;
        for line in lines::split_lines(text).into_iter().rev() {
            if let Some(interrupted) = lines::tail_decision(line) {
                return Ok(interrupted);
            }
        }
    }
    Ok(true)
}

// ast-grep-ignore: no-long-names - LiteLLM's _recover_interrupted_stream_output_tokens re-counts the buffered text of a cut stream.
pub(crate) fn recover_interrupted_stream_output_tokens<Counter>(
    response: &mut Response,
    all_chunks: &[Chunk],
    model: &str,
    counter: &Counter,
) -> Result<bool, InterruptedStreamUsageError>
where
    Counter: UsageTokenCounter,
{
    let Response::Chat(chat) = response else {
        return Ok(false);
    };
    if !stream_was_interrupted(all_chunks)? {
        return Ok(false);
    }
    let output = output::completion_output(&chat.choices).map_err(encode_failure)?;
    if output.is_empty() {
        return Ok(false);
    }
    let Some(recovered) = re_tokenized(counter, model, &output) else {
        return Ok(false);
    };
    if recovered <= chat.usage.completion_tokens {
        return Ok(false);
    }
    chat.usage.completion_tokens = recovered;
    chat.usage.total_tokens = chat.usage.prompt_tokens.saturating_add(recovered);
    // Anthropic costing reads text_tokens, so the message_start placeholder there is corrected too.
    if let Some(details) = chat.usage.completion_tokens_details.as_mut()
        && details.text_tokens.is_some()
    {
        details.text_tokens = Some(recovered);
    }
    Ok(true)
}

// A count failure keeps the placeholder instead of failing the caller, as LiteLLM's bare except does.
fn re_tokenized<Counter>(counter: &Counter, model: &str, output: &str) -> Option<i64>
where
    Counter: UsageTokenCounter,
{
    if let Ok(Ok(count)) = block_on(counter.count_response_text(model, output)) {
        Some(count)
    } else {
        tracing::warn!(
            "Could not re-tokenize interrupted stream output; keeping placeholder completion token count."
        );
        None
    }
}

// LiteLLM's _clear_placeholder_cost: the recovered tokens are priced again by the failure log.
pub(crate) fn clear_placeholder_cost(response: &mut Response) {
    let Response::Chat(chat) = response else {
        return;
    };
    chat.usage.cost = None;
    let _removed = chat.hidden_params.remove("response_cost");
}

// LiteLLM's _usage_only_response_or_none: a usage-only response, or none after a warning.
pub(crate) fn usage_only_response_or_none(
    all_chunks: &[Chunk],
    model: &str,
    speed: Option<&str>,
) -> Option<Response> {
    let chunks: Vec<RawSseChunk> = all_chunks.iter().map(raw_chunk).collect();
    let calculator = AnthropicUsageCalculator;
    let request = UsageOnlyRequest {
        chunks: &chunks,
        model,
        speed,
        calculator: &calculator,
        now: unix_seconds(),
        next_id: Uuid::new_v4,
    };
    match build_usage_only_response(&request) {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(
                "Anthropic passthrough: usage-only fallback failed (model={model}): {error}"
            );
            None
        }
    }
}

// LiteLLM's _assemble_streaming_response: an empty or failed reconstruction falls back to usage-only.
pub(crate) fn assemble_streaming_response<Costs, Counter>(
    all_chunks: &[Chunk],
    call_log: &LoggingState,
    model: &str,
    speed: Option<&str>,
    services: &InterruptedStreamServices<'_, Costs, Counter>,
) -> Option<Response>
where
    Costs: ResponseCostService,
    Counter: UsageTokenCounter,
{
    match assembly::run(all_chunks, call_log, model, speed, services) {
        Ok(Some(response)) => Some(response),
        Ok(None) => usage_only_response_or_none(all_chunks, model, speed),
        Err(error) => {
            tracing::warn!(
                "Anthropic passthrough: stream assembly raised (model={model}): {error}; falling back to usage-only cost from raw SSE events."
            );
            usage_only_response_or_none(all_chunks, model, speed)
        }
    }
}

// ast-grep-ignore: no-long-names - LiteLLM's _build_streaming_response_for_logging drops the placeholder cost only after a recovery.
pub fn build_streaming_response_for_logging<Costs, Counter>(
    call_log: &LoggingState,
    request_body: &JsonObject,
    all_chunks: &[Chunk],
    model: &str,
    services: &InterruptedStreamServices<'_, Costs, Counter>,
) -> Result<Option<Response>, InterruptedStreamUsageError>
where
    Costs: ResponseCostService,
    Counter: UsageTokenCounter,
{
    let speed = cost_relevant_speed(Some(request_body));
    let Some(mut response) =
        assemble_streaming_response(all_chunks, call_log, model, speed.as_deref(), services)
    else {
        return Ok(None);
    };
    // ast-grep-ignore: no-long-names
    let recovered = recover_interrupted_stream_output_tokens(
        &mut response,
        all_chunks,
        model,
        services.counter,
    )?;
    if recovered {
        clear_placeholder_cost(&mut response);
    }
    Ok(Some(response))
}

// ast-grep-ignore: no-long-names - LiteLLM's record_partial_usage_for_failure bills the recovered usage and its cost on the failure row.
pub fn record_partial_usage_for_failure<Costs, Counter>(
    call_log: &mut LoggingState,
    request_body: &JsonObject,
    all_chunks: &[Chunk],
    services: &InterruptedStreamServices<'_, Costs, Counter>,
) -> Result<(), InterruptedStreamUsageError>
where
    Costs: ResponseCostService,
    Counter: UsageTokenCounter,
{
    if all_chunks.is_empty() {
        return Ok(());
    }
    let model = resolve_logged_model(call_log, request_body, all_chunks)?;
    let Some(response) =
        // ast-grep-ignore: no-long-names
        build_streaming_response_for_logging(call_log, request_body, all_chunks, &model, services)?
    else {
        return Ok(());
    };
    let usage = usage_of(&response);
    // The reconstructed response is what LiteLLM hands completion_cost, as the JSON B3630's service reads.
    let value = serde_json::to_value(&response).map_err(encode_failure)?;
    let partial: Json = serde_json::from_value(value).map_err(encode_failure)?;
    let cost = cost_partial_stream_or_zero(services.costs, &partial, &model, call_log);
    // ast-grep-ignore: no-long-names
    partial_usage::record_partial_usage_for_failure(call_log, &usage, cost);
    Ok(())
}

// A response that cannot round-trip through JSON fails the reconstruction it came from.
fn encode_failure(source: serde_json::Error) -> InterruptedStreamUsageError {
    InterruptedStreamUsageError::Reconstruction {
        source: Box::new(ReconstructionError::Encode(source)),
    }
}

#[cfg(any(test, feature = "testing"))]
pub mod testing;
