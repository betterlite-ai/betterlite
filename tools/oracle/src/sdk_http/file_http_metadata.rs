use std::collections::BTreeMap;
use std::future::Future;

use betterlite_core::api::vector_store_http_write::service::ClientKind;
use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};

use super::OracleLimits;
use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::kinds::SdkResult;

mod native;
mod support;
mod xai;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Retrieve,
    Delete,
    List,
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

// The six handlers' inputs: the operation and its mode, the id or the purpose, and the three overrides
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub operation: Operation,
    pub mode: Mode,
    #[serde(default)]
    pub file_id: Option<String>,
    #[serde(default)]
    pub purpose: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
}

impl Call {
    const fn logging_call_type(&self) -> &'static str {
        match self.operation {
            Operation::Retrieve => "file_retrieve",
            Operation::Delete => "file_delete",
            Operation::List => "file_list",
        }
    }
}

// The exception _handle_error or the response transform raises, as its callers can observe it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    pub status: u16,
    pub synthesized: bool,
    pub message: String,
    pub headers: BTreeMap<String, Json>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub result: Option<Json>,
    pub failure: Option<Failure>,
}

impl CallKind for Call {
    fn validate(&self, _limits: &OracleLimits) -> Result<(), SdkHttpError> {
        match self.operation {
            Operation::Retrieve | Operation::Delete => {
                if self.file_id.as_deref().is_none_or(str::is_empty) {
                    return Err(SdkHttpError::Invalid(String::from(
                        "a retrieve or delete call needs a file_id",
                    )));
                }
                if self.purpose.is_some() {
                    return Err(SdkHttpError::Invalid(String::from(
                        "purpose belongs to a list call",
                    )));
                }
                Ok(())
            }
            Operation::List => {
                if self.file_id.is_some() {
                    return Err(SdkHttpError::Invalid(String::from(
                        "a list call has no file_id",
                    )));
                }
                Ok(())
            }
        }
    }

    fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> impl Future<Output = Result<SdkResult, SdkHttpError>> {
        native::execute(self, api_base, env)
    }
}
