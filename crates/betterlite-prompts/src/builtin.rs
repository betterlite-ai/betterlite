use betterlite_logging::analytics::python::FormatError;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Parts(Vec<ContentPart>),
    Null,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct ContentPart {
    pub fields: IndexMap<String, Field>,
}

impl ContentPart {
    fn text(&self, key: &str) -> Option<&str> {
        match self.fields.get(key) {
            Some(Field::Text(text)) => Some(text),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Field {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    Text(String),
    List(Vec<Self>),
    Object(IndexMap<String, Self>),
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ToolFunction,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ToolFunction {
    pub name: String,
    pub arguments: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Message {
    pub role: String,
    pub content: Content,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Debug, Error)]
pub enum PromptError {
    #[error("message {index} has unsupported role {role}")]
    Role { index: usize, role: String },
    #[error("message {index} must contain text")]
    Text { index: usize },
    #[error("invalid tool-call arguments")]
    ToolArguments {
        #[source]
        source: serde_json::Error,
    },
    #[error("prompt exceeds its byte budget")]
    Limit,
    #[error("input exceeds its byte budget")]
    InputLimit(#[source] serde_json::Error),
    #[error("a final message is required")]
    Empty,
    #[error("prompt JSON cannot be serialized")]
    Json(#[from] serde_json::Error),
    #[error("prompt text cannot be formatted")]
    Format(#[from] fmt::Error),
    #[error("content cannot be formatted")]
    Repr(#[from] FormatError),
    #[error("inline image data is malformed")]
    Image,
    #[error("content part is missing {field}")]
    MissingPart { field: &'static str },
    #[error("remote image acquisition is not enabled")]
    RemoteImage,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct OllamaPrompt {
    pub prompt: String,
    pub images: Vec<String>,
}

mod bounds;
pub mod falcon;
mod families;
pub mod images;
pub mod ollama;
pub mod system;
mod tests;
mod text;

pub fn default_pt(messages: &[Message]) -> Result<String, PromptError> {
    bounds::check(messages)?;
    let texts = messages
        .iter()
        .enumerate()
        .map(|(index, message)| text::strict(message, index))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(texts.join(" "))
}

pub fn deepseek<Render>(messages: &[Message], render: Render) -> Result<String, PromptError>
where
    Render: FnOnce(&str, &[Message]) -> Result<String, PromptError>,
{
    bounds::check(messages)?;
    let output = render("deepseek-r1/deepseek-r1-7b-instruct", messages)?;
    bounds::finish(output)
}
macro_rules! templates {
    ($($name:ident: $renderer:ident($($argument:expr),*);)*) => {
        $(pub fn $name(messages: &[Message]) -> Result<String, PromptError> {
            let prompt = families::$renderer(messages, $($argument),*)?;
            bounds::finish(prompt)
        })*
    };
}

templates! {
    alpaca: custom(("<s>","","</s>"), &[("system","### Instruction:\n","\n\n"),("user","### Instruction:\n","\n\n"),("assistant","### Response:\n","\n\n")], false);
    llama2: custom(("<s>","","</s>"), &[("system","[INST] <<SYS>>\n","\n<</SYS>>\n [/INST]\n"),("user","[INST] "," [/INST]\n"),("assistant","","\n")], false);
    mistral: custom(("","<s>",""), &[("system","[INST] \n"," [/INST]\n"),("user","[INST] "," [/INST]\n"),("assistant"," ","</s> ")], false);
    granite: custom(("","",""), &[("system","<|system|>\n","\n"),("user","<|user|>\n","\n<|assistant|>\n"),("assistant","","\n")], true);
    falcon_chat: labelled(&[("system","System: "),("user","User: "),("assistant","Falcon: ")], "");
    mpt: labelled(&[("system","<|im_start|>system"),("user","<|im_start|>user"),("assistant","<|im_start|>assistant")], "<|im_end|>\n");
    wizardcoder: labelled(&[("system",""),("user","### Instruction:\n"),("assistant","### Response:\n")], "\n\n");
    phind: labelled(&[("system","### System Prompt\n"),("user","### User Message\n"),("assistant","### Assistant\n")], "\n\n");
}

pub fn claude(messages: &[Message]) -> Result<String, PromptError> {
    let style = families::Style::Claude;
    families::anthropic(messages, style)
}

pub fn claude_2_1(messages: &[Message]) -> Result<String, PromptError> {
    let style = families::Style::Claude21;
    families::anthropic(messages, style)
}

pub fn titan(messages: &[Message]) -> Result<String, PromptError> {
    let style = families::Style::Titan;
    families::anthropic(messages, style)
}
