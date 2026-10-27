use std::str;

use betterlite_openai::error::{ErrorResponse, ProviderError, ProviderErrorParts};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeaderValue {
    Text(String),
    Bytes(Vec<u8>),
    Integer(i64),
    Boolean(bool),
    Null,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorHeaders {
    Http(Vec<(String, String)>),
    Mapping(Vec<(String, HeaderValue)>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseBody {
    Read(String),
    Unread { reason_phrase: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorReply {
    pub response: ErrorResponse,
    pub headers: Vec<(String, String)>,
    pub body: ResponseBody,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorInput {
    pub base: ProviderErrorParts,
    pub headers: Option<ErrorHeaders>,
    pub response: Option<ErrorReply>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BedrockError {
    pub error: ProviderError,
    pub headers: Option<ErrorHeaders>,
    pub response_headers: Vec<(String, String)>,
    pub response_body: ResponseBody,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum HeaderError {
    #[error("header {name} cannot be encoded as ASCII")]
    NotAscii { name: String },
}

#[must_use]
pub fn error_response_text(body: &ResponseBody) -> &str {
    match body {
        ResponseBody::Read(text) => text,
        ResponseBody::Unread { reason_phrase } => reason_phrase,
    }
}

impl TryFrom<ErrorInput> for BedrockError {
    type Error = HeaderError;

    fn try_from(input: ErrorInput) -> Result<Self, HeaderError> {
        let (response, response_headers, response_body) = match input.response {
            Some(reply) => (Some(reply.response), reply.headers, reply.body),
            None => (
                None,
                synthesized_headers(input.headers.as_ref())?,
                ResponseBody::Read(String::new()),
            ),
        };
        // base.headers and base.response are replaced by the Bedrock views of the same inputs
        let parts = ProviderErrorParts {
            headers: input.headers.as_ref().map(text_headers),
            response,
            ..input.base
        };
        Ok(Self {
            error: ProviderError::from(parts),
            headers: input.headers,
            response_headers,
            response_body,
        })
    }
}

fn text_headers(headers: &ErrorHeaders) -> Vec<(String, String)> {
    match headers {
        ErrorHeaders::Http(pairs) => pairs.clone(),
        ErrorHeaders::Mapping(entries) => entries
            .iter()
            .filter_map(|(name, value)| match value {
                HeaderValue::Text(text) => Some((name.clone(), text.clone())),
                HeaderValue::Bytes(_)
                | HeaderValue::Integer(_)
                | HeaderValue::Boolean(_)
                | HeaderValue::Null => None,
            })
            .collect(),
    }
}

fn synthesized_headers(
    headers: Option<&ErrorHeaders>,
) -> Result<Vec<(String, String)>, HeaderError> {
    match headers {
        None => Ok(Vec::new()),
        Some(ErrorHeaders::Http(pairs)) => Ok(pairs.clone()),
        Some(ErrorHeaders::Mapping(entries)) => mapping_headers(entries),
    }
}

fn mapping_headers(
    entries: &[(String, HeaderValue)],
) -> Result<Vec<(String, String)>, HeaderError> {
    let mut kept = Vec::new();
    for (name, value) in entries {
        let bytes = match value {
            HeaderValue::Text(text) if text.is_ascii() => text.as_bytes(),
            HeaderValue::Bytes(bytes) => bytes.as_slice(),
            HeaderValue::Text(_) => return Err(HeaderError::NotAscii { name: name.clone() }),
            HeaderValue::Integer(_) | HeaderValue::Boolean(_) | HeaderValue::Null => continue,
        };
        if !name.is_ascii() {
            return Err(HeaderError::NotAscii { name: name.clone() });
        }
        kept.push((name.clone(), bytes));
    }
    // httpx picks one encoding for the whole header set: UTF-8 when every value decodes, else latin-1
    let utf8 = kept.iter().all(|(_, bytes)| str::from_utf8(bytes).is_ok());
    Ok(kept
        .into_iter()
        .map(|(name, bytes)| match str::from_utf8(bytes) {
            Ok(text) if utf8 => (name, text.to_owned()),
            Ok(_) | Err(_) => (name, bytes.iter().copied().map(char::from).collect()),
        })
        .collect())
}

mod tests;
