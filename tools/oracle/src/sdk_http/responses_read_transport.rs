use std::collections::BTreeMap;
use std::future::Future;

use betterlite_core::api::responses_read_http::service::ClientKind;
use betterlite_models::decisions::Json;
use betterlite_openai::response_resources::request::{InputItemsOrder, ListInputItemsQuery};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::OracleLimits;
use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::kinds::SdkResult;

mod native;
mod support;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Retrieve,
    InputItems,
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

// The handler arguments the provider transformer turns into the query, whose defaults match limit=20 and order=desc
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Query {
    #[serde(default)]
    pub after: Option<String>,
    #[serde(default)]
    pub before: Option<String>,
    #[serde(default)]
    pub include: Option<Vec<String>>,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default = "default_order")]
    pub order: InputItemsOrder,
}

impl Query {
    fn list_input_items(&self) -> ListInputItemsQuery {
        ListInputItemsQuery {
            after: self.after.clone(),
            before: self.before.clone(),
            include: self.include.clone(),
            limit: Some(self.limit),
            order: Some(self.order),
        }
    }
}

impl Default for Query {
    fn default() -> Self {
        Self {
            after: None,
            before: None,
            include: None,
            limit: default_limit(),
            order: default_order(),
        }
    }
}

const fn default_limit() -> i64 {
    20
}

const fn default_order() -> InputItemsOrder {
    InputItemsOrder::Desc
}

// One handler invocation: the operation and mode, the id, the pagination and the three overrides
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub operation: Operation,
    pub mode: Mode,
    pub response_id: String,
    #[serde(default)]
    pub query: Query,
    #[serde(default)]
    pub extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub extra_body: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub runs: Vec<Run>,
}

// The exception _handle_error raises, as its callers can observe it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    pub status: u16,
    pub synthesized: bool,
    pub message: String,
    pub headers: Option<BTreeMap<String, Json>>,
}

// The typed response's public fields, the part both sides can read without a private member
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Retrieved {
    pub id: String,
    pub created_at: i64,
    pub status: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunOutcome {
    pub retrieved: Option<Retrieved>,
    pub result: Option<Json>,
    pub failure: Option<Failure>,
    pub validation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub runs: Vec<RunOutcome>,
}

impl CallKind for Call {
    fn validate(&self, limits: &OracleLimits) -> Result<(), SdkHttpError> {
        if self.runs.len() > limits.max_requests {
            return Err(SdkHttpError::Invalid(format!(
                "the call runs {} reads, past the request limit of {}",
                self.runs.len(),
                limits.max_requests
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
