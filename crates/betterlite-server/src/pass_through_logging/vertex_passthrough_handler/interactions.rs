use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_vertex::vertex_relay_metadata::interactions::{
    InteractionsPassthroughInput, InteractionsPassthroughOutcome, interactions_handler,
};
use betterlite_vertex::vertex_relay_metadata::types::{CustomLlmProvider, ResponseLoggingKwargs};

use super::{VertexPassthroughError, VertexPassthroughInput};
use crate::pass_through_logging::gemini_passthrough_logging::cost::{
    FactoryCost, PassthroughCompletionCost, completion_body,
};
use crate::pass_through_logging::gemini_passthrough_logging::relay::{RelayLogging, relay_kwargs};

pub(super) fn interactions(
    cost: &dyn PassthroughCompletionCost,
    input: VertexPassthroughInput<'_>,
    vertex_location: Option<&str>,
) -> Result<HandlerOutcome, VertexPassthroughError> {
    let request_model = input
        .request_body
        .and_then(|body| body.get("model"))
        .and_then(|value| value.as_str());
    let mut relay = RelayLogging::project(input.logging_obj);
    let outcome = interactions_handler(
        &FactoryCost(cost),
        InteractionsPassthroughInput {
            response_body: input.response_body,
            request_model,
            logging: &mut relay.logging,
            kwargs: ResponseLoggingKwargs::default(),
            custom_llm_provider: CustomLlmProvider::VertexAi,
            vertex_location,
        },
    )?;
    relay.commit();
    Ok(logged_outcome(outcome, input.kwargs))
}

fn logged_outcome(
    outcome: InteractionsPassthroughOutcome,
    kwargs: PassThroughLoggingKwargs,
) -> HandlerOutcome {
    let Some(response) = &outcome.result else {
        return HandlerOutcome {
            result: None,
            kwargs,
        };
    };
    HandlerOutcome {
        result: Some(PassThroughResponse::ModelResponse(completion_body(
            response,
        ))),
        kwargs: relay_kwargs(&kwargs, outcome.kwargs),
    }
}
