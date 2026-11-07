use betterlite_router::wildcard_catalog::{Candidate, PatternCatalog};
use serde::{Deserialize, Serialize};

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    Add {
        pattern: String,
        model: String,
        model_id: Option<String>,
    },
    Remove {
        model_id: String,
    },
    Route {
        request: String,
        filtered_model_names: Option<Vec<String>>,
    },
    GetPattern {
        model: String,
        provider: Option<String>,
    },
    GetDeployments {
        model: String,
        provider: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resolved {
    pub model: String,
    pub model_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub routes: Vec<Option<Vec<Resolved>>>,
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    match execute(input) {
        Ok(output) => Ok(ProbeOutput::WildcardCatalog(output)),
        Err(error) => Err(error),
    }
}

pub(crate) fn execute(input: &Input) -> Result<Output, OperationError> {
    let mut catalog = PatternCatalog::default();
    let mut routes = Vec::new();
    for step in &input.steps {
        match step {
            Step::Add {
                pattern,
                model,
                model_id,
            } => {
                let candidate = Candidate {
                    model_id: model_id.clone(),
                    model: model.clone(),
                };
                catalog.add_pattern(pattern, candidate).map_err(|error| {
                    OperationError::InvalidRequest {
                        message: error.to_string(),
                    }
                })?;
            }
            Step::Remove { model_id } => catalog.remove_deployment(model_id),
            Step::Route {
                request,
                filtered_model_names,
            } => {
                let matched = catalog.route(request, filtered_model_names.as_deref());
                routes.push(matched.map(resolved));
            }
            Step::GetPattern { model, provider } => {
                let matched = catalog.get_pattern(model, provider.as_deref());
                routes.push(matched.map(resolved));
            }
            Step::GetDeployments { model, provider } => {
                let matched = catalog.get_deployments_by_pattern(model, provider.as_deref());
                routes.push(Some(resolved(matched)));
            }
        }
    }
    Ok(Output { routes })
}

fn resolved(candidates: Vec<Candidate>) -> Vec<Resolved> {
    candidates
        .into_iter()
        .map(|candidate| Resolved {
            model: candidate.model,
            model_id: candidate.model_id,
        })
        .collect()
}
