use std::str::Utf8Error;

use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use url::ParseError;

use crate::openai_base_config::Object;

pub const MAX_COLLECTED_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PassthroughError {
    #[error("the body bytes are not valid UTF-8")]
    InvalidUtf8 {
        #[source]
        source: Utf8Error,
    },
    #[error("the passthrough url {url} is not valid: {source}")]
    InvalidUrl {
        url: String,
        #[source]
        source: ParseError,
    },
    #[error("the query parameter {name} is not a string or a number")]
    QueryValue { name: String },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct PassthroughMetadata {
    #[serde(default)]
    pub model_group: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RelayedBody {
    Json(Object),
    Text(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RelayCallType {
    Aembedding,
    AimageGeneration,
    Arerank,
    Aresponses,
}

impl RelayCallType {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Aembedding => "aembedding",
            Self::AimageGeneration => "aimage_generation",
            Self::Arerank => "arerank",
            Self::Aresponses => "aresponses",
        }
    }
}

pub struct RelayShape<'suffix, T> {
    pub path_suffix: &'suffix str,
    pub call_type: RelayCallType,
    pub parse: fn(&Object) -> Option<T>,
}

pub trait PassthroughLogging {
    fn set_call_type(&mut self, call_type: RelayCallType);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PassthroughTarget<'call> {
    pub model: &'call str,
    pub custom_llm_provider: &'call str,
    pub endpoint: &'call str,
}

#[derive(Debug, Clone, Copy)]
pub struct RelayedResponse<'call> {
    pub status_code: u16,
    pub body: &'call [u8],
}

pub struct PassthroughUrlRequest<'call> {
    pub api_base: Option<&'call str>,
    pub api_key: Option<&'call SecretString>,
    pub target: PassthroughTarget<'call>,
    pub request_query_params: Option<&'call Object>,
    pub litellm_params: &'call Object,
}

pub struct PassthroughSigning<'call> {
    pub litellm_params: &'call Object,
    pub request_data: Option<&'call Object>,
    pub api_base: &'call str,
    pub model: Option<&'call str>,
}

pub(super) fn query_value_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Null | Value::Bool(_) | Value::Array(_) | Value::Object(_) => None,
    }
}
