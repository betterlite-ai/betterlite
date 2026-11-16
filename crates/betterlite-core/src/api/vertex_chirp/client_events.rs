use betterlite_models::decisions::Json;

use crate::api::transcription_protocol::JsonObject;

pub(super) const CONFIGURE_EVENT_TYPES: [&str; 2] =
    ["session.update", "transcription_session.update"];
pub(super) const APPEND_EVENT_TYPE: &str = "input_audio_buffer.append";
pub(super) const COMMIT_EVENT_TYPES: [&str; 2] =
    ["input_audio_buffer.commit", "input_audio_buffer.end"];
pub(super) const CLEAR_EVENT_TYPE: &str = "input_audio_buffer.clear";

pub(super) fn event_type(request: &JsonObject) -> Option<&str> {
    match request.get("type") {
        Some(Json::Text(text)) => Some(text.as_str()),
        _ => None,
    }
}
