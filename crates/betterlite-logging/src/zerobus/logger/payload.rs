use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::prelude::ToPrimitive as _;

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

use super::error::EnqueueError;

#[derive(Debug, Clone, Default)]
pub struct RedactionSettings {
    pub turn_off_message_logging: bool,
    pub excluded_fields: Vec<String>,
}

pub(super) struct EventDetails<'event> {
    pub(super) standard_logging_object: Option<IndexMap<String, Json>>,
    pub(super) stamp: ResponseStamp,
    pub(super) litellm_metadata: &'event IndexMap<String, Json>,
    pub(super) dynamic_params: &'event Json,
}

pub(super) struct PayloadPolicy<'settings> {
    pub(super) turn_off_message_logging: bool,
    pub(super) excluded_fields: &'settings [String],
    pub(super) global_turn_off: bool,
}

pub(super) fn event_details(event: &LoggerEvent) -> Result<EventDetails<'_>, EnqueueError> {
    let payload = event.payload.as_ref();
    Ok(EventDetails {
        standard_logging_object: Some(payload_to_map(payload)?),
        stamp: stamp_for(payload),
        litellm_metadata: &event.litellm_metadata,
        dynamic_params: &event.dynamic_params,
    })
}

fn stamp_for(payload: &StandardLoggingPayload) -> ResponseStamp {
    ResponseStamp {
        id: payload.id.clone(),
        created: payload.end_time.to_i64().unwrap_or_default(),
    }
}

pub(super) fn payload_for(
    details: &EventDetails<'_>,
    policy: &PayloadPolicy<'_>,
) -> Option<IndexMap<String, Json>> {
    let original = details.standard_logging_object.clone()?;
    let map = redact_details(&original, policy, &details.stamp);
    if should_redact(details, policy) {
        return redacted_payload(&map);
    }
    Some(map)
}

fn redact_details(
    standard_logging_object: &IndexMap<String, Json>,
    policy: &PayloadPolicy<'_>,
    stamp: &ResponseStamp,
) -> IndexMap<String, Json> {
    let details = ModelCallDetails {
        standard_logging_object: Some(standard_logging_object.clone()),
        proxy_server_request: None,
    };
    let redaction = RedactionPolicy {
        turn_off_message_logging: policy.turn_off_message_logging,
        excluded_fields: policy.excluded_fields.to_vec(),
        callback_redacts_itself: false,
    };
    // ast-grep-ignore: no-long-names
    match redact_standard_logging_payload(&details, &redaction, stamp) {
        Redaction::Unchanged => standard_logging_object.clone(),
        Redaction::Redacted {
            standard_logging_object: redacted,
            ..
        } => redacted,
    }
}

fn should_redact(details: &EventDetails<'_>, policy: &PayloadPolicy<'_>) -> bool {
    let params = IndexMap::from([(
        "metadata".to_owned(),
        Json::Object(details.litellm_metadata.clone()),
    )]);
    let dynamic = match details.dynamic_params {
        Json::Object(fields) => Some(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    };
    should_redact_message_logging(Some(&params), dynamic, policy.global_turn_off)
}

fn redacted_payload(map: &IndexMap<String, Json>) -> Option<IndexMap<String, Json>> {
    let Ok(payload) = map_to_payload(map) else {
        tracing::warn!(
            "zerobus: the redacted payload cannot be read back, so the event is dropped instead of logged unredacted"
        );
        return None;
    };
    match payload_to_map(&redact_standard_payload(&payload)) {
        Ok(redacted) => Some(redacted),
        Err(error) => {
            tracing::warn!(
                %error,
                "zerobus: the redacted payload could not be written, so the event is dropped"
            );
            None
        }
    }
}
