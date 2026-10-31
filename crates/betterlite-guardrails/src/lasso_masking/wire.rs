use std::collections::BTreeMap;

use serde::de::{DeserializeOwned, Error as _};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::bedrock_message_selection::wire::{Content, Role};
use crate::lasso::wire::ToolArguments;

// Box<RawValue> needs serde_json's own deserializer, and flatten always buffers through a generic Content type first, so split known fields from the raw object instead of flattening RawValue.
fn split_fields<Head>(raw: &str) -> serde_json::Result<(Head, BTreeMap<String, Box<RawValue>>)>
where
    Head: DeserializeOwned + Serialize,
{
    let head = serde_json::from_str::<Head>(raw)?;
    let modeled =
        serde_json::from_str::<BTreeMap<String, Box<RawValue>>>(&serde_json::to_string(&head)?)?;
    let mut rest = serde_json::from_str::<BTreeMap<String, Box<RawValue>>>(raw)?;
    rest.retain(|key, _| !modeled.contains_key(key));
    Ok((head, rest))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "Box<RawValue>")]
pub struct Message {
    pub role: Role,
    pub content: Option<Content>,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub tool_call_id: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Box<RawValue>>,
}

#[derive(Deserialize, Serialize)]
struct MessageHead {
    role: Role,
    #[serde(default)]
    content: Option<Content>,
    #[serde(default)]
    tool_calls: Option<Vec<ToolCall>>,
    #[serde(default)]
    tool_call_id: Option<String>,
}

impl TryFrom<Box<RawValue>> for Message {
    type Error = serde_json::Error;

    fn try_from(raw: Box<RawValue>) -> Result<Self, Self::Error> {
        let (head, extra) = split_fields::<MessageHead>(raw.get())?;
        Ok(Self {
            role: head.role,
            content: head.content,
            tool_calls: head.tool_calls,
            tool_call_id: head.tool_call_id,
            extra,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "Box<RawValue>")]
pub struct ToolCall {
    pub id: Option<String>,
    pub function: Option<ToolCallFunction>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Box<RawValue>>,
}

#[derive(Deserialize, Serialize)]
struct ToolCallHead {
    id: Option<String>,
    #[serde(default)]
    function: Option<ToolCallFunction>,
}

impl TryFrom<Box<RawValue>> for ToolCall {
    type Error = serde_json::Error;

    fn try_from(raw: Box<RawValue>) -> Result<Self, Self::Error> {
        let (head, extra) = split_fields::<ToolCallHead>(raw.get())?;
        Ok(Self {
            id: head.id,
            function: head.function,
            extra,
        })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(try_from = "Box<RawValue>")]
pub struct ToolCallFunction {
    pub name: Option<String>,
    pub arguments: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Box<RawValue>>,
}

#[derive(Default, Deserialize, Serialize)]
struct ToolCallFunctionHead {
    name: Option<String>,
    arguments: Option<String>,
}

impl TryFrom<Box<RawValue>> for ToolCallFunction {
    type Error = serde_json::Error;

    fn try_from(raw: Box<RawValue>) -> Result<Self, Self::Error> {
        let (head, extra) = split_fields::<ToolCallFunctionHead>(raw.get())?;
        Ok(Self {
            name: head.name,
            arguments: head.arguments,
            extra,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LassoMessage {
    pub role: String,
    pub content: LassoContent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
#[serde(try_from = "Box<RawValue>")]
pub enum LassoContent {
    ToolUse(ToolUseBlock),
    ToolResult(ToolResultBlock),
    Text(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "Box<RawValue>")]
pub struct ToolUseBlock {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: Option<String>,
    pub name: Option<String>,
    pub input: Option<ToolArguments>,
}

#[derive(Deserialize)]
struct ToolUseBlockHead {
    #[serde(rename = "type")]
    kind: String,
    id: Option<String>,
    name: Option<String>,
    #[serde(default)]
    input: Option<Box<RawValue>>,
}

impl TryFrom<Box<RawValue>> for ToolUseBlock {
    type Error = serde_json::Error;

    fn try_from(raw: Box<RawValue>) -> Result<Self, Self::Error> {
        let head = serde_json::from_str::<ToolUseBlockHead>(raw.get())?;
        Ok(Self {
            kind: head.kind,
            id: head.id,
            name: head.name,
            input: head.input.as_deref().map(parse_tool_arguments),
        })
    }
}

fn parse_tool_arguments(value: &RawValue) -> ToolArguments {
    let Ok(parsed) = serde_json::from_str(value.get()) else {
        return ToolArguments::Malformed {
            arguments: value.get().to_owned(),
        };
    };
    ToolArguments::Parsed(parsed)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultBlock {
    #[serde(rename = "type")]
    pub kind: String,
    pub tool_use_id: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
}

#[derive(Deserialize)]
struct ContentTag {
    #[serde(rename = "type")]
    kind: String,
}

impl TryFrom<Box<RawValue>> for LassoContent {
    type Error = serde_json::Error;

    fn try_from(raw: Box<RawValue>) -> Result<Self, Self::Error> {
        if raw.get().starts_with('"') {
            return serde_json::from_str(raw.get()).map(Self::Text);
        }
        let tag = serde_json::from_str::<ContentTag>(raw.get())?;
        match tag.kind.as_str() {
            "tool_use" => serde_json::from_str(raw.get()).map(Self::ToolUse),
            "tool_result" => serde_json::from_str(raw.get()).map(Self::ToolResult),
            other => Err(serde_json::Error::custom(format!(
                "unknown Lasso content type `{other}`"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "Box<RawValue>")]
pub struct ModelResponseShape {
    pub choices: Vec<ResponseChoice>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Box<RawValue>>,
}

#[derive(Deserialize, Serialize)]
struct ModelResponseShapeHead {
    #[serde(default)]
    choices: Vec<ResponseChoice>,
}

impl TryFrom<Box<RawValue>> for ModelResponseShape {
    type Error = serde_json::Error;

    fn try_from(raw: Box<RawValue>) -> Result<Self, Self::Error> {
        let (head, extra) = split_fields::<ModelResponseShapeHead>(raw.get())?;
        Ok(Self {
            choices: head.choices,
            extra,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "Box<RawValue>")]
pub struct ResponseChoice {
    pub message: Option<ResponseMessage>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Box<RawValue>>,
}

#[derive(Deserialize, Serialize)]
struct ResponseChoiceHead {
    #[serde(default)]
    message: Option<ResponseMessage>,
}

impl TryFrom<Box<RawValue>> for ResponseChoice {
    type Error = serde_json::Error;

    fn try_from(raw: Box<RawValue>) -> Result<Self, Self::Error> {
        let (head, extra) = split_fields::<ResponseChoiceHead>(raw.get())?;
        Ok(Self {
            message: head.message,
            extra,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "Box<RawValue>")]
pub struct ResponseMessage {
    pub role: Role,
    pub content: Option<String>,
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Box<RawValue>>,
}

#[derive(Deserialize, Serialize)]
struct ResponseMessageHead {
    role: Role,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ToolCall>>,
}

impl TryFrom<Box<RawValue>> for ResponseMessage {
    type Error = serde_json::Error;

    fn try_from(raw: Box<RawValue>) -> Result<Self, Self::Error> {
        let (head, extra) = split_fields::<ResponseMessageHead>(raw.get())?;
        Ok(Self {
            role: head.role,
            content: head.content,
            tool_calls: head.tool_calls,
            extra,
        })
    }
}
