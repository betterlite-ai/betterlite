use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_with::{DefaultOnNull, serde_as};

use crate::count_tokens::python_text::{PyValue, display_text};

type Fields = IndexMap<String, PyValue>;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Blocks(Vec<PyValue>),
}

// LiteLLM reads these with dict.get(key, ""), so a null or non-string value is sent on as given
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(transparent)]
pub struct Forwarded(Value);

impl Default for Forwarded {
    fn default() -> Self {
        Self(Value::from(""))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ToolCallFunction {
    #[serde(default)]
    pub name: Forwarded,
    #[serde(default)]
    pub arguments: Forwarded,
}

#[serde_as]
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ToolCall {
    #[serde(default)]
    pub id: Forwarded,
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub function: ToolCallFunction,
}

#[serde_as]
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ChatMessage {
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub role: String,
    #[serde(default)]
    pub content: Option<MessageContent>,
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default)]
    pub tool_call_id: Forwarded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponsesPart {
    InputText { text: String },
    InputImage { image_url: String, detail: String },
    InputFile { filename: String, file_data: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ResponsesContent {
    Text(String),
    Parts(Vec<ResponsesPart>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CallItem {
    FunctionCall {
        call_id: Forwarded,
        name: Forwarded,
        arguments: Forwarded,
    },
    FunctionCallOutput {
        call_id: Forwarded,
        output: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum InputItem {
    Message {
        role: ContentRole,
        content: ResponsesContent,
    },
    Call(CallItem),
}

fn text_field<'value>(fields: &'value Fields, key: &str) -> Option<&'value str> {
    match fields.get(key) {
        Some(PyValue::Text(text)) => Some(text),
        _ => None,
    }
}

fn image_part(image_url: Option<&PyValue>) -> Option<ResponsesPart> {
    let (url, detail) = match image_url {
        Some(PyValue::Dict(fields)) => (
            text_field(fields, "url").filter(|text| !text.is_empty())?,
            text_field(fields, "detail").filter(|text| !text.is_empty()),
        ),
        Some(PyValue::Text(url)) if !url.is_empty() => (url.as_str(), None),
        _ => return None,
    };
    Some(ResponsesPart::InputImage {
        image_url: url.to_owned(),
        detail: detail.unwrap_or("auto").to_owned(),
    })
}

fn block_to_part(block: &PyValue, role: ContentRole) -> Option<ResponsesPart> {
    let fields = match block {
        PyValue::Text(text) => {
            return Some(ResponsesPart::InputText { text: text.clone() });
        }
        PyValue::Dict(fields) => fields,
        PyValue::Null | PyValue::Bool(_) | PyValue::Number(_) | PyValue::List(_) => return None,
    };
    match (text_field(fields, "type"), role) {
        (Some("text"), _) => Some(ResponsesPart::InputText {
            text: text_field(fields, "text").unwrap_or_default().to_owned(),
        }),
        (Some("image_url"), ContentRole::User) => image_part(fields.get("image_url")),
        (Some("file"), ContentRole::User) => {
            let Some(PyValue::Dict(file)) = fields.get("file") else {
                return None;
            };
            Some(ResponsesPart::InputFile {
                filename: text_field(file, "filename")
                    .filter(|text| !text.is_empty())?
                    .to_owned(),
                file_data: text_field(file, "file_data")
                    .filter(|text| !text.is_empty())?
                    .to_owned(),
            })
        }
        _ => None,
    }
}

#[must_use]
pub fn blocks_to_content(content: &[PyValue], role: ContentRole) -> ResponsesContent {
    let parts: Vec<ResponsesPart> = content
        .iter()
        .filter_map(|block| block_to_part(block, role))
        .collect();
    if parts
        .iter()
        .any(|part| !matches!(part, ResponsesPart::InputText { .. }))
    {
        return ResponsesContent::Parts(parts);
    }
    let texts: Vec<String> = parts
        .into_iter()
        .filter_map(|part| match part {
            ResponsesPart::InputText { text } => Some(text),
            ResponsesPart::InputImage { .. } | ResponsesPart::InputFile { .. } => None,
        })
        .collect();
    ResponsesContent::Text(texts.join("\n"))
}

fn instruction_text(content: Option<&MessageContent>) -> String {
    match content {
        None => String::new(),
        Some(MessageContent::Text(text)) => text.clone(),
        Some(MessageContent::Blocks(blocks)) => {
            let mut texts = Vec::new();
            for block in blocks {
                match block {
                    PyValue::Dict(fields) if text_field(fields, "type") == Some("text") => {
                        texts.push(text_field(fields, "text").unwrap_or_default().to_owned());
                    }
                    PyValue::Text(text) => texts.push(text.clone()),
                    PyValue::Null
                    | PyValue::Bool(_)
                    | PyValue::Number(_)
                    | PyValue::List(_)
                    | PyValue::Dict(_) => {}
                }
            }
            texts.join("\n")
        }
    }
}

fn converted(content: Option<&MessageContent>, role: ContentRole) -> ResponsesContent {
    match content {
        None => ResponsesContent::Text(String::new()),
        Some(MessageContent::Text(text)) => ResponsesContent::Text(text.clone()),
        Some(MessageContent::Blocks(blocks)) => blocks_to_content(blocks, role),
    }
}

const fn is_empty(content: &ResponsesContent) -> bool {
    match content {
        ResponsesContent::Text(text) => text.is_empty(),
        ResponsesContent::Parts(parts) => parts.is_empty(),
    }
}

fn push_assistant(message: &ChatMessage, input_items: &mut Vec<InputItem>) {
    let text = converted(message.content.as_ref(), ContentRole::Assistant);
    let has_text = !is_empty(&text);
    if has_text {
        input_items.push(InputItem::Message {
            role: ContentRole::Assistant,
            content: text.clone(),
        });
    }
    for call in &message.tool_calls {
        input_items.push(InputItem::Call(CallItem::FunctionCall {
            call_id: call.id.clone(),
            name: call.function.name.clone(),
            arguments: call.function.arguments.clone(),
        }));
    }
    if message.tool_calls.is_empty() && !has_text {
        input_items.push(InputItem::Message {
            role: ContentRole::Assistant,
            content: text,
        });
    }
}

#[must_use]
pub fn messages_to_responses_input(messages: &[ChatMessage]) -> (Vec<InputItem>, Option<String>) {
    let mut input_items = Vec::new();
    let mut instructions_parts = Vec::new();
    for message in messages {
        let content = message.content.as_ref();
        match message.role.as_str() {
            "system" | "developer" => instructions_parts.push(instruction_text(content)),
            "user" => input_items.push(InputItem::Message {
                role: ContentRole::User,
                content: converted(content, ContentRole::User),
            }),
            "assistant" => push_assistant(message, &mut input_items),
            "tool" => input_items.push(InputItem::Call(CallItem::FunctionCallOutput {
                call_id: message.tool_call_id.clone(),
                output: match content {
                    Some(MessageContent::Text(text)) => text.clone(),
                    Some(MessageContent::Blocks(blocks)) if !blocks.is_empty() => {
                        display_text(&PyValue::List(blocks.clone()))
                    }
                    Some(MessageContent::Blocks(_)) | None => String::new(),
                },
            })),
            _ => {}
        }
    }
    let instructions = if instructions_parts.is_empty() {
        None
    } else {
        Some(instructions_parts.join("\n"))
    };
    (input_items, instructions)
}
