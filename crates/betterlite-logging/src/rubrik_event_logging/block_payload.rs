use betterlite_models::decisions::Json;
use betterlite_models::projections::parameter_names::logging_parameter_names;
use chrono::DateTime;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde_json::Number;
use uuid::Uuid;

use crate::logging_success::payload_state::STANDARD_LOGGING_OBJECT;
use crate::rubrik::request_data::correlation_id;
use crate::rubrik::system_prompt::prepend_system_prompt;
use crate::rubrik_moderation::context::text_field;
use crate::rubrik_moderation::error::ModifyResponseException;

use super::error::EventPayloadError;
use super::fields::field_object;

pub fn prepare_block_failure_payload(
    call_details: &IndexMap<String, Json>,
    exception: &ModifyResponseException,
    caller_metadata: &IndexMap<String, Json>,
) -> Result<IndexMap<String, Json>, EventPayloadError> {
    // A streamed block can arrive with the object already set; a prompt block never has it.
    let mut payload = if let Some(base) = field_object(call_details, STANDARD_LOGGING_OBJECT)? {
        base.clone()
    } else {
        let call_id = text_field(call_details, "litellm_call_id")
            .ok()
            .flatten()
            .unwrap_or("None");
        tracing::debug!(
            litellm_call_id = call_id,
            "Rubrik: standard_logging_object not yet on model_call_details; using the fallback payload"
        );
        build_fallback_payload(call_details, caller_metadata)?
    };
    payload.insert(
        "response".to_owned(),
        Json::Text(format!("ModifyResponseException: {}", exception.message)),
    );
    // The copied object's id is the provider response id, which would not correlate with the block log.
    let call_id = correlation_id(text_field(call_details, "litellm_call_id")?, None)
        .unwrap_or_else(|| format!("chatcmpl-{}", Uuid::new_v4()));
    payload.insert("id".to_owned(), Json::Text(call_id));
    if let Some(messages) =
        prepend_system_prompt(payload.get("messages"), call_details.get("system"))
    {
        payload.insert("messages".to_owned(), Json::List(messages));
    }
    Ok(payload)
}

pub fn build_fallback_payload(
    call_details: &IndexMap<String, Json>,
    caller_metadata: &IndexMap<String, Json>,
) -> Result<IndexMap<String, Json>, EventPayloadError> {
    let model = text_field(call_details, "model")?.unwrap_or("").to_owned();
    let start = unix_start_time(call_details)?;
    let mut payload = IndexMap::new();
    payload.insert("model".to_owned(), Json::Text(model.clone()));
    // The router names model_group inside acompletion, which a prompt block never reaches.
    payload.insert("model_group".to_owned(), Json::Text(model));
    payload.insert("model_id".to_owned(), Json::Text(String::new()));
    payload.insert(
        "model_parameters".to_owned(),
        Json::Object(model_parameters(call_details)?),
    );
    payload.insert("startTime".to_owned(), start.clone());
    payload.insert("endTime".to_owned(), start.clone());
    payload.insert("completionStartTime".to_owned(), start);
    payload.insert("messages".to_owned(), fallback_messages(call_details));
    payload.insert("metadata".to_owned(), Json::Object(caller_metadata.clone()));
    payload.insert("status".to_owned(), Json::Text("failure".to_owned()));
    Ok(payload)
}

fn unix_start_time(call_details: &IndexMap<String, Json>) -> Result<Json, EventPayloadError> {
    let Some(text) = text_field(call_details, "start_time")? else {
        return Ok(Json::Null);
    };
    let moment = DateTime::parse_from_rfc3339(text)
        .map_err(|source| EventPayloadError::StartTime { source })?;
    let seconds = Decimal::from(moment.timestamp());
    let nanos = Decimal::new(i64::from(moment.timestamp_subsec_nanos()), 9);
    let unix = seconds
        .checked_add(nanos)
        .and_then(|total| total.to_f64())
        .and_then(Number::from_f64)
        .map_or(Json::Null, Json::Number);
    Ok(unix)
}

fn model_parameters(
    call_details: &IndexMap<String, Json>,
) -> Result<IndexMap<String, Json>, EventPayloadError> {
    let Some(optional_params) = field_object(call_details, "optional_params")? else {
        return Ok(IndexMap::new());
    };
    let allowed = logging_parameter_names();
    Ok(optional_params
        .iter()
        .filter(|(name, _)| allowed.contains(name.as_str()))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect())
}

fn fallback_messages(call_details: &IndexMap<String, Json>) -> Json {
    match call_details.get("messages") {
        None | Some(Json::Null) => Json::List(Vec::new()),
        Some(messages) => messages.clone(),
    }
}
