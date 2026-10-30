use betterlite_openai::error::{ErrorRequest, ErrorResponse};
use betterlite_openai::openai_base_config::{Headers, Object};
use serde::{Deserialize, Serialize};
use std::str;
use thiserror::Error;

use crate::bedrock_settings::{
    BedrockError, ErrorHeaders, ErrorInput, ErrorReply, HeaderError, ResponseBody,
};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct FileContentRequest {
    pub method: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct RawResponse {
    pub status_code: u16,
    pub body: Vec<u8>,
    pub headers: Headers,
    pub request: FileContentRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HttpxBinaryResponseContent {
    pub response: RawResponse,
    pub hidden_params: Object,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BinaryResponseSummary {
    pub object: &'static str,
    pub content_type: Option<String>,
    pub num_bytes: usize,
}

impl HttpxBinaryResponseContent {
    #[must_use]
    pub fn logging_summary(&self) -> BinaryResponseSummary {
        let content_type = self.response.headers.get("content-type").cloned();
        let num_bytes = self.response.body.len();
        BinaryResponseSummary {
            object: "binary",
            content_type,
            num_bytes,
        }
    }
}

#[derive(Debug, Error)]
pub enum FileContentError {
    #[error("bedrock error ({status})")]
    Bedrock {
        status: u16,
        error: Box<BedrockError>,
    },
    #[error(transparent)]
    Header(#[from] HeaderError),
}

pub fn transform_file_response(
    raw_response: RawResponse,
) -> Result<HttpxBinaryResponseContent, FileContentError> {
    if raw_response.status_code >= 400 {
        let message = decode_response_text(&raw_response.body);
        let request = ErrorRequest {
            method: raw_response.request.method.clone(),
            url: raw_response.request.url.clone(),
        };
        let headers: Vec<(String, String)> = raw_response.headers.into_iter().collect();
        let response = ErrorResponse {
            status_code: raw_response.status_code,
            request,
        };
        let error = BedrockError::try_from(ErrorInput {
            status_code: raw_response.status_code,
            message: message.clone(),
            request: Some(response.request.clone()),
            body: None,
            status_code_is_synthesized: false,
            headers: Some(ErrorHeaders::Http(headers.clone())),
            response: Some(ErrorReply {
                response,
                headers,
                body: ResponseBody::Read(message),
            }),
        })
        .map_err(FileContentError::Header)?;
        return Err(FileContentError::Bedrock {
            status: raw_response.status_code,
            error: Box::new(error),
        });
    }
    Ok(HttpxBinaryResponseContent {
        response: raw_response,
        hidden_params: Object::new(),
    })
}

fn decode_response_text(bytes: &[u8]) -> String {
    let mut text = String::new();
    let mut remaining = bytes;
    while !remaining.is_empty() {
        match str::from_utf8(remaining) {
            Ok(valid) => {
                text.push_str(valid);
                break;
            }
            Err(error) => {
                let valid_length = error.valid_up_to();
                let valid_bytes = remaining.get(..valid_length).unwrap_or_default();
                text.push_str(str::from_utf8(valid_bytes).unwrap_or_default());
                text.push('\u{fffd}');
                let invalid_length = error
                    .error_len()
                    .unwrap_or_else(|| remaining.len().saturating_sub(valid_length));
                let consumed = valid_length
                    .checked_add(invalid_length)
                    .unwrap_or(remaining.len());
                remaining = remaining.get(consumed..).unwrap_or_default();
            }
        }
    }
    text
}

mod tests;
