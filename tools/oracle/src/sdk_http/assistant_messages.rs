pub mod execute;
pub mod failure;

use std::collections::BTreeMap;
use std::future::Future;

use betterlite_prompts::arguments::json::Json;
use serde::{Deserialize, Serialize};

use super::OracleLimits;
use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::Failure;
use super::result::kinds::SdkResult;

// One case runs the same wrapper inputs several times against one mock origin
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub runs: Vec<Run>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Sync,
    Async,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Append,
    List,
}

// the wrapper's positional and keyword inputs, with its own client keyword refused
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub mode: Mode,
    pub operation: Operation,
    pub provider: String,
    pub thread_id: String,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub content: Option<Json>,
    #[serde(default)]
    pub attachments: Option<Json>,
    #[serde(default)]
    pub metadata: Option<Json>,
    // GenericLiteLLMParams and the keywords the wrappers pop, sent on as written (main.py#L892-L1011)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Json>,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub drop_params: bool,
    // the wrapper's own client keyword; both sides build a client from the case credential instead
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<Json>,
}

// the process globals litellm.api_key and litellm.api_version that the wrappers fall back to
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub api_key: Option<String>,
    pub api_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub runs: Vec<RunOutcome>,
}

// exactly one of the two, so an empty value and a failure stay distinguishable
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunOutcome {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<Failure>,
}

impl CallKind for Call {
    fn validate(&self, _limits: &OracleLimits) -> Result<(), SdkHttpError> {
        validate(self)
    }

    fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> impl Future<Output = Result<SdkResult, SdkHttpError>> {
        execute::send(self, api_base, env)
    }
}

fn validate(call: &Call) -> Result<(), SdkHttpError> {
    if call.runs.is_empty() {
        return Err(SdkHttpError::Invalid(
            "the assistant messages call carries no runs".to_owned(),
        ));
    }
    for run in &call.runs {
        validate_run(run)?;
    }
    Ok(())
}

fn validate_run(run: &Run) -> Result<(), SdkHttpError> {
    if run.provider.is_empty() || run.thread_id.is_empty() {
        return Err(SdkHttpError::Invalid(
            "a message run needs a provider and a thread_id".to_owned(),
        ));
    }
    if run.client.is_some() {
        return Err(SdkHttpError::Invalid(
            "a message run supplies no client; the case api_base builds it".to_owned(),
        ));
    }
    let data = run.role.is_some()
        || run.content.is_some()
        || run.attachments.is_some()
        || run.metadata.is_some();
    match run.operation {
        Operation::Append if run.role.is_none() || run.content.is_none() => {
            return Err(SdkHttpError::Invalid(
                "an append run needs a role and a content".to_owned(),
            ));
        }
        Operation::List if data => {
            return Err(SdkHttpError::Invalid(
                "a list run passes no message data".to_owned(),
            ));
        }
        Operation::Append | Operation::List => {}
    }
    Ok(())
}
