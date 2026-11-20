use std::collections::BTreeMap;
use std::future::Future;

use betterlite_core::api::vector_file_http_read::service::Operation as ReadOperation;
use betterlite_core::api::vector_store_http_write::service::ClientKind;
use betterlite_models::decisions::Json;
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
pub enum Operation {
    Retrieve,
    Content,
}

impl Operation {
    const fn read_operation(self) -> ReadOperation {
        match self {
            Self::Retrieve => ReadOperation::Retrieve,
            Self::Content => ReadOperation::Content,
        }
    }

    const fn logging_call_type(self) -> &'static str {
        match self {
            Self::Retrieve => "vector_store_file_retrieve",
            Self::Content => "vector_store_file_content",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Sync,
    #[serde(rename = "async")]
    Asynchronous,
}

impl Mode {
    const fn client_kind(self) -> ClientKind {
        match self {
            Self::Sync => ClientKind::Sync,
            Self::Asynchronous => ClientKind::Asynchronous,
        }
    }
}

// The four handlers' inputs: which reads run and in which mode, the two ids and the three overrides
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub mode: Mode,
    pub vector_store_id: String,
    pub file_id: String,
    pub operations: Vec<Operation>,
    #[serde(default)]
    pub extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
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

// Each read's parsed reply in call order, or the provider error that stopped the loop
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub results: Vec<Json>,
    pub failure: Option<Failure>,
}

impl CallKind for Call {
    fn validate(&self, limits: &OracleLimits) -> Result<(), SdkHttpError> {
        if let Some(seconds) = self.timeout_seconds {
            validate_timeout(seconds, limits)?;
        }
        if self.operations.is_empty() {
            return Err(SdkHttpError::Invalid(String::from(
                "a read call runs at least one operation",
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
