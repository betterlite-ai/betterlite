use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::files::error::FilesError;
use crate::files::stream::flatten_headers;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilesPurpose {
    #[serde(rename = "assistants")]
    Assistants,
    #[serde(rename = "assistants_output")]
    AssistantsOutput,
    #[serde(rename = "batch")]
    Batch,
    #[serde(rename = "batch_output")]
    BatchOutput,
    #[serde(rename = "fine-tune")]
    FineTune,
    #[serde(rename = "fine-tune-results")]
    FineTuneResults,
    #[serde(rename = "vision")]
    Vision,
    #[serde(rename = "user_data")]
    UserData,
    #[serde(rename = "evals")]
    Evals,
    #[serde(rename = "messages")]
    Messages,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileStatus {
    Uploaded,
    Processed,
    Error,
    Pending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileKind {
    File,
}

// LiteLLM's OpenAIFileObject: the create path revalidates the SDK object into this record
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileObject {
    pub id: String,
    pub bytes: i64,
    pub created_at: i64,
    pub filename: String,
    pub object: FileKind,
    pub purpose: FilesPurpose,
    #[serde(default)]
    pub status: Option<FileStatus>,
    #[serde(default)]
    pub expires_at: Option<i64>,
    #[serde(default)]
    pub status_details: Option<String>,
}

pub fn decode_file_object(body: &[u8]) -> Result<FileObject, FilesError> {
    match serde_json::from_slice(body) {
        Ok(object) => Ok(object),
        Err(source) => Err(FilesError::Decode { source }),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreatePurpose {
    Assistants,
    Batch,
    FineTune,
    Messages,
}

impl CreatePurpose {
    #[must_use]
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::Assistants => "assistants",
            Self::Batch => "batch",
            Self::FineTune => "fine-tune",
            Self::Messages => "messages",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileUpload {
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpiresAfter {
    pub seconds: i64,
}

// LiteLLM's CreateFileRequest: every field reaches files.create
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileCreate {
    pub file: FileUpload,
    pub purpose: CreatePurpose,
    pub expires_after: Option<ExpiresAfter>,
    pub extra_headers: IndexMap<String, String>,
    pub extra_body: IndexMap<String, String>,
    pub timeout: Option<Duration>,
}

// LiteLLM's FileContentRequest; the SDK drops extra_body on a GET, so it never reaches the wire
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileContentRequest {
    pub file_id: String,
    pub extra_headers: IndexMap<String, String>,
    pub extra_body: IndexMap<String, String>,
    pub timeout: Option<Duration>,
}

// list_files forwards purpose only; after, limit and order are not accepted by the wrapper
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileQuery {
    pub purpose: Option<String>,
}

// LiteLLM's HttpxBinaryResponseContent: the raw file bytes with the response status and headers
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryContent {
    pub status: u16,
    pub headers: IndexMap<String, String>,
    pub body: Vec<u8>,
}

impl BinaryContent {
    #[must_use]
    pub fn from_response(reply: UpstreamResponse) -> Self {
        Self {
            status: reply.status,
            headers: flatten_headers(&reply.headers),
            body: reply.body,
        }
    }
}
