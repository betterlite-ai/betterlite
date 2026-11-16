use betterlite_prompts::arguments::json::Object;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RealtimeInputMessage {
    pub realtime_input: Object,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientContentMessage {
    pub client_content: ClientContent,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientContent {
    pub turns: Vec<Turn>,
    pub turn_complete: bool,
}

#[derive(Debug, Serialize)]
pub struct Turn {
    pub role: String,
    pub parts: Vec<TextPart>,
}

#[derive(Debug, Serialize)]
pub struct TextPart {
    pub text: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResponseMessage {
    pub tool_response: ToolResponse,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResponse {
    pub function_responses: Vec<FunctionResponse>,
}

#[derive(Debug, Serialize)]
pub struct FunctionResponse {
    pub response: Object,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

// B1382 decodes one server frame at a time, so these records cover the values it reads and leave sibling keys alone
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ServerMessage {
    pub usage_metadata: Option<Object>,
    pub server_content: Option<ServerContent>,
    pub setup_complete: Option<Object>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ServerContent {
    pub generation_complete: Option<bool>,
    pub turn_complete: Option<bool>,
    pub interrupted: Option<bool>,
    pub grounding_metadata: Option<Object>,
    pub input_transcription: Option<Transcription>,
    pub output_transcription: Option<Transcription>,
    pub model_turn: Option<ModelTurn>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub struct Transcription {
    pub text: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub struct ModelTurn {
    pub role: Option<String>,
    pub parts: Option<Vec<ModelTurnPart>>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelTurnPart {
    pub text: Option<String>,
    pub inline_data: Option<InlineData>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InlineData {
    pub mime_type: Option<String>,
    pub data: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageKind {
    Setup,
    Content,
    Other,
}

#[must_use]
pub fn message_kind(message: &Object) -> MessageKind {
    if message.contains_key("setup") {
        MessageKind::Setup
    } else if ["realtimeInput", "clientContent", "toolResponse"]
        .iter()
        .any(|key| message.contains_key(*key))
    {
        MessageKind::Content
    } else {
        MessageKind::Other
    }
}
