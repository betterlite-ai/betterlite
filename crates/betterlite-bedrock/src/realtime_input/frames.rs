use std::collections::BTreeMap;

use serde::Serialize;

use crate::realtime_session::records::{ContentEnd, Setting, TextInputConfiguration, UpdateFrame};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum InputFrame {
    Session(UpdateFrame),
    Event(EventFrame),
}

impl From<Event> for InputFrame {
    fn from(event: Event) -> Self {
        Self::Event(EventFrame { event })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EventFrame {
    pub event: Event,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Event {
    ContentStart(ContentStart),
    AudioInput(AudioInput),
    TextInput(UserTextInput),
    ToolResult(ToolResult),
    ContentEnd(ContentEnd),
    PromptEnd(PromptEnd),
    // sessionEnd carries an empty JSON object
    SessionEnd(BTreeMap<String, String>),
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
    #[serde(flatten)]
    pub configuration: InputConfiguration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ContentType {
    Audio,
    Text,
    Tool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ContentRole {
    User,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InputConfiguration {
    AudioInputConfiguration(AudioInputConfiguration),
    TextInputConfiguration(TextInputConfiguration),
    ToolResultInputConfiguration(ToolResultInputConfiguration),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioInputConfiguration {
    pub media_type: String,
    pub sample_rate_hertz: Setting<i64>,
    pub sample_size_bits: u32,
    pub channel_count: u32,
    pub audio_type: String,
    pub encoding: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResultInputConfiguration {
    pub tool_use_id: String,
    #[serde(rename = "type")]
    pub content_type: ContentType,
    pub text_input_configuration: TextInputConfiguration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioInput {
    pub prompt_name: String,
    pub content_name: String,
    pub content: Setting<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserTextInput {
    pub prompt_name: String,
    pub content_name: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResult {
    pub prompt_name: String,
    pub content_name: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptEnd {
    pub prompt_name: String,
}
