use std::sync::Arc;

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_core::api::media_relay_logging::spend::MediaSpend;
use betterlite_core::routes::CallType;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_logging::passthrough_stream_logging::types::{
    CollectedChunksCall, CollectedChunksHandler, HandlerError, HandlerOutcome,
};
use betterlite_models::decisions::Json;
use betterlite_vertex::gemini_response::complete::{
    CompleteRequest, RawResponse, transform_response,
};
use betterlite_vertex::gemini_response_parts::projection::CallMode;
use betterlite_vertex::studio_veo_operation::records::{VideoContext, create_response};
use betterlite_vertex::vertex_relay_metadata::interactions::{
    InteractionsPassthroughInput, interactions_handler,
};
use betterlite_vertex::vertex_relay_metadata::routes::is_interactions_route;
use betterlite_vertex::vertex_relay_metadata::types::{CustomLlmProvider, ResponseLoggingKwargs};
use betterlite_vertex::vertex_relay_metadata::url_models::extract_model_from_url;

use super::assembly;
use super::cost::{
    CostRequest, CostResponse, FactoryCost, PassthroughCompletionCost, completion_body,
};
use super::counter::CollectedCounter;
use super::payload::{GeminiPayload, gemini_log_payload, request_object, video_body};
use super::relay::{RelayLogging, relay_kwargs};
use super::reply;
use super::{GeminiPassthroughError, GeminiPassthroughInput, GeminiPassthroughLogger};
use crate::media_relay_logging::write_spend;

impl GeminiPassthroughLogger {
    pub fn new(cost: Arc<dyn PassthroughCompletionCost>) -> Self {
        Self {
            cost,
            counter: CollectedCounter::bundled(),
        }
    }

    // The interactions branch comes first, so a path ending in /interactions never reaches a decoder.
    pub fn gemini_passthrough_handler(
        &self,
        input: GeminiPassthroughInput<'_>,
    ) -> Result<HandlerOutcome, GeminiPassthroughError> {
        if is_interactions_route(input.url_route) {
            return self.interactions(input);
        }
        if input.url_route.contains("predictLongRunning") {
            return self.predict_long_running(&input);
        }
        if input.url_route.contains("generateContent") {
            return generate_content(self.cost.as_ref(), input);
        }
        Ok(HandlerOutcome {
            result: None,
            kwargs: input.kwargs,
        })
    }

    fn interactions(
        &self,
        input: GeminiPassthroughInput<'_>,
    ) -> Result<HandlerOutcome, GeminiPassthroughError> {
        // interactions_handler drops an empty request model itself, so this only unwraps the text case.
        let request_model = match input.request_body.get("model") {
            Some(Json::Text(model)) => Some(model.as_str()),
            _other => None,
        };
        let mut relay = RelayLogging::project(input.logging_obj);
        let outcome = interactions_handler(
            &FactoryCost(self.cost.as_ref()),
            InteractionsPassthroughInput {
                response_body: input.response_body,
                request_model,
                logging: &mut relay.logging,
                kwargs: ResponseLoggingKwargs::default(),
                custom_llm_provider: CustomLlmProvider::Gemini,
                vertex_location: None,
            },
        )?;
        relay.commit();
        let Some(response) = &outcome.result else {
            return Ok(HandlerOutcome {
                result: None,
                kwargs: input.kwargs,
            });
        };
        Ok(model_response_outcome(
            completion_body(response),
            relay_kwargs(&input.kwargs, outcome.kwargs),
        ))
    }

