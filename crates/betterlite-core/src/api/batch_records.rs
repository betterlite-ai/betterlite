use std::path::PathBuf;

use betterlite_error::proxy_exception::ProxyException;
use betterlite_openai::api::batch_records::{
    BatchRecordsError as WireError, BatchRow, UnparsedTail,
};
use betterlite_openai::files::records::FilesPurpose;
use thiserror::Error;

use crate::api::api_boundaries::ApiLimits;
use crate::routes::CallType;

mod jsonl;
mod limit;
mod metadata;
mod tests;

// FileTypes reduced to the two forms the rewrite acts on: a path, which is returned as it came, and the bytes of a file
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchFileContent {
    Bytes(Vec<u8>),
    Path(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchRecordsInput {
    ParseJsonl {
        content: Vec<u8>,
    },
    ReplaceModel {
        file: BatchFileContent,
        new_model_name: String,
    },
    ShouldReplaceModel {
        purpose: FilesPurpose,
        passthrough: bool,
    },
    RouterMetadataName {
        function_name: Option<String>,
    },
    IsBatchRetrieveCallType {
        call_type: CallType,
    },
    ValidateListLimit {
        limit: Option<i64>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InMemoryFile {
    pub content: Vec<u8>,
    pub name: &'static str,
    pub content_type: &'static str,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ReplacedContent {
    #[default]
    Original,
    Rewritten(InMemoryFile),
}

// logged_preview is the error line LiteLLM writes when a row never parsed: the first 100 characters of the buffer
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReplacedFile {
    pub content: ReplacedContent,
    pub logged_preview: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchRecordsResult {
    Rows(Vec<BatchRow>),
    Replaced(ReplacedFile),
    ShouldReplace(bool),
    MetadataName(&'static str),
    BatchRetrieve(bool),
    ListLimitAccepted,
}

#[derive(Debug, Error)]
pub enum BatchRecordsError {
    #[error(transparent)]
    Decode(#[from] WireError),
    #[error("error parsing final buffer: {}..., error: {}", .0.preview, .0.error)]
    Unparsed(UnparsedTail),
    #[error(transparent)]
    Encode(#[from] serde_json::Error),
    #[error(transparent)]
    ListLimit(Box<ProxyException>),
}

// the resolved limits bound every file the service reads; LiteLLM has no bound
#[derive(Debug, Clone, Copy)]
pub struct BatchRecords {
    pub limits: ApiLimits,
}

impl BatchRecords {
    pub fn run(&self, input: BatchRecordsInput) -> Result<BatchRecordsResult, BatchRecordsError> {
        match input {
            BatchRecordsInput::ParseJsonl { content } => {
                jsonl::parse_jsonl(&self.limits, &content).map(BatchRecordsResult::Rows)
            }
            BatchRecordsInput::ReplaceModel {
                file,
                new_model_name,
            } => jsonl::replace_model(&self.limits, file, &new_model_name)
                .map(BatchRecordsResult::Replaced),
            BatchRecordsInput::ShouldReplaceModel {
                purpose,
                passthrough,
            } => Ok(BatchRecordsResult::ShouldReplace(
                purpose == FilesPurpose::Batch && !passthrough,
            )),
            BatchRecordsInput::RouterMetadataName { function_name } => {
                Ok(BatchRecordsResult::MetadataName(
                    metadata::router_metadata_variable_name(function_name.as_deref()),
                ))
            }
            BatchRecordsInput::IsBatchRetrieveCallType { call_type } => {
                Ok(BatchRecordsResult::BatchRetrieve(matches!(
                    call_type,
                    CallType::AretrieveBatch | CallType::RetrieveBatch
                )))
            }
            BatchRecordsInput::ValidateListLimit { limit } => {
                limit::validate_batch_list_limit(limit)
                    .map(|()| BatchRecordsResult::ListLimitAccepted)
            }
        }
    }
}
