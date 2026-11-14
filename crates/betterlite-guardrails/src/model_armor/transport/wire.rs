use serde::{Deserialize, Serialize};

use crate::model_armor::file_scanning::ByteDataType;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    UserPrompt,
    ModelResponse,
}

impl Source {
    #[must_use]
    pub const fn method(self) -> &'static str {
        match self {
            Self::UserPrompt => "sanitizeUserPrompt",
            Self::ModelResponse => "sanitizeModelResponse",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SanitizeBody {
    UserPromptData(PromptData),
    ModelResponseData(PromptData),
}

impl SanitizeBody {
    #[must_use]
    pub fn wrap(source: Source, data: PromptData) -> Self {
        match source {
            Source::UserPrompt => Self::UserPromptData(data),
            Source::ModelResponse => Self::ModelResponseData(data),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PromptData {
    Text(TextPrompt),
    Bytes(ByteItem),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextPrompt {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ByteItem {
    pub byte_item: BytePayload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BytePayload {
    pub byte_data_type: ByteDataType,
    pub byte_data: String,
}
