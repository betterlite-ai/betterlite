use std::sync::Arc;

use betterlite_config::sections::request_headers::Settings;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::payload::{
    cost_number, passthrough_payload, stamp_response_identity, standard_logging_object_json,
    write_end_user,
};
use super::ports::{PassthroughLoggingProvider, ResponseCost};
use super::types::{
    CollectedChunksInput, MODEL, NO_COMPLETE_RESPONSE, PASSTHROUGH_LOGGING_PAYLOAD, PAYLOAD_ERROR,
    PassthroughChatInput, PassthroughHandlerError, PassthroughLoggingPayload, RESPONSE_COST,
    ResponsePayloadInput, StreamedResponse,
};
use crate::end_user::extraction::{IdSettings, request_end_user_id};
use crate::logging_success::payload_state::STANDARD_LOGGING_OBJECT;
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
        let logged_kwargs = self.build_response_logging_payload(ResponsePayloadInput {
            response: &mut response,
            model: &model,
            kwargs,
            start_time,
            end_time,
            logging_obj,
        });
        Ok(HandlerOutcome {
            result: Some(PassThroughResponse::ModelResponse(response)),
            kwargs: logged_kwargs,
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

    pub fn handle_collected_chunks(
        &self,
        input: &CollectedChunksInput<'_>,
    ) -> Result<HandlerOutcome, PassthroughHandlerError> {
        let model = request_model(input.request_body, None)?;
        let built =
            self.provider
                .build_streaming_response(input.all_chunks, input.logging_obj, &model)?;
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
                    start_time: input.start_time,
                    end_time: input.end_time,
                    logging_obj: input.logging_obj,
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

    #[must_use]
    pub fn build_response_logging_payload(
        &self,
        mut input: ResponsePayloadInput<'_>,
    ) -> PassThroughLoggingKwargs {
        if let Err(error) = self.fill_response_logging_payload(&mut input) {
            tracing::error!("{PAYLOAD_ERROR}: {error}");
        }
        input.kwargs
    }

    fn fill_response_logging_payload(
        &self,
        input: &mut ResponsePayloadInput<'_>,
    ) -> Result<(), PassthroughHandlerError> {
        self.record_cost(input)?;
        self.record_end_user(input)?;
        self.record_standard_logging_object(input)?;
        stamp_response_identity(input)
    }

    fn record_cost(
        &self,
        input: &mut ResponsePayloadInput<'_>,
    ) -> Result<(), PassthroughHandlerError> {
        let cost = self.cost.completion_cost(input.response, input.model)?;
        let number = cost_number(cost)?;
        input
            .kwargs
            .entries
            .insert(RESPONSE_COST.to_owned(), number.clone());
        input
            .kwargs
            .entries
            .insert(MODEL.to_owned(), Json::Text(input.model.to_owned()));
        let model = input.model;
        let logging_obj = input.logging_obj;
        logging_obj.edit(&mut |state| {
            state
                .model_call_details
                .insert(RESPONSE_COST.to_owned(), number.clone());
            model.clone_into(&mut state.model);
        });
        Ok(())
    }

    fn record_end_user(
        &self,
        input: &mut ResponsePayloadInput<'_>,
    ) -> Result<(), PassthroughHandlerError> {
        let Some(value) = input.kwargs.entries.get(PASSTHROUGH_LOGGING_PAYLOAD) else {
            return Ok(());
        };
        if matches!(value, Json::Null) {
            return Ok(());
        }
        let payload = passthrough_payload(value)?;
        let Some(user) = self
            .get_user_from_metadata(&payload)
            .filter(|user| !user.is_empty())
        else {
            return Ok(());
        };
        write_end_user(&mut input.kwargs, input.logging_obj, &user)
    }

    fn record_standard_logging_object(
        &self,
        input: &mut ResponsePayloadInput<'_>,
    ) -> Result<(), PassthroughHandlerError> {
        let logging_obj = input.logging_obj;
        let response: &Json = input.response;
        let start_time = input.start_time;
        let end_time = input.end_time;
        let builder = self.payload_builder.as_ref();
        let built = logging_obj
            .read(&mut |state| builder.build(state, response, start_time, end_time))
            .ok_or(PassthroughHandlerError::StateUnavailable)?;
        let object = match built {
            Some(payload) => standard_logging_object_json(&payload)?,
            None => Json::Null,
        };
        let text = serde_json::to_string_pretty(&object)
            .map_err(|source| PassthroughHandlerError::StandardLoggingJson { source })?;
        tracing::debug!("standard_logging_object= {text}");
        input
            .kwargs
            .entries
            .insert(STANDARD_LOGGING_OBJECT.to_owned(), object);
        Ok(())
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
