use std::collections::BTreeMap;
use std::future::Future;

use betterlite_core::api::responses_write_http::service::ClientKind;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::kinds::SdkResult;

mod native;
mod support;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Delete,
    Cancel,
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

// The four handlers' inputs: the operation and its mode, the id, the typed key and the three overrides
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub operation: Operation,
    pub mode: Mode,
    pub response_id: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub extra_body: Option<IndexMap<String, Json>>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub result: Option<Json>,
    pub failure: Option<Failure>,
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