    fn predict_long_running(
        &self,
        input: &GeminiPassthroughInput<'_>,
    ) -> Result<HandlerOutcome, GeminiPassthroughError> {
        let model = extract_model_from_url(input.url_route);
        let request_data = request_object(input.request_body)?;
        let settings = VertexSettings::default();
        let video = create_response(
            &model,
            input.response_body,
            &VideoContext {
                provider: Some("gemini"),
                request_data: Some(&request_data),
                settings: &settings,
            },
        )?;
        let response_cost = self
            .cost
            .completion_cost(&CostRequest {
                response: CostResponse::Video(&video),
                model: &model,
                custom_llm_provider: "gemini",
                vertex_location: None,
                call_type: CallType::CreateVideo,
            })
            .map_err(|source| GeminiPassthroughError::Cost {
                model: model.clone(),
                source,
            })?;
        let spend = MediaSpend {
            model,
            custom_llm_provider: "gemini",
            response_cost: Some(response_cost),
        };
        write_spend(input.logging_obj, &spend);
        input.logging_obj.edit(&mut |state| {
            state.custom_llm_provider = Some("gemini".to_owned());
        });
        Ok(HandlerOutcome {
            result: Some(PassThroughResponse::VideoOperation(video_body(&video)?)),
            kwargs: spend.kwargs(&input.kwargs),
        })
    }
}

// The generateContent branch decodes the buffered reply, logs and prices it, and returns the logged body.
fn generate_content(
    cost: &dyn PassthroughCompletionCost,
    input: GeminiPassthroughInput<'_>,
) -> Result<HandlerOutcome, GeminiPassthroughError> {
    let model = extract_model_from_url(input.url_route);
    // The body is the ModelResponse LiteLLM would have gotten from httpx_response.json().
    let mut ids = uuid::Uuid::new_v4;
    let response = transform_response(
        &RawResponse {
            headers: input.response_headers,
            body: input.response_body,
        },
        &CompleteRequest {
            model: &model,
            now: input.end_time.timestamp(),
            call_mode: CallMode::Tools,
        },
        &mut ids,
    )?;
    let mut body = reply::logged_response(&response, input.end_time.timestamp())
        .map_err(|source| GeminiPassthroughError::Encode { source })?;
    let kwargs = gemini_log_payload(
        cost,
        GeminiPayload {
            response: &mut body,
            model: &model,
            logging_obj: input.logging_obj,
            custom_llm_provider: "gemini",
        },
        input.kwargs,
    )?;
    Ok(model_response_outcome(body, kwargs))
}

// Every branch that logs a body returns it as the logged ModelResponse with its priced kwargs.
const fn model_response_outcome(body: Json, kwargs: PassThroughLoggingKwargs) -> HandlerOutcome {
    HandlerOutcome {
        result: Some(PassThroughResponse::ModelResponse(body)),
        kwargs,
    }
}

impl CollectedChunksHandler for GeminiPassthroughLogger {
    fn handle(&self, call: &CollectedChunksCall<'_>) -> Result<HandlerOutcome, HandlerError> {
        let model = call
            .model
            .filter(|name| !name.is_empty())
            .map_or_else(|| extract_model_from_url(call.url_route), str::to_owned);
        let now = call.end_time.timestamp();
        let chunks = if call.url_route.contains("generateContent")
            || call.url_route.contains("streamGenerateContent")
        {
            assembly::gemini_chunks(call.lines, now).map_err(HandlerError::leaf)?
        } else {
            Vec::new()
        };
        let Some(mut body) =
            assembly::complete_response(chunks, &model, now, self.counter.as_ref())
                .map_err(HandlerError::leaf)?
        else {
            tracing::error!(
                "Unable to build complete streaming response for Gemini passthrough endpoint, not logging..."
            );
            return Ok(HandlerOutcome {
                result: None,
                kwargs: PassThroughLoggingKwargs::default(),
            });
        };
        let kwargs = gemini_log_payload(
            self.cost.as_ref(),
            GeminiPayload {
                response: &mut body,
                model: &model,
                logging_obj: call.logging_obj,
                custom_llm_provider: "gemini",
            },
            PassThroughLoggingKwargs::default(),
        )
        .map_err(HandlerError::leaf)?;
        Ok(model_response_outcome(body, kwargs))
    }
}
