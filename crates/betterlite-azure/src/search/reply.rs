use serde::Deserialize;
use serde_json::Number;

use super::error::{AzureSearchError, ErrorBody, provider_error};

#[derive(Debug, Clone, Copy)]
pub struct AzureSearchReply<'reply> {
    pub status_code: u16,
    pub headers: &'reply [(String, String)],
    pub body: &'reply [u8],
}

#[derive(Debug, Clone, Deserialize)]
pub struct GroundingEnvelope {
    pub output: Vec<OutputItem>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub error: Option<ErrorBody>,
    #[serde(default)]
    pub incomplete_details: Option<IncompleteDetails>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OutputItem {
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub content: Vec<ContentPart>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ContentPart {
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub annotations: Vec<Annotation>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Annotation {
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub start_index: Option<JsonIndex>,
    // LiteLLM's schema validates the field even though no code reads it
    #[serde(default)]
    pub end_index: Option<JsonIndex>,
}

// LiteLLM holds any Python int here; an index outside 0..=len(text) ends the text to _snippet, and -0 is zero
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(try_from = "Number")]
pub struct JsonIndex(Option<u64>);

impl TryFrom<Number> for JsonIndex {
    type Error = &'static str;

    fn try_from(number: Number) -> Result<Self, Self::Error> {
        if number.is_f64() {
            return Err("a citation index must be an integer");
        }
        Ok(Self(
            number
                .as_u64()
                .or_else(|| u64::try_from(number.as_i64()?).ok()),
        ))
    }
}

impl JsonIndex {
    #[must_use]
    pub fn marker_start(self, length: usize) -> usize {
        let Some(position) = self.0 else {
            return length;
        };
        usize::try_from(position)
            .ok()
            .filter(|start| *start <= length)
            .unwrap_or(length)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct IncompleteDetails {
    #[serde(default)]
    pub reason: Option<String>,
}

pub fn decode_grounding_reply(
    reply: &AzureSearchReply<'_>,
) -> Result<GroundingEnvelope, AzureSearchError> {
    serde_json::from_slice(reply.body).map_err(|error| {
        provider_error(
            &format!("response does not match the Foundry Responses API schema: {error}"),
            reply.status_code,
            reply.headers,
        )
    })
}
