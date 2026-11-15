use betterlite_core::api::transcription_protocol::JsonObject;
use betterlite_core::api::transcription_protocol::events::UuidEventIds;
use betterlite_core::meta_muse_realtime::MuseMode;
use betterlite_core::meta_muse_realtime::events::MuseEventTransformer;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;

use super::normalize_ids;
use crate::helpers::HelperResult;

#[derive(Debug, Deserialize)]
pub(crate) struct EventsInput {
    mode: MuseMode,
    frames: Vec<JsonObject>,
}

pub(crate) fn events(fields: &EventsInput) -> HelperResult {
    let mut transformer = MuseEventTransformer::default();
    let mut ids = UuidEventIds;
    let events: Vec<Value> = fields
        .frames
        .iter()
        .map(|frame| {
            transformer
                .transform(frame, fields.mode, &mut ids)
                .map_or(Value::Null, |events| json!(events))
        })
        .collect();
    let usage = json!(transformer.take_unbilled_usage());
    Ok(normalize_ids(&json!({"events": events, "usage": usage})))
}
