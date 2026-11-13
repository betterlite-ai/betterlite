use betterlite_openai::batches::records::{BatchObject, BatchPage};

use super::file_contracts::config::{FileParams, Headers};

pub mod config;
pub mod error;
mod tests;

// operation/tests.rs builds OperationRequest::Batch(BatchRequest) from this bare name, so the marker stays a unit struct
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BatchRequest;

// A create, retrieve or cancel answers with one batch, a list with one cursor page
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchResult {
    Batch(Box<BatchObject>),
    Page(BatchPage),
}

// LiteLLM's `bytes | str | dict[str, Any]` request body
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchBody {
    Bytes(Vec<u8>),
    Text(String),
    Object(FileParams),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchPrepared {
    pub url: String,
    pub headers: Headers,
    pub body: BatchBody,
}
