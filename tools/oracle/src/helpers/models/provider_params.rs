use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::params::capabilities::{
    CapabilityQuery, ModelProjection, OpenAiCapability, OpenAiConfigTag, ParameterCapabilities,
    supported_parameters,
};
use betterlite_models::params::{
    CallType, ChatParameters, ParameterError, ParameterPolicy, ParameterRequest, map_parameters,
};
use betterlite_models::projections::parameter_names::{ModelParameters, logging_model_parameters};
use betterlite_models::projections::reasoning_effort::{
    ThinkingBudgetThresholds, effort_from_thinking_budget,
};
use betterlite_models::projections::service_tier::resolved_service_tier;
use serde::Deserialize;
use serde_json::value::to_raw_value;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Step {
    SupportedParams {
        query: Query,
    },
    MapParameters {
        query: Query,
        parameters: Box<ChatParameters>,
        #[serde(default)]
        policy: ParameterPolicy,
    },
    ReasoningEffort {
        budget_tokens: i64,
    },
    ResolvedServiceTier {
        response_tier: Option<String>,
        usage_tier: Option<String>,
        requested_tier: Option<String>,
    },
    LoggingProjection {
        model_parameters: BTreeMap<String, Value>,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Query {
    provider: ProviderId,
    call_type: CallType,
    model: String,
    config: OpenAiConfigTag,
    #[serde(default)]
    capabilities: BTreeSet<OpenAiCapability>,
    #[serde(default)]
    base_model: Option<String>,
    #[serde(default)]
    base_capabilities: Option<BTreeSet<OpenAiCapability>>,
}

impl Query {
    fn capability_query(&self) -> CapabilityQuery<'_> {
        CapabilityQuery {
            provider: self.provider,
            call_type: self.call_type,
            config: self.config,
            model: ModelProjection {
                model: &self.model,
                capabilities: &self.capabilities,
            },
            base_model: self.base_model.as_ref().map(|model| ModelProjection {
                model,
                capabilities: self
                    .base_capabilities
                    .as_ref()
                    .unwrap_or(&self.capabilities),
            }),
        }
    }
}

impl Step {
    fn run(&self) -> HelperResult {
        match self {
            Self::SupportedParams { query } => {
                Ok(match supported_parameters(&query.capability_query()) {
                    ParameterCapabilities::Listed(names) => json!(names),
                    ParameterCapabilities::Absent => Value::Null,
                })
            }
            Self::MapParameters {
                query,
                parameters,
                policy,
            } => {
                let mapping = map_parameters(&ParameterRequest {
                    capabilities: query.capability_query(),
                    parameters,
                    policy,
                })
                .map_err(|error| parameter_error(&error))?;
                Ok(json!(mapping))
            }
            Self::ReasoningEffort { budget_tokens } => Ok(json!(effort_from_thinking_budget(
                *budget_tokens,
                ThinkingBudgetThresholds::default(),
            ))),
            Self::ResolvedServiceTier {
                response_tier,
                usage_tier,
                requested_tier,
            } => Ok(json!(resolved_service_tier(
                response_tier.as_deref(),
                usage_tier.as_deref(),
                requested_tier.as_deref(),
            ))),
            Self::LoggingProjection { model_parameters } => Ok(json!(logging_model_parameters(
                &raw_parameters(model_parameters)?
            ))),
        }
    }
}

fn raw_parameters(source: &BTreeMap<String, Value>) -> Result<ModelParameters, serde_json::Error> {
    let mut parameters = ModelParameters::new();
    for (name, value) in source {
        parameters.insert(name.clone(), to_raw_value(value)?);
    }
    Ok(parameters)
}

fn parameter_error(error: &ParameterError) -> Box<HelperError> {
    let status = if matches!(error, ParameterError::Encode { .. }) {
        500
    } else {
        400
    };
    Box::new(HelperError {
        class: String::from("ParameterError"),
        status: Some(status),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut results = Vec::with_capacity(input.steps.len());
        for step in &input.steps {
            results.push(step.run()?);
        }
        Ok(Value::Array(results))
    }
}
