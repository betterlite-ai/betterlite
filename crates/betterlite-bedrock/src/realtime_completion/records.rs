use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeltaType {
    Text,
    Audio,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaChunk {
    pub delta: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResponseState {
    pub output_item_id: Option<String>,
    pub response_id: Option<String>,
    pub conversation_id: Option<String>,
    pub delta_type: Option<DeltaType>,
    pub delta_chunks: Option<Vec<DeltaChunk>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type")]
pub enum CompletionEvent {
    #[serde(rename = "response.text.done")]
    TextDone(TextDone),
    #[serde(rename = "response.audio.done")]
    AudioDone(AudioDone),
    #[serde(rename = "response.content_part.done")]
    ContentPartDone(ContentPartDone),
    #[serde(rename = "response.output_item.done")]
    OutputItemDone(OutputItemDone),
    #[serde(rename = "response.done")]
    ResponseDone(ResponseDone),
    #[serde(rename = "response.function_call_arguments.done")]
    FunctionCallArgumentsDone(FunctionCallArgumentsDone),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TextDone {
    pub content_index: u32,
    pub event_id: String,
    pub item_id: String,
    pub output_index: u32,
    pub response_id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AudioDone {
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
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ContentPart {
    Text { text: String },
    Audio { transcript: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OutputItemDone {
    pub event_id: String,
    pub output_index: u32,
    pub response_id: String,
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
    pub content: [(); 0],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResponseDone {
    pub event_id: String,
    pub response: ResponseObject,
}

// LiteLLM's leftover done event never sets output, so absent and [] are different wire states
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResponseObject {
    pub object: &'static str,
    pub id: String,
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<[(); 0]>,
    pub conversation_id: String,
    pub usage: ResponseUsage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResponseUsage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub input_token_details: InputTokenDetails,
    pub output_token_details: OutputTokenDetails,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InputTokenDetails {
    #[serde(rename = "audio_tokens")]
    pub audio: i64,
    #[serde(rename = "text_tokens")]
    pub text: i64,
    #[serde(rename = "cached_tokens")]
    pub cached: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OutputTokenDetails {
    pub audio_tokens: i64,
    pub text_tokens: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FunctionCallArgumentsDone {
    pub event_id: String,
    pub response_id: String,
    pub item_id: String,
    pub output_index: u32,
    pub call_id: String,
    pub name: String,
    pub arguments: String,
}
