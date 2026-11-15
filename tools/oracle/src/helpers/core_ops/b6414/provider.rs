use betterlite_core::api::realtime_contracts::events::AllDeltaType;
use betterlite_core::api::realtime_contracts::events::RealtimeResponseTransformInput;
use betterlite_core::api::realtime_contracts::transport::RealtimeMessage;
use betterlite_core::api::transcription_protocol::JsonObject;
use betterlite_core::api::transcription_protocol::events::UuidEventIds;
use betterlite_core::meta_muse_realtime::MuseMode;
use betterlite_core::meta_muse_realtime::events::MuseEventTransformer;
use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;

use super::config_for;
use super::message_document;
use super::normalize_ids;
use crate::helpers::HelperResult;

#[derive(Debug, Deserialize)]
pub(crate) struct EventsInput {
    mode: MuseMode,
    frames: Vec<JsonObject>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponseInput {
    model: String,
    access_token: String,
    logging_session_id: String,
    messages: Vec<String>,
    frames: Vec<FrameInput>,
    transform: TransformInput,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum FrameInput {
    Text { text: String },
    Binary { binary: Vec<u8> },
}

#[derive(Debug, Deserialize)]
pub(crate) struct TransformInput {
    session_configuration_request: Option<String>,
    current_output_item_id: Option<String>,
    current_response_id: Option<String>,
    current_delta_chunks: Option<Vec<Json>>,
    current_item_chunks: Option<Vec<Json>>,
    current_conversation_id: Option<String>,
    current_delta_type: Option<AllDeltaType>,
}

impl FrameInput {
    fn message(&self) -> RealtimeMessage {
        match self {
            Self::Text { text } => RealtimeMessage::Text(text.clone()),
            Self::Binary { binary } => RealtimeMessage::Binary(binary.clone()),
        }
    }
}

impl TransformInput {
    fn input(&self) -> RealtimeResponseTransformInput {
        RealtimeResponseTransformInput {
            session_configuration_request: self.session_configuration_request.clone(),
            current_output_item_id: self.current_output_item_id.clone(),
            current_response_id: self.current_response_id.clone(),
            current_delta_chunks: self.current_delta_chunks.clone(),
            current_item_chunks: self.current_item_chunks.clone(),
            current_conversation_id: self.current_conversation_id.clone(),
            current_delta_type: self.current_delta_type,
        }
    }
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

pub(crate) fn response(fields: &ResponseInput) -> HelperResult {
    let mut config = config_for(&fields.model, &fields.access_token)?;
    let messages: Vec<Value> = fields
        .messages
        .iter()
        .map(|message| {
            config
                .transform_realtime_request(message, &fields.model, None)
                .map_or(Value::Null, |frames| {
                    Value::Array(frames.iter().map(message_document).collect())
                })
        })
        .collect();
    let transform = fields.transform.input();
    let frames: Vec<Value> = fields
        .frames
        .iter()
        .map(|frame| {
            config
                .transform_realtime_response(&frame.message(), &fields.model, &transform)
                .map_or(Value::Null, |response| json!(response))
        })
        .collect();
    let created = json!(config.transform_session_created_event(&fields.logging_session_id));
    let usage = json!(config.unbilled_usage_on_session_close(&fields.model));
    Ok(normalize_ids(&json!({
        "messages": messages,
        "frames": frames,
        "created": created,
        "usage": usage,
    })))
}
