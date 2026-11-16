use serde::Serialize;
use serde_json::Number;

use crate::gemini_live_setup::session::MaxResponseTokens;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type")]
pub enum ContentEvent {
    #[serde(rename = "response.created")]
    ResponseCreated(ResponseCreated),
    #[serde(rename = "response.output_item.added")]
    OutputItemAdded(OutputItemAdded),
    #[serde(rename = "conversation.item.added")]
    ConversationItemAdded(ConversationItemAdded),
    #[serde(rename = "response.content_part.added")]
    ContentPartAdded(ContentPartAdded),
    #[serde(rename = "response.output_text.delta")]
    TextDelta(ResponseDelta),
    #[serde(rename = "response.output_audio.delta")]
    AudioDelta(ResponseDelta),
    #[serde(rename = "response.output_text.done")]
    TextDone(ResponseTextDone),
    #[serde(rename = "response.output_audio.done")]
    AudioDone(ResponseAudioDone),
    #[serde(rename = "response.content_part.done")]
    ContentPartDone(ContentPartDone),
    #[serde(rename = "response.output_item.done")]
    OutputItemDone(OutputItemDone),
}

impl ContentEvent {
    #[must_use]
    pub fn done_text(&self) -> Option<&str> {
        if let Self::TextDone(done) = self {
            return Some(done.text.as_str());
        }
        None
    }

    #[must_use]
    pub fn done_ids(&self) -> Option<(&str, &str)> {
        match self {
            Self::TextDone(done) => Some((done.item_id.as_str(), done.response_id.as_str())),
            Self::AudioDone(done) => Some((done.item_id.as_str(), done.response_id.as_str())),
            Self::ResponseCreated(_)
            | Self::OutputItemAdded(_)
            | Self::ConversationItemAdded(_)
            | Self::ContentPartAdded(_)
            | Self::TextDelta(_)
            | Self::AudioDelta(_)
            | Self::ContentPartDone(_)
            | Self::OutputItemDone(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResponseCreated {
    pub event_id: String,
    pub response: ResponseObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResponseObject {
    pub object: String,
    pub id: String,
    pub status: String,
    pub status_details: Option<()>,
    pub output: Vec<OutputItem>,
    pub conversation_id: String,
    pub modalities: Vec<String>,
    pub temperature: Option<Number>,
    pub max_output_tokens: Option<MaxResponseTokens>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OutputItemAdded {
    pub event_id: String,
    pub response_id: String,
    pub output_index: u32,
    pub item: OutputItem,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConversationItemAdded {
    pub event_id: String,
    pub previous_item_id: Option<()>,
    pub item: OutputItem,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OutputItem {
    pub id: String,
    pub object: String,
    #[serde(rename = "type")]
    pub item_type: String,
    pub status: String,
    pub role: String,
    pub content: Vec<ContentPart>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ContentPart {
    Text { text: String },
    Audio { transcript: String },
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
pub struct ResponseDelta {
    pub content_index: u32,
    pub event_id: String,
    pub item_id: String,
    pub output_index: u32,
    pub response_id: String,
    pub delta: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResponseTextDone {
    pub content_index: u32,
    pub event_id: String,
    pub item_id: String,
    pub output_index: u32,
    pub response_id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResponseAudioDone {
    pub content_index: u32,
    pub event_id: String,
    pub item_id: String,
    pub output_index: u32,
    pub response_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContentPartDone {
    pub content_index: u32,
    pub event_id: String,
    pub item_id: String,
    pub output_index: u32,
    pub part: ContentPart,
    pub response_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OutputItemDone {
    pub event_id: String,
    pub item: OutputItem,
    pub output_index: u32,
    pub response_id: String,
}
