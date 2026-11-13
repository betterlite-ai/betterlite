use betterlite_models::decisions::Json;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::static_eligibility::error::EligibilityError;
use betterlite_router::static_eligibility::filter::{
    EligibilityInput, EligibilityResult, EligibilityServices, eligible,
};
use betterlite_router::static_eligibility::request::RequestFacts;
use betterlite_router::static_eligibility::tokens::{TokenPayload, count_for_pre_call_checks};
use serde::{Deserialize, Serialize};

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

mod counter;
mod resolve;

use counter::{ProbeCounter, ProbeLimits, ProbeSupport};
use resolve::resolve;

const API_KEY: &str = "sk-probe";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub model_name: String,
    pub model: String,
    pub id: String,
    #[serde(default)]
    pub base_model: Option<String>,
    #[serde(default)]
    pub max_input_tokens: Option<u64>,
    #[serde(default)]
    pub blocked: Option<bool>,
    #[serde(default)]
    pub supports_web_search: Option<bool>,
    #[serde(default)]
    pub supported_environments: Option<Vec<String>>,
    #[serde(default)]
    pub order: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lookup {
    pub model: String,
    #[serde(default)]
    pub specific_deployment: bool,
    #[serde(default)]
    pub messages: Option<Vec<Json>>,
    #[serde(default)]
    pub tools: Vec<Json>,
    #[serde(default)]
    pub response_format: bool,
    #[serde(default)]
    pub target_order: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub environment: String,
    pub enable_pre_call_checks: bool,
    #[serde(default)]
    pub drop_params: bool,
    pub models: Vec<Entry>,
    pub lookups: Vec<Lookup>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Selected,
    BadRequest,
    RateLimited,
    ContextExceeded,
    ServiceUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Found {
    pub outcome: Outcome,
    pub selected: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub lookups: Vec<Found>,
}

fn ids(deployments: &[DeploymentSpec]) -> Vec<String> {
    let mut selected = Vec::new();
    for deployment in deployments {
        selected.push(deployment.id().unwrap_or_default().to_owned());
    }
    selected
}

async fn answer(
    input: &Input,
    lookup: &Lookup,
    counter: &ProbeCounter,
) -> Result<Found, OperationError> {
    let Some(candidates) = resolve(input, lookup)? else {
        return Ok(Found {
            outcome: Outcome::BadRequest,
            selected: Vec::new(),
        });
    };
    let mut facts = RequestFacts {
        messages: lookup.messages.clone(),
        system: None,
        tools: lookup.tools.clone(),
        response_format: lookup.response_format,
        target_order: lookup.target_order,
        input_token_count: None,
        compaction_pending: false,
        drop_params: input.drop_params,
    };
    if input.enable_pre_call_checks && facts.messages.is_some() {
        let payload = TokenPayload {
            messages: facts.messages.as_deref(),
            system: None,
            tools: &facts.tools,
        };
        facts.input_token_count = count_for_pre_call_checks(
            &lookup.model,
            candidates.deployments(),
            &payload,
            &ProbeLimits,
            counter,
        )
        .await;
    }
    let services = EligibilityServices {
        limits: &ProbeLimits,
        params: &ProbeSupport,
    };
    let outcome = match eligible(
        EligibilityInput {
            model: &lookup.model,
            candidates,
            facts: &facts,
            environment: Some(&input.environment),
            enable_pre_call_checks: input.enable_pre_call_checks,
        },
        &services,
    ) {
        Ok(EligibilityResult { deployments, .. }) if deployments.is_empty() => Found {
            outcome: Outcome::RateLimited,
            selected: Vec::new(),
        },
        Ok(EligibilityResult { deployments, .. }) => Found {
            outcome: Outcome::Selected,
            selected: ids(&deployments),
        },
        Err(EligibilityError::Blocked { .. }) => Found {
            outcome: Outcome::ServiceUnavailable,
            selected: Vec::new(),
        },
        Err(EligibilityError::ContextWindow(_error)) => Found {
            outcome: Outcome::ContextExceeded,
            selected: Vec::new(),
        },
        Err(EligibilityError::Environment(error)) => {
            return Err(OperationError::InvalidRequest {
                message: error.to_string(),
            });
        }
        Err(EligibilityError::WebSearch(error)) => {
            return Err(OperationError::InvalidRequest {
                message: error.to_string(),
            });
        }
    };
    Ok(outcome)
}

async fn execute(input: &Input) -> Result<Output, OperationError> {
    let counter = ProbeCounter::new()?;
    let mut lookups = Vec::new();
    for lookup in &input.lookups {
        lookups.push(answer(input, lookup, &counter).await?);
    }
    Ok(Output { lookups })
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| OperationError::InvalidRequest {
            message: error.to_string(),
        })?;
    let output = runtime.block_on(execute(input))?;
    Ok(ProbeOutput::StaticEligibility(output))
}
