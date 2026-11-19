use std::collections::BTreeMap;
use std::future::Future;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::OracleLimits;
use super::call_checks::validate_timeout;
use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::kinds::SdkResult;

mod native;
mod support;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Sync,
    #[serde(rename = "async")]
    Asynchronous,
    Streaming,
}

impl Mode {
    const fn logging_call_type(self) -> &'static str {
        match self {
            Self::Sync => "file_content",
            Self::Asynchronous | Self::Streaming => "afile_content",
        }
    }
}

// The three handlers' inputs: which one runs, the file, the caller's headers and body, and the two streaming knobs
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub mode: Mode,
    pub file_id: String,
    #[serde(default)]
    pub extra_headers: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub extra_body: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub chunk_size: Option<usize>,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
}

// The exception get_error_class returns, as its callers can observe it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    pub status: u16,
    pub synthesized: bool,
    pub message: String,
    pub headers: BTreeMap<String, Json>,
}

// What transform_file_content_response returned, or the chunks and headers the stream yielded
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ContentValue {
    Buffered(BufferedContent),
    Streamed(StreamedContent),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BufferedContent {
    pub status: u16,
    pub headers: BTreeMap<String, Json>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamedContent {
    pub chunks: Vec<Vec<u8>>,
    pub headers: BTreeMap<String, Json>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub result: Option<ContentValue>,
    pub failure: Option<Failure>,
}

impl CallKind for Call {
    fn validate(&self, limits: &OracleLimits) -> Result<(), SdkHttpError> {
        if let Some(seconds) = self.timeout_seconds {
            validate_timeout(seconds, limits)?;
        }
        if self.mode == Mode::Streaming {
            return match self.chunk_size {
                Some(0) | None => Err(SdkHttpError::Invalid(String::from(
                    "a streaming content call needs a chunk_size of at least 1",
                ))),
                Some(_) => Ok(()),
            };
        }
        if self.chunk_size.is_some() {
            return Err(SdkHttpError::Invalid(String::from(
                "only a streaming content call takes a chunk_size",
            )));
        }
        Ok(())
    }

    fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> impl Future<Output = Result<SdkResult, SdkHttpError>> {
        native::execute(self, api_base, env)
    }
}
