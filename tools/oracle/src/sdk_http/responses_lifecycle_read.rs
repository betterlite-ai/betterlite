use std::collections::BTreeMap;
use std::future::Future;

use betterlite_core::api::responses_lifecycle_call::input::{
    ResponsesInputItemsCall, ResponsesRetrieveCall,
};
use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};

use super::OracleLimits;
use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::Failure;
use super::result::kinds::SdkResult;

mod native;
mod support;

// One case drives the four SDK reads, so a retrieve and its input items share the mock and its answers
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub steps: Vec<Step>,
}

// The step names are the SDK function names, so a case reads like the call it scripts
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    GetResponses(RetrieveStep),
    AgetResponses(RetrieveStep),
    ListInputItems(InputItemsStep),
    AlistInputItems(InputItemsStep),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetrieveStep {
    pub call: ResponsesRetrieveCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputItemsStep {
    pub call: ResponsesInputItemsCall,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub steps: Vec<StepOutcome>,
}

// A step answers one of the three: the rewritten response, the provider's raw item list, or the SDK exception
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepOutcome {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retrieved: Option<Retrieved>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Json>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<Failure>,
}

// The response fields a caller reads off ResponsesAPIResponse, without the typed output items
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Retrieved {
    pub id: String,
    pub created_at: i64,
    pub status: Option<String>,
    pub model: Option<String>,
}

impl CallKind for Call {
    fn validate(&self, limits: &OracleLimits) -> Result<(), SdkHttpError> {
        if self.steps.is_empty() {
            return Err(SdkHttpError::Invalid(
                "a responses lifecycle read call needs at least one step".to_owned(),
            ));
        }
        if self.steps.len() > limits.max_requests {
            return Err(SdkHttpError::Invalid(format!(
                "a responses lifecycle read call has {} steps, past the limit of {}",
                self.steps.len(),
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
