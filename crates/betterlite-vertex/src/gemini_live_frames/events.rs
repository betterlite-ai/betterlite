use betterlite_prompts::arguments::json::Json;
use serde::Serialize;

use super::error::LiveFrameError;
use crate::gemini_live_accounting::transcription::TranscriptionUsage;

pub const INPUT_TRANSCRIPT_COMPLETED: &str =
    "conversation.item.input_audio_transcription.completed";
pub const OUTPUT_TRANSCRIPT_DELTA: &str = "response.output_audio_transcript.delta";

#[derive(Debug, Serialize)]
pub struct InputTranscriptEvent {
    #[serde(rename = "type")]
    pub event_type: &'static str,
    pub event_id: String,
    pub transcript: String,
    pub item_id: String,
    pub content_index: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TranscriptionUsage>,
}

#[derive(Debug, Serialize)]
pub struct OutputTranscriptEvent {
    #[serde(rename = "type")]
    pub event_type: &'static str,
    pub event_id: String,
    pub transcript: String,
    pub item_id: String,
    pub content_index: u32,
    pub output_index: u32,
    pub response_id: String,
    pub delta: String,
}

pub fn event_json<E>(event: &E) -> Result<Json, LiveFrameError>
where
    E: Serialize,
{
    let value = serde_json::to_value(event).map_err(LiveFrameError::Encode)?;
    serde_json::from_value(value).map_err(LiveFrameError::Encode)
}
