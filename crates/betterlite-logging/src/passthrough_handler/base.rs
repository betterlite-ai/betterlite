use std::sync::Arc;

use betterlite_config::sections::request_headers::Settings;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::ports::{PassthroughLoggingProvider, ResponseCost};
use super::types::{
    CollectedChunksInput, MODEL, NO_COMPLETE_RESPONSE, PassthroughChatInput,
    PassthroughHandlerError, PassthroughLoggingPayload, ResponsePayloadInput, StreamedResponse,
};
use crate::end_user::extraction::{IdSettings, request_end_user_id};
use crate::logging_success::ports::StandardPayloadBuilder;
use crate::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use crate::passthrough_logging_state::response::PassThroughResponse;
use crate::passthrough_stream_logging::types::HandlerOutcome;

// LiteLLM's BasePassthroughLoggingHandler, with the provider's abstract members injected as ports
pub struct BasePassthroughLoggingHandler {
    pub provider: Arc<dyn PassthroughLoggingProvider>,
    pub cost: Arc<dyn ResponseCost>,
    pub payload_builder: Arc<dyn StandardPayloadBuilder>,
    pub request_headers: Settings,
    pub validate_end_user_id_in_db: bool,
}

impl BasePassthroughLoggingHandler {
    #[must_use]
    pub fn passthrough_chat_handler(
        &self,
        input: PassthroughChatInput<'_>,
    ) -> Result<HandlerOutcome, PassthroughHandlerError> {
        let PassthroughChatInput {
            upstream,
            response_body,
            logging_obj,
            request_body,
            start_time,
            end_time,
            kwargs,
        } = input;
        let model = request_model(request_body, Some(response_body))?;
        let config = self.provider.get_provider_config(&model);
        let mut response = config.transform_response(upstream, &model, logging_obj)?;
        let kwargs = self.build_response_logging_payload(ResponsePayloadInput {
            response: &mut response,
            model: &model,
            kwargs,
            start_time,
            end_time,
            logging_obj,
        });
        Ok(HandlerOutcome {
            result: Some(PassThroughResponse::ModelResponse(response)),
            kwargs,
        })
    }

    #[must_use]
    pub fn get_user_from_metadata(&self, payload: &PassthroughLoggingPayload) -> Option<String> {
        let request_body = payload.request_body.as_ref()?;
        request_end_user_id(
            request_body,
            None,
            &IdSettings {
                request_headers: &self.request_headers,
                validate_end_user_id_in_db: self.validate_end_user_id_in_db,
            },
        )
    }

    #[must_use]
    pub fn handle_collected_chunks(
        &self,
        input: CollectedChunksInput<'_>,
    ) -> Result<HandlerOutcome, PassthroughHandlerError> {
        let CollectedChunksInput {
            logging_obj,
            request_body,
            all_chunks,
            start_time,
            end_time,
        } = input;
        let model = request_model(request_body, None)?;
        let built = self
            .provider
            .build_streaming_response(all_chunks, logging_obj, &model)?;
        let Some(mut complete) = built else {
            tracing::error!("{NO_COMPLETE_RESPONSE}");
            return Ok(HandlerOutcome {
                result: None,
                kwargs: PassThroughLoggingKwargs::default(),
            });
        };
        let kwargs = match &mut complete {
            StreamedResponse::Model(json) | StreamedResponse::TextCompletion(json) => self
                .build_response_logging_payload(ResponsePayloadInput {
                    response: json,
                    model: &model,
                    kwargs: PassThroughLoggingKwargs::default(),
                    start_time,
                    end_time,
                    logging_obj,
                }),
        };
        let result = match complete {
            StreamedResponse::Model(json) => PassThroughResponse::ModelResponse(json),
            StreamedResponse::TextCompletion(json) => PassThroughResponse::TextCompletion(json),
        };
        Ok(HandlerOutcome {
            result: Some(result),
            kwargs,
        })
    }
}

// request_body.get("model", response_body.get("model", "")); a null takes the default (docs/compatibility.md)
fn request_model(
    request_body: &JsonObject,
    response_body: Option<&JsonObject>,
) -> Result<String, PassthroughHandlerError> {
    match request_body.get(MODEL) {
        Some(Json::Null) | None => response_model(response_body),
        Some(value) => model_text(value, "request_body.model"),
    }
}

fn response_model(response_body: Option<&JsonObject>) -> Result<String, PassthroughHandlerError> {
    match response_body.and_then(|body| body.get(MODEL)) {
        Some(Json::Null) | None => Ok(String::new()),
        Some(value) => model_text(value, "response_body.model"),
    }
}

fn model_text(value: &Json, field: &'static str) -> Result<String, PassthroughHandlerError> {
    match value {
        Json::Text(model) => Ok(model.clone()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            Err(PassthroughHandlerError::ModelType { field })
        }
    }
}
