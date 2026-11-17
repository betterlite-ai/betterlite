use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::prelude::ToPrimitive as _;

use super::core::PointFiveCore;
use super::error::PointFiveError;
use crate::callback_logger::event::LoggerEvent;
use crate::exporter_options::redaction::{
    ModelCallDetails,
    Redaction,
    RedactionPolicy,
    ResponseStamp,
    // ast-grep-ignore: no-long-names
    redact_standard_logging_payload,
};
use crate::fields::payload::StandardLoggingPayload;
use crate::logging_failure::events::{map_to_payload, payload_to_map};
use crate::payload::redaction::redact_standard_payload;
use crate::spend_bodies::redaction::should_redact_message_logging;

// _record_for: the excluded fields and this callback's flag, then the framework's own predicate
pub(crate) fn record_for(
    core: &PointFiveCore,
    event: &LoggerEvent,
) -> Result<StandardLoggingPayload, PointFiveError> {
    let payload = redacted_for_flag(core, &event.payload)?;
    let metadata = IndexMap::from([(
        "litellm_metadata".to_owned(),
        Json::Object((*event.litellm_metadata).clone()),
    )]);
    let dynamic = match event.dynamic_params.as_ref() {
        Json::Object(fields) => Some(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    };
    if should_redact_message_logging(Some(&metadata), dynamic, core.global_message_redaction) {
        return Ok(redact_standard_payload(&payload));
    }
    Ok(payload)
}

// redact_standard_logging_payload_from_model_call_details: the callback's turn_off flag alone
fn redacted_for_flag(
    core: &PointFiveCore,
    payload: &StandardLoggingPayload,
) -> Result<StandardLoggingPayload, PointFiveError> {
    if !core.params.turn_off_message_logging {
        return Ok(payload.clone());
    }
    let details = ModelCallDetails {
        standard_logging_object: Some(payload_to_map(payload)?),
        proxy_server_request: None,
    };
    let policy = RedactionPolicy {
        turn_off_message_logging: true,
        excluded_fields: Vec::new(),
        callback_redacts_itself: false,
    };
    // ast-grep-ignore: no-long-names
    match redact_standard_logging_payload(&details, &policy, &response_stamp(payload)) {
        Redaction::Unchanged => Ok(payload.clone()),
        Redaction::Redacted {
            standard_logging_object,
            ..
        } => Ok(map_to_payload(&standard_logging_object)?),
    }
}

fn response_stamp(payload: &StandardLoggingPayload) -> ResponseStamp {
    let response = payload.response.as_ref().and_then(|value| match value {
        Json::Object(fields) => Some(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    });
    let id = response
        .and_then(|fields| fields.get("id"))
        .and_then(text_value)
        .filter(|id| !id.is_empty())
        .map_or_else(|| payload.id.clone(), str::to_owned);
    let created = response
        .and_then(|fields| fields.get("created"))
        .and_then(number_value)
        .or_else(|| payload.start_time.to_i64())
        .unwrap_or(0);
    ResponseStamp { id, created }
}

const fn text_value(value: &Json) -> Option<&str> {
    match value {
        Json::Text(text) => Some(text.as_str()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

fn number_value(value: &Json) -> Option<i64> {
    match value {
        Json::Number(number) => number.as_i64(),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    }
}
