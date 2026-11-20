use std::collections::BTreeMap;
use std::future::Future;

use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};

use super::OracleLimits;
use super::call_checks::validate_timeout;
use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::Failure;
use super::result::kinds::SdkResult;

mod native;
mod support;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Sync,
    #[serde(rename = "async")]
    Asynchronous,
}

impl Mode {
    // `if _is_async is True`: the sync mode passes a boolean that fails that identity check too
    pub(super) const fn request_flag(self) -> Json {
        match self {
            Self::Sync => Json::Bool(false),
            Self::Asynchronous => Json::Bool(true),
        }
    }
}

// The handler's inputs: the modes it runs, the model, the optional params, the key and the timeout
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub model: String,
    pub modes: Vec<Mode>,
    #[serde(default)]
    pub optional_params: BTreeMap<String, Json>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
}

impl CallKind for Call {
    fn validate(&self, limits: &OracleLimits) -> Result<(), SdkHttpError> {
        if self.modes.is_empty() {
            return Err(SdkHttpError::Invalid(
                "the rerank call needs at least one mode".to_owned(),
            ));
        }
        if let Some(seconds) = self.timeout_seconds {
            validate_timeout(seconds, limits)?;
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

// What the pre-call callback saw: the query it logged and the body LiteLLM stored for it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreCallSeen {
    pub input: Json,
    pub body: Json,
}

// The arguments the response transformer received, and the answer it was handed
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransformSeen {
    pub model: String,
    pub api_key: Option<String>,
    pub request_data: Json,
    pub optional_params: Json,
    pub litellm_params: Json,
    pub raw_status: u16,
    pub raw_body: Json,
}

// One call of the handler in one mode: its record or failure, the callback order, and what the hooks saw
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunOutcome {
    pub result: Option<Json>,
    pub failure: Option<Failure>,
    pub events: Vec<String>,
    pub pre_call: Option<PreCallSeen>,
    pub transform: Option<TransformSeen>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub runs: Vec<RunOutcome>,
}
