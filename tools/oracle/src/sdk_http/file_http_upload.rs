use std::collections::BTreeMap;
use std::future::Future;

use betterlite_models::decisions::Json;
use betterlite_openai::api::file_type_records::UploadUrlLocation;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::kinds::SdkResult;

mod native;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunMode {
    Sync,
    #[serde(rename = "async")]
    Asynchronous,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UploadMethod {
    #[default]
    Post,
    Put,
}

// A body the provider transform produced: text reaches httpx as a string, raw content as bytes
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Raw(Vec<u8>),
}

impl Content {
    // `self` is borrowed so a plan can log the same value it sends
    #[must_use]
    pub fn bytes(&self) -> Vec<u8> {
        match self {
            Self::Text(text) => text.as_bytes().to_vec(),
            Self::Raw(bytes) => bytes.clone(),
        }
    }
}

// The four handlers' input: the mode, the caller's request pieces, and the plan create_file receives
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub mode: RunMode,
    #[serde(default)]
    pub upload_method: UploadMethod,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub litellm_params: IndexMap<String, Json>,
    pub file: FileSpec,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
    pub plan: Plan,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileSpec {
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default)]
    pub content_type: Option<String>,
    pub content: Content,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Plan {
    TwoStep(Box<TwoStepSpec>),
    Presigned(PresignedSpec),
    Media(MediaSpec),
    Multipart(MultipartSpec),
    Text(String),
    Raw(Vec<u8>),
}

// initial_request wins over method, method over streaming_media_upload, then bytes/string, then file
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TwoStepSpec {
    pub initial_request: InitialRequestSpec,
    pub upload_request: UploadRequestSpec,
    #[serde(default)]
    pub upload_url_location: Option<UploadUrlLocation>,
    #[serde(default)]
    pub upload_url_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InitialRequestSpec {
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    pub data: IndexMap<String, Json>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UploadRequestSpec {
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    pub data: Content,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresignedSpec {
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    pub data: Content,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaSpec {
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub chunks: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MultipartSpec {
    pub parts: Vec<PartSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartSpec {
    pub name: String,
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default)]
    pub content_type: Option<String>,
    pub content: Content,
}

// The exception _handle_error raises, as its callers can observe it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    pub status: u16,
    pub synthesized: bool,
    pub message: String,
    pub headers: BTreeMap<String, Json>,
}

// What the async handler handed pre_call: the input and key, and the additional args it kept
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Diagnostics {
    pub input: Json,
    pub api_key: Json,
    pub api_base: Json,
    pub headers: Json,
    pub complete_input_dict: Json,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub result: Option<Json>,
    pub failure: Option<Failure>,
    pub diagnostics: Option<Diagnostics>,
}

impl CallKind for Call {
    fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> impl Future<Output = Result<SdkResult, SdkHttpError>> {
        native::execute(self, api_base, env)
    }
}
