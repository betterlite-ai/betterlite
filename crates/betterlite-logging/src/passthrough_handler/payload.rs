use std::str::FromStr as _;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::Number;

use super::base::BasePassthroughLoggingHandler;
use super::types::{
    BODY, LITELLM_PARAMS, MODEL, PASSTHROUGH_LOGGING_PAYLOAD, PAYLOAD_ERROR, PROXY_SERVER_REQUEST,
    PassthroughHandlerError, PassthroughLoggingPayload, RESPONSE_COST, ResponsePayloadInput, USER,
};
use crate::fields::payload::StandardLoggingPayload;
use crate::logging_success::payload_state::STANDARD_LOGGING_OBJECT;
use crate::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use crate::passthrough_stream_logging::object::StreamLoggingObject;

impl BasePassthroughLoggingHandler {
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
            state.model = model.to_owned();
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

fn cost_number(cost: Decimal) -> Result<Json, PassthroughHandlerError> {
    Number::from_str(&cost.to_string())
        .map(Json::Number)
        .map_err(|source| PassthroughHandlerError::CostNumber { source })
}

fn passthrough_payload(value: &Json) -> Result<PassthroughLoggingPayload, PassthroughHandlerError> {
    let text = serde_json::to_string(value)
        .map_err(|source| PassthroughHandlerError::PassthroughPayloadJson { source })?;
    let mut decoder = serde_json::Deserializer::from_str(&text);
    serde_path_to_error::deserialize(&mut decoder)
        .map_err(|source| PassthroughHandlerError::PassthroughPayload { source })
}

fn write_end_user(
    kwargs: &mut PassThroughLoggingKwargs,
    logging_obj: &StreamLoggingObject,
    user: &str,
) -> Result<(), PassthroughHandlerError> {
    let request = Json::Object(IndexMap::from([(
        BODY.to_owned(),
        Json::Object(IndexMap::from([(
            USER.to_owned(),
            Json::Text(user.to_owned()),
        )])),
    )]));
    let params = kwargs
        .entries
        .entry(LITELLM_PARAMS.to_owned())
        .or_insert_with(|| Json::Object(IndexMap::new()));
    // a null takes the default object, as an absent key does (docs/compatibility.md)
    if matches!(*params, Json::Null) {
        *params = Json::Object(IndexMap::new());
    }
    let Json::Object(fields) = params else {
        return Err(PassthroughHandlerError::LitellmParamsShape);
    };
    fields.insert(PROXY_SERVER_REQUEST.to_owned(), request.clone());
    logging_obj.edit(&mut |state| {
        state
            .litellm_params
            .insert(PROXY_SERVER_REQUEST.to_owned(), request.clone());
    });
    Ok(())
}

fn standard_logging_object_json(
    payload: &StandardLoggingPayload,
) -> Result<Json, PassthroughHandlerError> {
    let text = serde_json::to_string(payload)
        .map_err(|source| PassthroughHandlerError::StandardLoggingJson { source })?;
    serde_json::from_str(&text)
        .map_err(|source| PassthroughHandlerError::StandardLoggingJson { source })
}

fn stamp_response_identity(
    input: &mut ResponsePayloadInput<'_>,
) -> Result<(), PassthroughHandlerError> {
    let call_id = input
        .logging_obj
        .read(&mut |state| state.litellm_call_id.clone())
        .ok_or(PassthroughHandlerError::StateUnavailable)?;
    let model = input.model.to_owned();
    let logging_obj = input.logging_obj;
    let Json::Object(fields) = &mut *input.response else {
        return Err(PassthroughHandlerError::ResponseShape);
    };
    fields.insert("id".to_owned(), Json::Text(call_id));
    fields.insert(MODEL.to_owned(), Json::Text(model.clone()));
    logging_obj.edit(&mut |state| {
        state
            .model_call_details
            .insert(MODEL.to_owned(), Json::Text(model.clone()));
    });
    Ok(())
}
