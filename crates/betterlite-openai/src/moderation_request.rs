use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ModerationRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub input: ModerationInput,
    #[serde(flatten)]
    pub passthrough: IndexMap<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum ModerationInput {
    Text(String),
    Texts(Vec<String>),
    MultiModal(Vec<ModerationInputPart>),
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum ModerationInputPart {
    Image(ImagePart),
    Text(TextPart),
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ImagePart {
    pub image_url: ImageUrl,
    #[serde(rename = "type")]
    pub kind: ImageInputType,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ImageUrl {
    pub url: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub enum ImageInputType {
    #[serde(rename = "image_url")]
    ImageUrl,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TextPart {
    pub text: String,
    #[serde(rename = "type")]
    pub kind: TextInputType,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub enum TextInputType {
    #[serde(rename = "text")]
    Text,
}

#[derive(Debug, Error)]
pub enum ModerationRequestError {
    #[error("the request body is not a valid moderation request: {source}")]
    Body {
        #[source]
        source: serde_json::Error,
    },
    #[error("betterlite does not support the {name} moderation request parameter yet")]
    Unsupported { name: String },
    #[error("the moderation request could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

impl ModerationRequest {
    pub fn parse(bytes: &[u8]) -> Result<Self, ModerationRequestError> {
        let request: Self = serde_json::from_slice(bytes)
            .map_err(|source| ModerationRequestError::Body { source })?;
        if let Some(name) = request.passthrough.keys().next() {
            return Err(ModerationRequestError::Unsupported { name: name.clone() });
        }
        Ok(request)
    }

    pub fn upstream_body(&self, model: &str) -> Result<Vec<u8>, ModerationRequestError> {
        #[derive(Serialize)]
        struct UpstreamBody<'request> {
            input: &'request ModerationInput,
            model: &'request str,
        }
        serde_json::to_vec(&UpstreamBody {
            input: &self.input,
            model,
        })
        .map_err(|source| ModerationRequestError::Encode { source })
    }
}

mod tests;
