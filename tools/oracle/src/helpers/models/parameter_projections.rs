use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::projections::parameter_names::{
    CONTENT_PARAMETER_NAMES, ModelParameters, PROVIDER_CHAT_PARAMETER_NAMES,
    logging_model_parameters,
};
use betterlite_models::projections::reasoning_effort::{
    ThinkingBudgetThresholds, effort_from_thinking_budget,
};
use betterlite_models::projections::service_tier::resolved_service_tier;
use serde::Deserialize;
use serde_json::value::to_raw_value;
use serde_json::{Value, json};
use thiserror::Error;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Step {
    LoggingProjection {
        model_parameters: BTreeMap<String, Value>,
    },
    ExcludedContentNames,
    ProviderChatNames,
    ReasoningEffort {
        budget_tokens: i64,
        #[serde(default)]
        thresholds: ThinkingBudgetThresholds,
    },
    ResolvedServiceTier {
        response_tier: Option<String>,
        usage_tier: Option<String>,
        requested_tier: Option<String>,
    },
}

#[derive(Debug, Error)]
enum RunError {
    #[error(transparent)]
    Parameters(#[from] serde_json::Error),
}

impl From<RunError> for Box<crate::helpers::HelperError> {
    fn from(error: RunError) -> Self {
        Self::new(crate::helpers::HelperError {
            class: "SerializationError".to_owned(),
            status: None,
            error_type: Some(None),
            param: Some(None),
            code: Some(None),
            message: error.to_string(),
        })
    }
}

fn raw_parameters(
    parameters: &BTreeMap<String, Value>,
) -> Result<ModelParameters, serde_json::Error> {
    parameters
        .iter()
        .map(|(name, value)| Ok((name.clone(), to_raw_value(value)?)))
        .collect()
}

impl Step {
    fn run(&self) -> Result<Value, RunError> {
        Ok(match self {
            Self::LoggingProjection { model_parameters } => {
                json!(logging_model_parameters(&raw_parameters(model_parameters)?))
            }
            Self::ExcludedContentNames => {
                json!(
                    CONTENT_PARAMETER_NAMES
                        .iter()
                        .copied()
                        .collect::<BTreeSet<_>>()
                )
            }
            Self::ProviderChatNames => {
                json!(
                    PROVIDER_CHAT_PARAMETER_NAMES
                        .iter()
                        .copied()
                        .collect::<BTreeSet<_>>()
                )
            }
            Self::ReasoningEffort {
                budget_tokens,
                thresholds,
            } => json!(effort_from_thinking_budget(*budget_tokens, *thresholds)),
            Self::ResolvedServiceTier {
                response_tier,
                usage_tier,
                requested_tier,
            } => json!(resolved_service_tier(
                response_tier.as_deref(),
                usage_tier.as_deref(),
                requested_tier.as_deref(),
            )),
        })
    }
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let mut results = Vec::new();
        for step in &input.steps {
            results.push(step.run()?);
        }
        Ok(Value::Array(results))
    }
}
