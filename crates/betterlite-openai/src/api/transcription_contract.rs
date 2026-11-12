use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::openai_base_config::Object;

// LiteLLM's request data is a form or JSON object, or the raw audio bytes
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AudioTranscriptionData {
    Object(Object),
    Bytes(Vec<u8>),
}

// A multipart file value is either text or raw bytes, as LiteLLM's files dict allows
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AudioTranscriptionFileValue {
    Text(String),
    Bytes(Vec<u8>),
}

pub type AudioTranscriptionFiles = BTreeMap<String, AudioTranscriptionFileValue>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioTranscriptionRequestData {
    pub data: AudioTranscriptionData,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<AudioTranscriptionFiles>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
}

// TranscriptionResponse is an OpenAIObject, so text is its one declared field and providers add the rest
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptionResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(flatten)]
    pub extra: Object,
}
