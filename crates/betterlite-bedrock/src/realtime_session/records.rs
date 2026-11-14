use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

pub const AUDIO_MEDIA_TYPE: &str = "audio/lpcm";
pub const AUDIO_ENCODING: &str = "base64";
pub const AUDIO_TYPE: &str = "SPEECH";
pub const TEXT_MEDIA_TYPE: &str = "text/plain";
pub const TOOL_MEDIA_TYPE: &str = "application/json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ExactDecimal(#[serde(with = "rust_decimal::serde::arbitrary_precision")] pub Decimal);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Setting<T>(pub Option<T>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MaxResponseTokens {
    Count(i64),
    Infinite(Infinite),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Infinite {
    Inf,
}

// LiteLLM's configuration marker keeps its snake_case keys around the camelCase event payloads
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionConfiguration {
    pub session_start: SessionStartEvent,
    pub prompt_start: PromptStartEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStartFrame {
    pub event: SessionStartEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStartEvent {
    pub session_start: SessionStart,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStart {
    pub inference_configuration: InferenceConfiguration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InferenceConfiguration {
    pub max_tokens: Setting<MaxResponseTokens>,
    pub top_p: ExactDecimal,
    pub temperature: Setting<ExactDecimal>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptStartFrame {
    pub event: PromptStartEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptStartEvent {
    pub prompt_start: PromptStart,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptStart {
    pub prompt_name: String,
    pub text_output_configuration: MediaType,
    pub audio_output_configuration: AudioOutputConfiguration,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_use_output_configuration: Option<MediaType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_configuration: Option<ToolConfiguration>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaType {
    pub media_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioOutputConfiguration {
    pub media_type: String,
    pub sample_rate_hertz: Setting<i64>,
    pub sample_size_bits: u32,
    pub channel_count: u32,
    pub voice_id: Setting<String>,
    pub encoding: String,
    pub audio_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolConfiguration {
    pub tools: Vec<Tool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
    pub tool_spec: ToolSpec,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: ToolInputSchema,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolInputSchema {
    pub json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentStartFrame {
    pub event: ContentStartEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentStartEvent {
    pub content_start: ContentStart,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentStart {
    pub prompt_name: String,
    pub content_name: String,
    #[serde(rename = "type")]
    pub content_type: ContentType,
    pub interactive: bool,
    pub role: ContentRole,
    pub text_input_configuration: TextInputConfiguration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ContentType {
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ContentRole {
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextInputConfiguration {
    pub media_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextInputFrame {
    pub event: TextInputEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextInputEvent {
    pub text_input: TextInput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextInput {
    pub prompt_name: String,
    pub content_name: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentEndFrame {
    pub event: ContentEndEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentEndEvent {
    pub content_end: ContentEnd,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentEnd {
    pub prompt_name: String,
    pub content_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum UpdateFrame {
    SessionStart(SessionStartFrame),
    PromptStart(PromptStartFrame),
    ContentStart(ContentStartFrame),
    TextInput(TextInputFrame),
    ContentEnd(ContentEndFrame),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEvent {
    pub event_id: String,
    pub session: SessionObject,
    #[serde(rename = "type")]
    pub event_type: SessionEventType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SessionEventType {
    #[serde(rename = "session.created")]
    Created,
    #[serde(rename = "session.updated")]
    Updated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionObject {
    pub id: String,
    pub modalities: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageEvent {
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_tokens: i64,
    pub details: UsageDetails,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageDetails {
    pub total: UsageDetailsTotal,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageDetailsTotal {
    pub input: UsageTokens,
    pub output: UsageTokens,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageTokens {
    pub speech_tokens: i64,
    pub text_tokens: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BedrockContentEnd {
    pub stop_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioSettings {
    pub sample_rate_hertz: Setting<i64>,
    pub sample_size_bits: u32,
    pub channel_count: u32,
    pub encoding: String,
    pub audio_type: String,
    pub media_type: String,
}
