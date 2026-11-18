use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_core::api::media_relay_logging::spend::MediaSpend;
use betterlite_core::routes::CallType;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_logging::passthrough_stream_logging::types::{
    CollectedChunksCall, CollectedChunksHandler, HandlerError, HandlerOutcome,
};
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
use super::cost::{CostRequest, CostResponse, FactoryCost, completion_body};
use super::payload::{create_gemini_response_logging_payload, request_object, video_body};
use super::relay::{relay_kwargs, relay_logging, write_relay_logging};
use super::reply;
use super::{GeminiPassthroughError, GeminiPassthroughInput, GeminiPassthroughLogger};
use crate::media_relay_logging::write_spend;

impl GeminiPassthroughLogger {
    // The interactions branch comes first, so a path ending in /interactions never reaches a decoder.
    pub fn gemini_passthrough_handler(
        &self,
        input: GeminiPassthroughInput<'_>,
    ) -> Result<HandlerOutcome, GeminiPassthroughError> {
        if is_interactions_route(input.url_route) {
            return self.interactions(input);
        }
        if input.url_route.contains("predictLongRunning") {
            return self.predict_long_running(input);
        }
        if input.url_route.contains("generateContent") {
            return self.generate_content(input);
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
        let request_model = input
            .request_body
            .get("model")
            .and_then(serde_json::Value::as_str)
            .filter(|model| !model.is_empty());
        let mut relay = relay_logging(input.logging_obj);
        let outcome = interactions_handler(
            &FactoryCost(self.cost.as_ref()),
            InteractionsPassthroughInput {
                response_body: input.response_body,
                request_model,
                logging: &mut relay,
                kwargs: ResponseLoggingKwargs::default(),
                custom_llm_provider: CustomLlmProvider::Gemini,
                vertex_location: None,
            },
        )?;
        write_relay_logging(input.logging_obj, &relay);
        let Some(response) = &outcome.result else {
            return Ok(HandlerOutcome {
                result: None,
                kwargs: input.kwargs,
            });
        };
        Ok(HandlerOutcome {
            result: Some(PassThroughResponse::ModelResponse(completion_body(
                response,
            ))),
            kwargs: relay_kwargs(&input.kwargs, outcome.kwargs),
        })
    }

    fn predict_long_running(
        &self,
        input: GeminiPassthroughInput<'_>,
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

    fn generate_content(
        &self,
        input: GeminiPassthroughInput<'_>,
    ) -> Result<HandlerOutcome, GeminiPassthroughError> {
        let model = extract_model_from_url(input.url_route);
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
        let kwargs = create_gemini_response_logging_payload(
            self.cost.as_ref(),
            &mut body,
            &model,
            input.kwargs,
            input.logging_obj,
            "gemini",
        )?;
        Ok(HandlerOutcome {
            result: Some(PassThroughResponse::ModelResponse(body)),
            kwargs,
        })
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
            assembly::complete_response(chunks, &model, now).map_err(HandlerError::leaf)?
        else {
            tracing::error!(
                "Unable to build complete streaming response for Gemini passthrough endpoint, not logging..."
            );
            return Ok(HandlerOutcome {
                result: None,
                kwargs: PassThroughLoggingKwargs::default(),
            });
        };
        let kwargs = create_gemini_response_logging_payload(
            self.cost.as_ref(),
            &mut body,
            &model,
            PassThroughLoggingKwargs::default(),
            call.logging_obj,
            "gemini",
        )
        .map_err(HandlerError::leaf)?;
        Ok(HandlerOutcome {
            result: Some(PassThroughResponse::ModelResponse(body)),
            kwargs,
        })
    }
}
