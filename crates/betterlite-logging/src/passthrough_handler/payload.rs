use std::str::FromStr as _;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::Number;

use super::types::{
    BODY, LITELLM_PARAMS, MODEL, PROXY_SERVER_REQUEST, PassthroughHandlerError,
    PassthroughLoggingPayload, ResponsePayloadInput, USER,
};
use crate::fields::payload::StandardLoggingPayload;
use crate::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use crate::passthrough_stream_logging::object::StreamLoggingObject;

pub(super) fn cost_number(cost: Decimal) -> Result<Json, PassthroughHandlerError> {
    Number::from_str(&cost.to_string())
        .map(Json::Number)
        .map_err(|source| PassthroughHandlerError::CostNumber { source })
}

pub(super) fn passthrough_payload(
    value: &Json,
) -> Result<PassthroughLoggingPayload, PassthroughHandlerError> {
    let text = serde_json::to_string(value)
        .map_err(|source| PassthroughHandlerError::PassthroughPayloadJson { source })?;
    let mut decoder = serde_json::Deserializer::from_str(&text);
    serde_path_to_error::deserialize(&mut decoder)
        .map_err(|source| PassthroughHandlerError::PassthroughPayload { source })
}

pub(super) fn write_end_user(
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

pub(super) fn standard_logging_object_json(
    payload: &StandardLoggingPayload,
) -> Result<Json, PassthroughHandlerError> {
    let text = serde_json::to_string(payload)
        .map_err(|source| PassthroughHandlerError::StandardLoggingJson { source })?;
    serde_json::from_str(&text)
        .map_err(|source| PassthroughHandlerError::StandardLoggingJson { source })
}

pub(super) fn stamp_response_identity(
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
