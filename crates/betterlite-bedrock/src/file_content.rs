use betterlite_openai::openai_base_config::{Headers, Object};
use encoding_rs::UTF_8;
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BedrockError {
    pub status_code: u16,
    pub message: String,
    pub headers: Headers,
    pub response: RawResponse,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum FileContentError {
    Bedrock(Box<BedrockError>),
}

pub fn transform_file_response(
    raw_response: RawResponse,
) -> Result<HttpxBinaryResponseContent, FileContentError> {
    if raw_response.status_code >= 400 {
        let (decoded_message, _, _) = UTF_8.decode(&raw_response.body);
        let message = decoded_message.into_owned();
        return Err(FileContentError::Bedrock(Box::new(BedrockError {
            status_code: raw_response.status_code,
            message,
            headers: raw_response.headers.clone(),
            response: raw_response,
        })));
    }
    Ok(HttpxBinaryResponseContent {
        response: raw_response,
        hidden_params: Object::new(),
    })
}

mod tests;
