use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

pub type ImageParams = IndexMap<String, Json>;
pub type ImageHeaders = BTreeMap<String, String>;
pub type ImageFiles = Vec<(String, ImageFile)>;

// LiteLLM passes FileTypes to the transforms untouched; a route reads uploads into a named byte part
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImageFile {
    pub filename: Option<String>,
    pub content: Vec<u8>,
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImageHttpResponse {
    pub status_code: u16,
    pub headers: ImageHeaders,
    pub body: Vec<u8>,
}

// httpx's RequestFiles: a sequence of named file parts beside the non-file data
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImageRequestFields {
    pub data: Option<ImageParams>,
    pub params: Option<ImageParams>,
    pub files: Option<ImageFiles>,
    pub content: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub b64_json: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revised_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageInputTokens {
    #[serde(default)]
    pub image_tokens: i64,
    #[serde(default)]
    pub text_tokens: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageUsage {
    #[serde(default)]
    pub input_tokens: i64,
    #[serde(default)]
    pub input_tokens_details: ImageInputTokens,
    #[serde(default)]
    pub output_tokens: i64,
    #[serde(default)]
    pub total_tokens: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageResponse {
    #[serde(default)]
    pub created: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Vec<ImageData>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<ImageUsage>,
}

// the kinds of own class-dict entry BaseImageEditConfig.get_config tells apart
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageClassKind {
    Value,
    Function,
    BuiltinFunction,
    ClassMethod,
    StaticMethod,
    Property,
    CallableObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ImageClassEntry {
    pub name: String,
    pub kind: ImageClassKind,
    pub value: Json,
    pub inherited: bool,
}

// executable class-dict objects reach the caller as their kind, never as a serialized value
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ImageConfigValue {
    Value { value: Json },
    Property,
    CallableObject,
}

pub type ImageConfig = IndexMap<String, ImageConfigValue>;
