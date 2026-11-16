use betterlite_models::decisions::Json;
use betterlite_models::projections::parameter_names::{ModelParameters, logging_model_parameters};
use indexmap::IndexMap;
use serde_json::value::RawValue;

use crate::fields::messages::append_system_prompt_messages;
use crate::fields::response::{ExtractedResponse, HiddenParams, get_final_response_obj};
use crate::logging_state::LoggingState;
use crate::payload::azure::{get_model_router_selected_model, is_model_router_call};
use crate::payload::compose::inputs::optional_text;
use crate::payload::compose::{PayloadComposition, PayloadRequest};
use crate::payload::error::PayloadError;
use crate::payload::redaction::redacted_messages_json;
use crate::payload::response::{PayloadResponsePrivacy, response_model_name};

pub(crate) fn final_response(
    request: &PayloadRequest<'_>,
    extracted: &ExtractedResponse,
    redact: bool,
) -> Json {
    let privacy = PayloadResponsePrivacy::from_state(request.state, redact);
    get_final_response_obj(
        Some(&extracted.response_obj),
        request.init_response_obj,
        privacy.served_texts.as_ref(),
        &privacy,
    )
}

pub(crate) fn model_name(
    request: &PayloadRequest<'_>,
    extracted: &ExtractedResponse,
    base_model: &str,
    response: &Json,
) -> String {
    let hidden = raw_hidden_params(extracted);
    let requested = Some(request.state.model.as_str());
    if let Some(stamped) = get_model_router_selected_model(hidden.as_ref()) {
        return stamped;
    }
    if is_model_router_call(requested, hidden.as_ref())
        && let Some(model) = response_model_name(response)
    {
        return model.to_owned();
    }
    base_model.to_owned()
}

fn raw_hidden_params(extracted: &ExtractedResponse) -> Option<Json> {
    match extracted.hidden_params.as_ref() {
        Some(HiddenParams::FromObject(fields)) => Some(Json::Object(fields.clone())),
        Some(HiddenParams::FromException(_)) | None => None,
    }
}

pub(crate) fn logged_messages(
    request: &PayloadRequest<'_>,
    composition: &PayloadComposition<'_>,
    redact: bool,
) -> Result<Option<Json>, PayloadError> {
    if let Some(stored) = request.state.truncated_messages_for_logging.as_ref() {
        return Ok(Some(stored.clone()));
    }
    let system = optional_text(request.state.model_call_details.get("system"), "system")?;
    let messages = if redact {
        Some(redacted_messages_json())
    } else {
        request
            .state
            .messages
            .as_ref()
            .map(|messages| Json::List(messages.clone()))
    };
    let Some(combined) = append_system_prompt_messages(system.as_deref(), messages.as_ref()) else {
        return Ok(None);
    };
    composition
        .truncator
        .truncate_messages(&combined)
        .map(Some)
        .map_err(PayloadError::from)
}

pub(crate) fn model_parameters(
    state: &LoggingState,
) -> Result<IndexMap<String, Json>, PayloadError> {
    let parameters = state
        .optional_params
        .iter()
        .map(|(name, value)| raw_parameter(name, value))
        .collect::<Result<ModelParameters, PayloadError>>()?;
    logging_model_parameters(&parameters)
        .into_iter()
        .map(|(name, raw)| Ok((name, parse_parameter(&raw)?)))
        .collect()
}

fn raw_parameter(name: &str, value: &Json) -> Result<(String, Box<RawValue>), PayloadError> {
    let encoded = serde_json::to_string(value).map_err(|source| PayloadError::Json { source })?;
    let raw = RawValue::from_string(encoded).map_err(|source| PayloadError::Json { source })?;
    Ok((name.to_owned(), raw))
}

fn parse_parameter(raw: &RawValue) -> Result<Json, PayloadError> {
    serde_json::from_str(raw.get()).map_err(|source| PayloadError::Json { source })
}
