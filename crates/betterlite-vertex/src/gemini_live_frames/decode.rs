use std::str::from_utf8;

use betterlite_core::api::realtime_contracts::transport::RealtimeMessage;
use betterlite_prompts::arguments::json::Object;
use serde_json::Deserializer;

use super::error::LiveFrameError;
use crate::gemini_live_client::records::ServerMessage;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveFrame {
    pub entries: Object,
    pub message: ServerMessage,
}

pub fn decode_gemini_live_frame(
    message: &RealtimeMessage,
    limit: usize,
) -> Result<LiveFrame, LiveFrameError> {
    match message {
        RealtimeMessage::Text(text) => decode_gemini_live_frame_bytes(text.as_bytes(), limit),
        RealtimeMessage::Binary(bytes) => decode_gemini_live_frame_bytes(bytes, limit),
    }
}

pub fn decode_gemini_live_frame_bytes(
    bytes: &[u8],
    limit: usize,
) -> Result<LiveFrame, LiveFrameError> {
    if bytes.len() > limit {
        return Err(LiveFrameError::TooLarge { limit });
    }
    let entries: Object =
        serde_json::from_slice(bytes).map_err(|source| top_level_error(source, bytes))?;
    let mut decoder = Deserializer::from_slice(bytes);
    let message: ServerMessage =
        serde_path_to_error::deserialize(&mut decoder).map_err(field_error)?;
    decoder
        .end()
        .map_err(|source| top_level_error(source, bytes))?;
    Ok(LiveFrame { entries, message })
}

fn top_level_error(source: serde_json::Error, bytes: &[u8]) -> LiveFrameError {
    if source.is_data() {
        return LiveFrameError::NotAnObject(source);
    }
    LiveFrameError::InvalidJson {
        message: frame_text(bytes),
        source,
    }
}

fn frame_text(bytes: &[u8]) -> String {
    match from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(error) => format!("the frame is not UTF-8: {error}"),
    }
}

fn field_error(error: serde_path_to_error::Error<serde_json::Error>) -> LiveFrameError {
    let path = error.path().to_string();
    let source = error.into_inner();
    if source.is_data() {
        return LiveFrameError::InvalidField { path, source };
    }
    let message = source.to_string();
    LiveFrameError::InvalidJson { message, source }
}
