use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::resource_sdk_settings::Settings;
use betterlite_core::api::fine_tuning_read::service::FineTuningRead;
use betterlite_core::api::fine_tuning_read::{
    FineTuningListCall, FineTuningReadError, FineTuningReadInput, FineTuningReadOutcome,
    FineTuningRetrieveCall,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use self::http::ReadWireError;
use super::OracleLimits;
use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::Failure;
use super::result::kinds::SdkResult;

mod http;

// one case drives several list, alist, retrieve and aretrieve calls so a read sequence shares a mock
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    List(ListStep),
    #[serde(rename = "alist")]
    AList(ListStep),
    Retrieve(RetrieveStep),
    #[serde(rename = "aretrieve")]
    ARetrieve(RetrieveStep),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListStep {
    pub call: FineTuningListCall,
    // LiteLLM only hands a supplied client to its openai handler
    #[serde(default)]
    pub supplied_client: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetrieveStep {
    pub call: FineTuningRetrieveCall,
    #[serde(default)]
    pub supplied_client: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub steps: Vec<StepOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepOutcome {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<Failure>,
}

impl CallKind for Call {
    fn validate(&self, limits: &OracleLimits) -> Result<(), SdkHttpError> {
        if self.steps.is_empty() {
            return Err(SdkHttpError::Invalid(
                "a fine-tuning read call needs at least one step".to_owned(),
            ));
        }
        if self.steps.len() > limits.max_requests {
            return Err(SdkHttpError::Invalid(format!(
                "a fine-tuning read call has {} steps, past the limit of {}",
                self.steps.len(),
                limits.max_requests
            )));
        }
        Ok(())
    }

    async fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> Result<SdkResult, SdkHttpError> {
        let settings = environment_settings(env)?;
        let outbound = http::outbound(env)?;
        let mut steps = Vec::with_capacity(self.steps.len());
        for step in &self.steps {
            let (mut input, supplied) = step.input();
            // the harness owns the mock's base, so a step without one takes its place
            set_api_base(&mut input, api_base);
            let dispatch = http::HttpDispatch {
                client: outbound.clone(),
                supplied,
            };
            let read = FineTuningRead {
                dispatch,
                settings: settings.clone(),
            };
            steps.push(outcome(read.run(&input).await)?);
        }
        Ok(SdkResult::FineTuningRead {
            value: Box::new(Outcome { steps }),
        })
    }
}

impl Step {
    fn input(&self) -> (FineTuningReadInput, bool) {
        match self {
            Self::List(step) => (
                FineTuningReadInput::List(Box::new(step.call.clone())),
                step.supplied_client,
            ),
            Self::AList(step) => (
                FineTuningReadInput::AList(Box::new(step.call.clone())),
                step.supplied_client,
            ),
            Self::Retrieve(step) => (
                FineTuningReadInput::Retrieve(Box::new(step.call.clone())),
                step.supplied_client,
            ),
            Self::ARetrieve(step) => (
                FineTuningReadInput::ARetrieve(Box::new(step.call.clone())),
                step.supplied_client,
            ),
        }
    }
}

// the call's own base is what the SDK receives; without one the mock's base stands in
fn set_api_base(input: &mut FineTuningReadInput, api_base: &str) {
    let params = match input {
        FineTuningReadInput::List(call) | FineTuningReadInput::AList(call) => &mut call.params,
        FineTuningReadInput::Retrieve(call) | FineTuningReadInput::ARetrieve(call) => {
            &mut call.params
        }
    };
    if params.api_base.is_none() {
        params.api_base = Some(api_base.to_owned());
    }
}

fn environment_settings(env: &BTreeMap<String, String>) -> Result<Settings, SdkHttpError> {
    let loader = ConfigLoader {
        lookup: env.clone(),
    };
    let config = loader
        .load("model_list: []")
        .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    Ok(config.sections.resource_sdk_settings)
}

fn outcome(
    result: Result<FineTuningReadOutcome, FineTuningReadError<ReadWireError>>,
) -> Result<StepOutcome, SdkHttpError> {
    match result {
        Ok(FineTuningReadOutcome::Page(page)) => Ok(StepOutcome {
            page: Some(serde_json::to_value(page).map_err(SdkHttpError::Serialize)?),
            job: None,
            failure: None,
        }),
        Ok(FineTuningReadOutcome::Job(job)) => Ok(StepOutcome {
            page: None,
            job: Some(serde_json::to_value(job).map_err(SdkHttpError::Serialize)?),
            failure: None,
        }),
        Err(error) => Ok(StepOutcome {
            page: None,
            job: None,
            failure: Some(failure(&error)),
        }),
    }
}

fn failure(error: &FineTuningReadError<ReadWireError>) -> Failure {
    match error {
        FineTuningReadError::Dispatch(ReadWireError::Sdk(source)) => Failure {
            class: class_for_status(source.status).to_owned(),
            status: source.status,
            kind: Some(source.error_type.clone()),
            param: Some(source.param.clone()),
            code: Some(source.code.clone()),
            message: source.message.clone(),
        },
        // list is undecorated and retrieve carries @client, but both re-raise the provider error
        FineTuningReadError::UnsupportedList { .. }
        | FineTuningReadError::UnsupportedRetrieve { .. } => Failure {
            class: "BadRequestError".to_owned(),
            status: 400,
            kind: Some(None),
            param: Some(None),
            code: Some(None),
            message: format!("litellm.BadRequestError: {error}"),
        },
        FineTuningReadError::Timeout(_) => plain_failure("ValueError", error),
        FineTuningReadError::Decode(_) => plain_failure("ValidationError", error),
        FineTuningReadError::Page { .. } => plain_failure("APIError", error),
        FineTuningReadError::Dispatch(_) => plain_failure("APIConnectionError", error),
    }
}

fn plain_failure<E>(class: &str, error: &E) -> Failure
where
    E: std::fmt::Display,
{
    Failure {
        class: class.to_owned(),
        status: 0,
        kind: None,
        param: None,
        code: None,
        message: error.to_string(),
    }
}

// the classes the openai client raises for an HTTP status
const fn class_for_status(status: u16) -> &'static str {
    match status {
        400 => "BadRequestError",
        401 => "AuthenticationError",
        403 => "PermissionDeniedError",
        404 => "NotFoundError",
        409 => "ConflictError",
        422 => "UnprocessableEntityError",
        429 => "RateLimitError",
        500..=599 => "InternalServerError",
        _ => "APIStatusError",
    }
}
