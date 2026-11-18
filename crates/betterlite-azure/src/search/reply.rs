use serde::Deserialize;

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
    pub start_index: Option<i64>,
    // LiteLLM's schema validates the field even though no code reads it
    #[serde(default)]
    pub end_index: Option<i64>,
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
