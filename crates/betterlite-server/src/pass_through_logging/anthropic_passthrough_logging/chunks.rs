use betterlite_core::cost::recording::PortFailure;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_logging::passthrough_stream_logging::types::{
    CollectedChunksCall, CollectedChunksHandler, HandlerError, HandlerOutcome,
};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_streaming::streamed_response_assembly::response::Response;
use rust_decimal::Decimal;

use super::super::interrupted_stream_usage::InterruptedStreamServices;
// ast-grep-ignore: no-long-names - B3632 owns this name and the stream seam calls it here
use super::super::interrupted_stream_usage::build_streaming_response_for_logging;
use super::super::response_costing::{ResponseCostRequest, ResponseCostService};
use super::super::stream_fields::{Chunk, extract_message_start_field, resolve_logged_model};
use super::payload::AnthropicPayloadInput;
use super::{AnthropicPassthroughError, AnthropicPassthroughLogger, logged_json};

// B3632's services take a sized cost service, so the shared trait object is lent through this view.
struct LentCost<'cost>(&'cost dyn ResponseCostService);

impl ResponseCostService for LentCost<'_> {
    fn completion_cost(&self, request: &ResponseCostRequest<'_>) -> Result<Decimal, PortFailure> {
        self.0.completion_cost(request)
    }
}

// The stream seam calls this once a stream ends; the call log lives in the logging object's slot.
impl CollectedChunksHandler for AnthropicPassthroughLogger {
    fn handle(&self, call: &CollectedChunksCall<'_>) -> Result<HandlerOutcome, HandlerError> {
        let all_chunks: Vec<Chunk> = call.lines.iter().cloned().map(Chunk::Text).collect();
        let empty = JsonObject::new();
        let request_body = match call.request_body {
            Some(Json::Object(body)) => body,
            None
            | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
                &empty
            }
        };
        let mut outcome = None;
        call.logging_obj.edit(&mut |call_log| {
            outcome = Some(self.log_collected_chunks(call_log, request_body, &all_chunks));
        });
        let logged = outcome.ok_or(AnthropicPassthroughError::CallLogHeld)?;
        logged.map_err(HandlerError::from)
    }
}

impl AnthropicPassthroughLogger {
    // _handle_logging_anthropic_collected_chunks; LiteLLM's endpoint type and success handler are never read.
    pub(super) fn log_collected_chunks(
        &self,
        call_log: &mut LoggingState,
        request_body: &JsonObject,
        all_chunks: &[Chunk],
    ) -> Result<HandlerOutcome, AnthropicPassthroughError> {
        let model = resolve_logged_model(call_log, request_body, all_chunks)?;
        let tools = self
            .tools
            .as_ref()
            .ok_or(AnthropicPassthroughError::Tools)?;
        let costs = LentCost(self.cost.as_ref());
        let services = InterruptedStreamServices {
            costs: &costs,
            counter: &tools.counter,
        };
        // ast-grep-ignore: no-long-names
        let built = build_streaming_response_for_logging(
            call_log,
            request_body,
            all_chunks,
            &model,
            &services,
        )?;
        let Some(response) = built else {
            tracing::error!(
                "Unable to build complete streaming response for Anthropic passthrough endpoint, not logging..."
            );
            return Ok(HandlerOutcome::default());
        };
        let mut body = logged_json(&response)?;
        let response_id = extract_message_start_field(all_chunks, "id")?;
        let kwargs = self.create_anthropic_response_logging_payload(AnthropicPayloadInput {
            response: &mut body,
            model: &model,
            kwargs: PassThroughLoggingKwargs::default(),
            call_log,
            response_id: response_id.as_deref(),
        });
        let result = match response {
            Response::Chat(_) => PassThroughResponse::ModelResponse(body),
            Response::Text(_) => PassThroughResponse::TextCompletion(body),
        };
        Ok(HandlerOutcome {
            result: Some(result),
            kwargs,
        })
    }
}
