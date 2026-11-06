use std::collections::BTreeMap;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::presidio_response::anthropic::is_anthropic_message_response;
use crate::presidio_response::error::PresidioResponseError;

pub type Extensions = BTreeMap<String, Box<RawValue>>;

// A record keeps the fields it models typed and every other field, an explicit null included, in `extra`, so it serializes back to what it read.
macro_rules! object_record {
    ($name:ident { $($field:ident: $ty:ty => $key:literal),* $(,)? }) => {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        #[serde(try_from = "Extensions")]
        pub struct $name {
            $(#[serde(rename = $key, skip_serializing_if = "Option::is_none")]
            pub $field: Option<$ty>,)*
            #[serde(flatten)]
            pub extra: Extensions,
        }

        impl TryFrom<Extensions> for $name {
            type Error = serde_json::Error;

            fn try_from(mut extra: Extensions) -> Result<Self, Self::Error> {
                $(let $field = match extra.get($key) {
                    Some(raw) if raw.get().trim() != "null" => {
                        let parsed = serde_json::from_str(raw.get())?;
                        extra.remove($key);
                        Some(parsed)
                    }
                    Some(_) | None => None,
                };)*
                Ok(Self { $($field,)* extra })
            }
        }
    };
}

object_record!(ChatMessage { content: Content => "content" });
object_record!(ChatChoice { message: ChatResponseMessage => "message" });
object_record!(ChatResponseMessage {
    content: Content => "content",
    tool_calls: Vec<ToolCallItem> => "tool_calls",
    function_call: FunctionCall => "function_call",
});
object_record!(ToolCall { function: Function => "function" });

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "Extensions")]
pub struct ChatModelResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub choices: Option<Vec<ChatChoice>>,
    #[serde(flatten)]
    pub extra: Extensions,
}

impl TryFrom<Extensions> for ChatModelResponse {
    type Error = serde_json::Error;

    fn try_from(mut extra: Extensions) -> Result<Self, Self::Error> {
        let choices = take_list(&mut extra, "choices")?;
        Ok(Self { choices, extra })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "Extensions")]
pub struct AnthropicMessage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<ContentItem>>,
    #[serde(flatten)]
    pub extra: Extensions,
}

impl TryFrom<Extensions> for AnthropicMessage {
    type Error = serde_json::Error;

    fn try_from(mut extra: Extensions) -> Result<Self, Self::Error> {
        let content = take_list(&mut extra, "content")?;
        Ok(Self { content, extra })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged, try_from = "Box<RawValue>")]
pub enum Content {
    Text(String),
    Blocks(Vec<ContentItem>),
    Other(Box<RawValue>),
}

impl TryFrom<Box<RawValue>> for Content {
    type Error = serde_json::Error;

    fn try_from(value: Box<RawValue>) -> Result<Self, Self::Error> {
        match value.get().trim_start().as_bytes().first() {
            Some(b'"') => serde_json::from_str(value.get()).map(Self::Text),
            Some(b'[') => serde_json::from_str(value.get()).map(Self::Blocks),
            Some(_) | None => Ok(Self::Other(value)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged, try_from = "Box<RawValue>")]
pub enum ContentItem {
    Object(ContentPart),
    Other(Box<RawValue>),
}

impl TryFrom<Box<RawValue>> for ContentItem {
    type Error = serde_json::Error;

    fn try_from(value: Box<RawValue>) -> Result<Self, Self::Error> {
        if value.get().trim_start().starts_with('{') {
            return serde_json::from_str(value.get()).map(Self::Object);
        }
        Ok(Self::Other(value))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "Extensions")]
pub struct ContentPart {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(flatten)]
    pub extra: Extensions,
}

impl TryFrom<Extensions> for ContentPart {
    type Error = serde_json::Error;

