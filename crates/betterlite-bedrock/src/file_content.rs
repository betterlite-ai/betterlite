use betterlite_openai::error::{ErrorRequest, ErrorResponse};
use betterlite_openai::openai_base_config::Object;
use encoding_rs::{Encoding, UTF_8};
use mime::Mime;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::bedrock_settings::{
    BedrockError, ErrorHeaders, ErrorInput, ErrorReply, HeaderError, ResponseBody,
};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct FileContentRequest {
    pub method: String,
    pub url: String,
}

// A list, not a map: Bedrock's HTTP response can repeat a header name, and httpx keeps every value.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct RawResponse {
    pub status_code: u16,
    pub body: Vec<u8>,
    pub headers: Vec<(String, String)>,
    pub request: FileContentRequest,
}

fn header_value<'headers>(
    headers: &'headers [(String, String)],
    name: &str,
) -> Option<&'headers str> {
    for (key, value) in headers {
        if key.eq_ignore_ascii_case(name) {
            return Some(value.as_str());
        }
    }
    None
}

// bedrock_settings.rs is B682's and gives BedrockError no Display/Error, so this derived wrapper carries it instead.
#[derive(Debug, Error)]
#[error("bedrock provider error")]
pub struct BedrockFailure {
    pub cause: Box<BedrockError>,
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
        let content_type = header_value(&self.response.headers, "content-type").map(str::to_owned);
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
        // litellm's exception keeps the raw_response object itself; this carries those bytes too.
        body: Vec<u8>,
        #[source]
        error: BedrockFailure,
    },
    #[error(transparent)]
    Header(#[from] HeaderError),
}

pub fn transform_file_response(
    raw_response: RawResponse,
) -> Result<HttpxBinaryResponseContent, FileContentError> {
    if raw_response.status_code >= 400 {
        let message = decode_response_text(&raw_response.body, &raw_response.headers);
        let request = ErrorRequest {
            method: raw_response.request.method.clone(),
            url: raw_response.request.url.clone(),
        };
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
            headers: Some(ErrorHeaders::Http(raw_response.headers.clone())),
            response: Some(ErrorReply {
                response,
                headers: raw_response.headers.clone(),
                body: ResponseBody::Read(message),
            }),
        })
        .map_err(FileContentError::Header)?;
        return Err(FileContentError::Bedrock {
            status: raw_response.status_code,
            body: raw_response.body,
            error: BedrockFailure {
                cause: Box::new(error),
            },
        });
    }
    Ok(HttpxBinaryResponseContent {
        response: raw_response,
        hidden_params: Object::new(),
    })
}

// Matches httpx.Response.text: the Content-Type charset, falling back to UTF-8, replacing malformed bytes.
fn decode_response_text(bytes: &[u8], headers: &[(String, String)]) -> String {
    let charset = header_value(headers, "content-type")
        .and_then(|value| value.parse::<Mime>().ok())
        .and_then(|content_type| {
            content_type
                .get_param(mime::CHARSET)
                .map(|name| name.to_string())
        });
    let encoding = charset
        .as_deref()
        .and_then(|name| Encoding::for_label(name.as_bytes()))
        .unwrap_or(UTF_8);
    encoding.decode_without_bom_handling(bytes).0.into_owned()
}

mod tests;
