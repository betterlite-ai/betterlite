use std::slice;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Error as JsonError;
use serde_json::value::RawValue;

use super::json_object::JsonObject;
use super::openai_wire::{
    ImageUrl, InputAudio, OpenAiContent, OpenAiContentPart, OpenAiFunctionCall, OpenAiMessage,
    OpenAiRole, OpenAiToolCall, ToolKind,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SamplingRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpContentBlock {
    Text {
        text: String,
    },
    Image {
        data: String,
        #[serde(rename = "mimeType", alias = "mime_type")]
        mime_type: String,
    },
    Audio {
        data: String,
        #[serde(rename = "mimeType", alias = "mime_type")]
        mime_type: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: JsonObject,
    },
    ToolResult {
        #[serde(rename = "toolUseId", alias = "tool_use_id")]
        tool_use_id: String,
        #[serde(default)]
        content: Vec<ToolResultBlock>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum ToolResultBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SamplingContent {
    Single(McpContentBlock),
    Many(Vec<McpContentBlock>),
}

// serde buffers tagged and untagged enums, where a RawValue tool input cannot be read, so dispatch here
#[derive(Deserialize)]
struct ContentTag {
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
struct TextFields {
    text: String,
}

#[derive(Deserialize)]
struct MediaFields {
    data: String,
    #[serde(rename = "mimeType", alias = "mime_type")]
    mime_type: String,
}

#[derive(Deserialize)]
struct ToolUseFields {
    id: String,
    name: String,
    input: JsonObject,
}

#[derive(Deserialize)]
struct ToolResultFields {
    #[serde(rename = "toolUseId", alias = "tool_use_id")]
    tool_use_id: String,
    #[serde(default)]
    content: Vec<ToolResultBlock>,
}

fn parse_content_block(text: &str) -> Result<McpContentBlock, JsonError> {
    let tag: ContentTag = serde_json::from_str(text)?;
    match tag.kind.as_str() {
        "text" => serde_json::from_str::<TextFields>(text)
            .map(|fields| McpContentBlock::Text { text: fields.text }),
        "image" => serde_json::from_str::<MediaFields>(text).map(|fields| McpContentBlock::Image {
            data: fields.data,
            mime_type: fields.mime_type,
        }),
        "audio" => serde_json::from_str::<MediaFields>(text).map(|fields| McpContentBlock::Audio {
            data: fields.data,
            mime_type: fields.mime_type,
        }),
        "tool_use" => {
            serde_json::from_str::<ToolUseFields>(text).map(|fields| McpContentBlock::ToolUse {
                id: fields.id,
                name: fields.name,
                input: fields.input,
            })
        }
        "tool_result" => serde_json::from_str::<ToolResultFields>(text).map(|fields| {
            McpContentBlock::ToolResult {
                tool_use_id: fields.tool_use_id,
                content: fields.content,
            }
        }),
        other => Err(JsonError::custom(format!(
            "unknown MCP content block type `{other}`"
        ))),
    }
}

pub(crate) fn sampling_content<'de, D>(deserializer: D) -> Result<SamplingContent, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Box::<RawValue>::deserialize(deserializer)?;
    match serde_json::from_str::<Vec<Box<RawValue>>>(raw.get()) {
        Ok(items) => {
            let mut blocks = Vec::with_capacity(items.len());
            for item in &items {
                blocks.push(parse_content_block(item.get()).map_err(D::Error::custom)?);
            }
            Ok(SamplingContent::Many(blocks))
        }
        Err(_not_a_sequence) => parse_content_block(raw.get())
            .map(SamplingContent::Single)
            .map_err(D::Error::custom),
    }
}

pub(crate) enum ConvertedPart {
    Regular(OpenAiContentPart),
    ToolUse(OpenAiToolCall),
    ToolResult(OpenAiMessage),
}

pub(crate) fn convert_content(content: &SamplingContent) -> Vec<ConvertedPart> {
    match content {
        SamplingContent::Single(block) => vec![convert_block(block)],
        SamplingContent::Many(blocks) => blocks.iter().map(convert_block).collect(),
    }
}

fn convert_block(block: &McpContentBlock) -> ConvertedPart {
    match block {
        McpContentBlock::Text { text } => {
            ConvertedPart::Regular(OpenAiContentPart::Text { text: text.clone() })
        }
        McpContentBlock::Image { data, mime_type } => {
            ConvertedPart::Regular(OpenAiContentPart::ImageUrl {
                image_url: ImageUrl {
                    url: format!("data:{mime_type};base64,{data}"),
                },
            })
        }
        McpContentBlock::Audio { data, mime_type } => {
            ConvertedPart::Regular(OpenAiContentPart::InputAudio {
                input_audio: InputAudio {
                    data: data.clone(),
                    format: audio_format(mime_type).to_owned(),
                },
            })
        }
        McpContentBlock::ToolUse { id, name, input } => ConvertedPart::ToolUse(OpenAiToolCall {
            id: id.clone(),
            kind: ToolKind::Function,
            function: OpenAiFunctionCall {
                name: name.clone(),
                arguments: input.as_ref().to_owned(),
            },
        }),
        McpContentBlock::ToolResult {
            tool_use_id,
            content,
        } => ConvertedPart::ToolResult(tool_result_message(tool_use_id, content)),
    }
}

fn audio_format(mime_type: &str) -> &'static str {
    match mime_type {
        "audio/mp3" | "audio/mpeg" => "mp3",
        "audio/flac" => "flac",
        "audio/ogg" => "ogg",
        _ => "wav",
    }
}

fn tool_result_message(tool_use_id: &str, content: &[ToolResultBlock]) -> OpenAiMessage {
    let texts: Vec<&str> = content
        .iter()
        .filter_map(|block| match block {
            ToolResultBlock::Text { text } => Some(text.as_str()),
            ToolResultBlock::Other => None,
        })
        .collect();
    OpenAiMessage {
        role: OpenAiRole::Tool,
        content: Some(OpenAiContent::Text(texts.join("\n"))),
        tool_calls: None,
        tool_call_id: Some(tool_use_id.to_owned()),
    }
}

pub(crate) fn blocks(content: &SamplingContent) -> &[McpContentBlock] {
    match content {
        SamplingContent::Single(block) => slice::from_ref(block),
        SamplingContent::Many(blocks) => blocks,
    }
}

pub(crate) fn has_tool_use(content: &SamplingContent) -> bool {
    blocks(content)
        .iter()
        .any(|block| matches!(block, McpContentBlock::ToolUse { .. }))
}

pub(crate) fn has_tool_result(content: &SamplingContent) -> bool {
    blocks(content)
        .iter()
        .any(|block| matches!(block, McpContentBlock::ToolResult { .. }))
}

pub(crate) fn text_parts(content: &SamplingContent) -> Option<String> {
    let texts: Vec<&str> = blocks(content)
        .iter()
        .filter_map(|block| match block {
            McpContentBlock::Text { text } => Some(text.as_str()),
            McpContentBlock::Image { .. }
            | McpContentBlock::Audio { .. }
            | McpContentBlock::ToolUse { .. }
            | McpContentBlock::ToolResult { .. } => None,
        })
        .collect();
    (!texts.is_empty()).then(|| texts.join("\n"))
}

pub(crate) fn extract_tool_calls(content: &SamplingContent) -> Vec<OpenAiToolCall> {
    let mut calls = Vec::new();
    for part in convert_content(content) {
        if let ConvertedPart::ToolUse(call) = part {
            calls.push(call);
        }
    }
    calls
}

pub(crate) fn extract_tool_results(content: &SamplingContent) -> Vec<OpenAiMessage> {
    convert_content(content)
        .into_iter()
        .filter_map(|part| match part {
            ConvertedPart::ToolResult(message) => Some(message),
            ConvertedPart::Regular(_) | ConvertedPart::ToolUse(_) => None,
        })
        .collect()
}
