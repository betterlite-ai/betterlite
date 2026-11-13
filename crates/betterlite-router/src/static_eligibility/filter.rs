use betterlite_models::decisions::Json;
use betterlite_models::deployment_filters::{OrderedDeployment, get_order_filtered_deployments};

use crate::deployment_catalog::types::Candidates;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::router_predicates::params::ParamSupport;
use crate::static_eligibility::environment::active_in_environment;
use crate::static_eligibility::error::EligibilityError;
use crate::static_eligibility::params::filter_supported_params;
use crate::static_eligibility::request::RequestFacts;
use crate::static_eligibility::tokens::{TokenLimits, filter_context_window};
use crate::static_eligibility::web_search::filter_web_search;

// Why a candidate left the pool; the empty result tells the caller which filter took the last one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cause {
    Environment {
        model_name: String,
    },
    WebSearch {
        model_name: String,
    },
    ContextWindow {
        model_name: String,
        max_input_tokens: usize,
        input_tokens: usize,
    },
    UnsupportedParameter {
        model_name: String,
        param: String,
    },
}

pub struct EligibilityInput<'input> {
    pub model: &'input str,
    pub candidates: Candidates,
    pub facts: &'input RequestFacts,
    pub environment: Option<&'input str>,
    pub enable_pre_call_checks: bool,
}

pub struct EligibilityServices<'services> {
    pub limits: &'services dyn TokenLimits,
    pub params: &'services dyn ParamSupport,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EligibilityResult {
    pub deployments: Vec<DeploymentSpec>,
    pub causes: Vec<Cause>,
}

fn order_of(value: Option<&Json>) -> Option<i64> {
    match value {
        Some(Json::Number(number)) => number.as_i64(),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

impl OrderedDeployment for DeploymentSpec {
    fn litellm_params_order(&self) -> Option<i64> {
        order_of(self.litellm_params.get("order"))
    }

    fn model_info_order(&self) -> Option<i64> {
        order_of(self.model_info.get("order"))
    }
}

// A deployment chosen by its id comes back as one dict: LiteLLM only checks that it is not paused and skips every other filter.
fn single_eligibility(
    model: &str,
    mut active: Vec<DeploymentSpec>,
    causes: Vec<Cause>,
) -> Result<EligibilityResult, EligibilityError> {
    let Some(deployment) = active.pop() else {
        return Ok(EligibilityResult {
            deployments: Vec::new(),
            causes,
        });
    };
    if matches!(deployment.model_info.get("blocked"), Some(Json::Bool(true))) {
        return Err(EligibilityError::Blocked {
            model: model.to_owned(),
        });
    }
    Ok(EligibilityResult {
        deployments: vec![deployment],
        causes,
    })
}

// The static half of async_get_healthy_deployments: the environment gate at construction, then the filters a request applies.
pub fn eligible(
    input: EligibilityInput<'_>,
    services: &EligibilityServices<'_>,
) -> Result<EligibilityResult, EligibilityError> {
    let single = matches!(&input.candidates, Candidates::Single(_));
    let mut causes = Vec::new();
    let deployments = match input.candidates {
        Candidates::Single(deployment) => vec![*deployment],
        Candidates::Pool(pool) => pool,
    };
    let mut active = Vec::new();
    for deployment in deployments {
        if active_in_environment(&deployment.model_info, input.environment)? {
            active.push(deployment);
        } else {
            causes.push(Cause::Environment {
                model_name: deployment.model_name.clone(),
            });
        }
    }
    if single {
        return single_eligibility(input.model, active, causes);
    }
    let mut deployments = filter_web_search(active, &input.facts.tools, &mut causes)?;
    if input.enable_pre_call_checks && input.facts.messages.is_some() {
        if !input.facts.compaction_pending {
            deployments = filter_context_window(
                input.model,
                deployments,
                input.facts.input_token_count,
                services.limits,
                &mut causes,
            )?;
        }
        deployments = filter_supported_params(
            deployments,
            input.facts.response_format,
            input.facts.drop_params,
            services.params,
            &mut causes,
        );
    }
    let deployments = get_order_filtered_deployments(deployments, input.facts.target_order);
    Ok(EligibilityResult {
        deployments,
        causes,
    })
}
