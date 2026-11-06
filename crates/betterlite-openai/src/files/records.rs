use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use indexmap::IndexMap;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::files::error::{FilesError, StatusError};
use crate::files::stream::flatten_headers;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilesPurpose {
    Assistants,
    AssistantsOutput,
    Batch,
    BatchOutput,
    #[serde(rename = "fine-tune")]
    FineTune,
    #[serde(rename = "fine-tune-results")]
    FineTuneResults,
    Vision,
    UserData,
    Evals,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<FileStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_details: Option<String>,
}

// The SDK's FileDeleted
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDeleted {
    pub id: String,
    pub object: FileKind,
    pub deleted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ListKind {
    List,
}

// The SDK's cursor page of file objects, with the list fields the files API reference declares
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilePage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object: Option<ListKind>,
    pub data: Vec<FileObject>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_more: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_id: Option<String>,
}

// Every JSON files reply: a non-2xx status is the SDK's status error, a 2xx body is the typed record
pub fn decode_file_object<R>(status: u16, body: &[u8]) -> Result<R, FilesError>
where
    R: DeserializeOwned,
{
    if !(200..300).contains(&status) {
        let failure = StatusError::from_response(status, body);
        return Err(FilesError::Status(Box::new(failure)));
    }
    serde_json::from_slice(body).map_err(|source| FilesError::Decode { source })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
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

impl FileQuery {
    // The SDK's query encoder drops an empty purpose
    #[must_use]
    pub fn pairs(&self) -> Vec<(String, String)> {
        self.purpose
            .iter()
            .filter(|purpose| !purpose.is_empty())
            .map(|purpose| ("purpose".to_owned(), purpose.clone()))
            .collect()
    }
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