    fn try_from(mut extra: Extensions) -> Result<Self, Self::Error> {
        let kind = take_string(&mut extra, "type");
        let text = take_string(&mut extra, "text");
        Ok(Self { kind, text, extra })
    }
}

impl ContentPart {
    pub fn set_text(&mut self, text: String) {
        self.extra.remove("text");
        self.text = Some(text);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged, try_from = "Box<RawValue>")]
pub enum ToolCallItem {
    Object(ToolCall),
    Other(Box<RawValue>),
}

impl TryFrom<Box<RawValue>> for ToolCallItem {
    type Error = serde_json::Error;

    fn try_from(value: Box<RawValue>) -> Result<Self, Self::Error> {
        if value.get().trim_start().starts_with('{') {
            return serde_json::from_str(value.get()).map(Self::Object);
        }
        Ok(Self::Other(value))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "Extensions")]
pub struct Function {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
    #[serde(flatten)]
    pub extra: Extensions,
}

impl TryFrom<Extensions> for Function {
    type Error = serde_json::Error;

    fn try_from(mut extra: Extensions) -> Result<Self, Self::Error> {
        let arguments = take_string(&mut extra, "arguments");
        Ok(Self { arguments, extra })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "Extensions")]
pub struct FunctionCall {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
    #[serde(flatten)]
    pub extra: Extensions,
}

impl TryFrom<Extensions> for FunctionCall {
    type Error = serde_json::Error;

    fn try_from(mut extra: Extensions) -> Result<Self, Self::Error> {
        let arguments = take_string(&mut extra, "arguments");
        Ok(Self { arguments, extra })
    }
}

#[derive(Debug, Clone)]
pub enum ResponseBody {
    Chat(ChatModelResponse),
    Stream(Box<RawValue>),
    Anthropic(AnthropicMessage),
    Other(Box<RawValue>),
}

impl ResponseBody {
    pub fn from_raw(raw: Box<RawValue>) -> Result<Self, PresidioResponseError> {
        let Ok(fields) = serde_json::from_str::<Extensions>(raw.get()) else {
            return Ok(Self::Other(raw));
        };
        if is_anthropic_message_response(&raw) {
            let parsed = serde_json::from_str(raw.get())
                .map_err(|source| PresidioResponseError::BodyJson { source })?;
            return Ok(Self::Anthropic(parsed));
        }
        let is_chunk = fields
            .get("object")
            .and_then(|value| serde_json::from_str::<String>(value.get()).ok())
            .is_some_and(|object| object == "chat.completion.chunk");
        if is_chunk {
            return Ok(Self::Stream(raw));
        }
        if fields
            .get("choices")
            .is_some_and(|value| value.get().trim_start().starts_with('['))
        {
            let parsed = serde_json::from_str(raw.get())
                .map_err(|source| PresidioResponseError::BodyJson { source })?;
            return Ok(Self::Chat(parsed));
        }
        Ok(Self::Other(raw))
    }

    pub fn into_raw(self) -> Result<Box<RawValue>, PresidioResponseError> {
        match self {
            Self::Chat(chat) => to_raw(&chat),
            Self::Anthropic(message) => to_raw(&message),
            Self::Stream(raw) | Self::Other(raw) => Ok(raw),
        }
    }
}

pub(crate) fn to_raw<Value>(value: &Value) -> Result<Box<RawValue>, PresidioResponseError>
where
    Value: Serialize + ?Sized,
{
    let encoded = serde_json::to_string(value)
        .map_err(|source| PresidioResponseError::BodyJson { source })?;
    RawValue::from_string(encoded).map_err(|source| PresidioResponseError::BodyJson { source })
}

fn take_string(extra: &mut Extensions, key: &str) -> Option<String> {
    let raw = extra.get(key)?;
    let value = serde_json::from_str(raw.get()).ok()?;
    extra.remove(key);
    Some(value)
}

fn take_list<T>(extra: &mut Extensions, key: &str) -> Result<Option<Vec<T>>, serde_json::Error>
where
    T: DeserializeOwned,
{
    let Some(raw) = extra.get(key) else {
        return Ok(None);
    };
    if !raw.get().trim_start().starts_with('[') {
        return Ok(None);
    }
    let parsed = serde_json::from_str(raw.get())?;
    extra.remove(key);
    Ok(Some(parsed))
}
