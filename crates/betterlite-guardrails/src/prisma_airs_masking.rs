mod guardrails;
pub mod mcp;

use serde::{Deserialize, Serialize};
use serde_json::value::{RawValue, to_raw_value};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MaskingError {
    #[error("structured MCP arguments require masked JSON: {0}")]
    InvalidMcpJson(#[source] serde_json::Error),
    #[error("MCP request has no rewritable arguments field")]
    MissingMcpArguments,
    #[error("failed to encode masked content: {0}")]
    Encoding(#[source] serde_json::Error),
}

#[derive(Debug, Deserialize)]
pub struct ScanResult {
    pub prompt_masked_data: Option<MaskedData>,
    pub response_masked_data: Option<MaskedData>,
}

#[derive(Debug, Deserialize)]
pub struct MaskedData {
    pub data: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(try_from = "RequestMessageWire")]
pub struct RequestMessage {
    pub role: Option<String>,
    pub content: Option<MaskContent>,
}

#[derive(Debug, Deserialize)]
struct RequestMessageWire {
    role: Option<String>,
    content: Option<Box<RawValue>>,
}

impl TryFrom<RequestMessageWire> for RequestMessage {
    type Error = serde_json::Error;

    fn try_from(wire: RequestMessageWire) -> Result<Self, Self::Error> {
        Ok(Self {
            role: wire.role,
            content: parse_mask_content(wire.content)?,
        })
    }
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum MaskContent {
    Text(String),
    Parts(Vec<Box<RawValue>>),
}

fn parse_mask_content(
    content: Option<Box<RawValue>>,
) -> Result<Option<MaskContent>, serde_json::Error> {
    let Some(raw) = content else { return Ok(None) };
    if raw.get().trim_start().starts_with('[') {
        Ok(Some(MaskContent::Parts(serde_json::from_str(raw.get())?)))
    } else {
        Ok(Some(MaskContent::Text(serde_json::from_str(raw.get())?)))
    }
}

#[derive(Debug, Deserialize)]
pub struct Response {
    pub choices: Option<Vec<ResponseChoice>>,
}

#[derive(Debug, Deserialize)]
pub struct ResponseChoice {
    pub message: Option<MaskedResponseMessage>,
}

#[derive(Debug, Deserialize)]
#[serde(try_from = "MaskedResponseMessageWire")]
pub struct MaskedResponseMessage {
    pub content: Option<MaskContent>,
    pub tool_calls: Option<Vec<MaskedToolCall>>,
    pub function_call: Option<MaskedFunctionCall>,
}

#[derive(Debug, Deserialize)]
struct MaskedResponseMessageWire {
    content: Option<Box<RawValue>>,
    tool_calls: Option<Vec<MaskedToolCall>>,
    function_call: Option<MaskedFunctionCall>,
}

impl TryFrom<MaskedResponseMessageWire> for MaskedResponseMessage {
    type Error = serde_json::Error;

    fn try_from(wire: MaskedResponseMessageWire) -> Result<Self, Self::Error> {
        Ok(Self {
            content: parse_mask_content(wire.content)?,
            tool_calls: wire.tool_calls,
            function_call: wire.function_call,
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct MaskedToolCall {
    pub function: Option<MaskedFunctionCall>,
}

#[derive(Debug, Deserialize)]
pub struct MaskedFunctionCall {
    pub arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TextPart {
    #[serde(rename = "type")]
    kind: Option<String>,
}

#[derive(Serialize)]
struct MaskedTextPart<'text> {
    #[serde(rename = "type")]
    kind: &'static str,
    text: &'text str,
}

#[must_use]
pub fn get_masked_text(scan_result: &ScanResult, is_response: bool) -> Option<&str> {
    let masked_data = if is_response {
        scan_result.response_masked_data.as_ref()
    } else {
        scan_result.prompt_masked_data.as_ref()
    }?;
    masked_data.data.as_deref()
}

pub fn mask_messages(
    messages: &mut [RequestMessage],
    masked_text: &str,
) -> Result<bool, MaskingError> {
    messages
        .iter_mut()
        .rfind(|message| message.role.as_deref() == Some("user"))
        .and_then(|message| message.content.as_mut())
        .map_or(Ok(false), |content| {
            mask_content(content, masked_text).map(|()| true)
        })
}

pub fn mask_response(response: &mut Response, masked_text: &str) -> Result<(), MaskingError> {
    let Some(choices) = response.choices.as_mut() else {
        return Ok(());
    };
    choices.iter_mut().try_fold((), |(), choice| {
        choice.message.as_mut().map_or(Ok(()), |message| {
            mask_response_message(message, masked_text)
        })
    })
}

fn mask_response_message(
    message: &mut MaskedResponseMessage,
    masked_text: &str,
) -> Result<(), MaskingError> {
    if let Some(content) = message.content.as_mut() {
        mask_content(content, masked_text)?;
    }
    let replacements = message
        .tool_calls
        .iter_mut()
        .flatten()
        .filter_map(|call| call.function.as_mut())
        .chain(message.function_call.iter_mut());
    for function in replacements {
        if function.arguments.is_some() {
            function.arguments = Some(masked_text.to_owned());
        }
    }
    Ok(())
}

fn mask_content(content: &mut MaskContent, masked_text: &str) -> Result<(), MaskingError> {
    match content {
        MaskContent::Text(text) => masked_text.clone_into(text),
        MaskContent::Parts(parts) => return mask_parts(parts, masked_text),
    }
    Ok(())
}

fn mask_parts(parts: &mut Vec<Box<RawValue>>, masked_text: &str) -> Result<(), MaskingError> {
    let mut masked = Vec::with_capacity(parts.len());
    for part in parts.drain(..) {
        if serde_json::from_str::<TextPart>(part.get())
            .is_ok_and(|text_part| text_part.kind.as_deref() == Some("text"))
        {
            masked.push(
                to_raw_value(&MaskedTextPart {
                    kind: "text",
                    text: masked_text,
                })
                .map_err(MaskingError::Encoding)?,
            );
        } else {
            masked.push(part);
        }
    }
    *parts = masked;
    Ok(())
}
