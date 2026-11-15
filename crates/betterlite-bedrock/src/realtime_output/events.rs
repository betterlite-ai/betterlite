use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type")]
pub enum OutputEvent {
    #[serde(rename = "response.created")]
    ResponseCreated(ResponseCreated),
    #[serde(rename = "response.output_item.added")]
    OutputItemAdded(OutputItemAdded),
    #[serde(rename = "response.content_part.added")]
    ContentPartAdded(ContentPartAdded),
    #[serde(rename = "response.text.delta")]
    TextDelta(ResponseDelta),
    #[serde(rename = "response.audio.delta")]
    AudioDelta(ResponseDelta),
    #[serde(rename = "input_audio_buffer.speech_started")]
    SpeechStarted(SpeechBoundary),
    #[serde(rename = "input_audio_buffer.speech_stopped")]
    SpeechStopped(SpeechBoundary),
    #[serde(rename = "conversation.item.input_audio_transcription.delta")]
    TranscriptionDelta(TranscriptionDelta),
    #[serde(rename = "conversation.item.input_audio_transcription.completed")]
    TranscriptionCompleted(TranscriptionCompleted),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResponseCreated {
    pub event_id: String,
    pub response: ResponseObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResponseObject {
    pub object: &'static str,
    pub id: String,
    pub status: &'static str,
    pub output: Vec<OutputItem>,
    pub conversation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OutputItemAdded {
    pub response_id: String,
    pub output_index: u32,
    pub item: OutputItem,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OutputItem {
    pub id: String,
    pub object: &'static str,
    #[serde(rename = "type")]
    pub item_type: &'static str,
    pub status: &'static str,
    pub role: &'static str,
    pub content: Vec<ContentPart>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContentPartAdded {
    pub content_index: u32,
    pub output_index: u32,
    pub event_id: String,
    pub item_id: String,
    pub part: ContentPart,
    pub response_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ContentPart {
    Text { text: String },
    Audio { transcript: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResponseDelta {
    pub content_index: u32,
    pub event_id: String,
    pub item_id: String,
    pub output_index: u32,
    pub response_id: String,
    pub delta: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SpeechBoundary {
    pub event_id: String,
    pub item_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TranscriptionDelta {
    pub event_id: String,
    pub item_id: String,
    pub content_index: u32,
    pub delta: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TranscriptionCompleted {
    pub event_id: String,
    pub item_id: String,
    pub content_index: u32,
    pub transcript: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeltaType {
    Text,
    Audio,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
pub struct OutputIds {
    pub response: Option<String>,
    pub item: Option<String>,
    pub conversation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentStarted {
    pub events: Vec<OutputEvent>,
    pub ids: OutputIds,
    pub delta_type: Option<DeltaType>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum ModelFields {
    Text(String),
    Other(IgnoredAny),
}

#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
pub struct ContentStart {
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default, rename = "type")]
    pub content_type: Option<String>,
    #[serde(default, rename = "additionalModelFields")]
    pub additional_model_fields: Option<ModelFields>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
pub struct TextOutput {
    #[serde(default)]
    pub content: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
pub struct AudioOutput {
    #[serde(default)]
    pub content: String,
}
