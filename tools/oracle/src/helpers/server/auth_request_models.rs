use std::collections::BTreeSet;

use betterlite_auth::auth_request_models::catalog::{CatalogError, DeploymentRecord, ModelCatalog};
use betterlite_auth::auth_request_models::managed::get_cache_prediction_deployments;
use betterlite_auth::auth_request_models::sources::{
    DispatchContext, request_dispatched_to_pass_through_endpoint,
    request_dispatched_to_provider_pass_through,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::HelperError;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    #[serde(rename = "route")]
    _route: String,
    endpoint_marker: Option<bool>,
    path_params: Vec<String>,
    team_id: Option<String>,
    deployments: Vec<DeploymentInput>,
    current_deployment_id: Option<String>,
    candidate_deployment_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeploymentInput {
    model_name: String,
    #[serde(rename = "litellm_params")]
    _litellm_params: Value,
    model_info: ModelInfo,
}

#[derive(Debug, Deserialize)]
struct ModelInfo {
    id: Option<String>,
    team_id: Option<String>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let steps: Result<Vec<Value>, Box<HelperError>> =
            input.steps.iter().map(run_step).collect();
        Ok(Value::Array(steps?))
    }
}

fn run_step(step: &Step) -> Result<Value, Box<HelperError>> {
    let path_params: BTreeSet<String> = step.path_params.iter().cloned().collect();
    let dispatch = DispatchContext {
        endpoint_marker: step.endpoint_marker,
        path_params: &path_params,
    };
    let catalog = StepCatalog {
        deployments: step
            .deployments
            .iter()
            .map(|deployment| DeploymentRecord {
                model_name: deployment.model_name.clone(),
                custom_llm_provider: None,
                model: None,
                team_id: deployment.model_info.team_id.clone(),
                deployment_id: deployment.model_info.id.clone(),
            })
            .collect(),
    };
    Ok(json!({
        "prediction": prediction(step, &catalog)?,
        "pass_through": request_dispatched_to_pass_through_endpoint(&dispatch),
        "provider_pass_through": request_dispatched_to_provider_pass_through(&dispatch),
    }))
}

fn prediction(step: &Step, catalog: &StepCatalog) -> Result<Value, Box<HelperError>> {
    let (Some(current), Some(candidate)) = (
        step.current_deployment_id.as_deref(),
        step.candidate_deployment_id.as_deref(),
    ) else {
        return Ok(Value::Null);
    };
    if catalog.deployments.is_empty() {
        return Ok(Value::Null);
    }
    let found = get_cache_prediction_deployments(
        current,
        candidate,
        Some(catalog),
        step.team_id.as_deref(),
    )
    .map_err(|error| {
        Box::new(HelperError {
            class: "CatalogError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })?;
    Ok(found.map_or(Value::Null, |(current, candidate)| {
        json!([current.model_name, candidate.model_name])
    }))
}

#[derive(Debug)]
struct StepCatalog {
    deployments: Vec<DeploymentRecord>,
}

impl ModelCatalog for StepCatalog {
    fn resolve_model_name_from_model_id(
        &self,
        _model_id: &str,
    ) -> Result<Option<String>, CatalogError> {
        Ok(None)
    }

    fn deployment(&self, deployment_id: &str) -> Result<Option<DeploymentRecord>, CatalogError> {
        Ok(self
            .deployments
            .iter()
            .find(|deployment| deployment.deployment_id.as_deref() == Some(deployment_id))
            .cloned())
    }

    fn model_names(&self) -> Result<BTreeSet<String>, CatalogError> {
        Ok(BTreeSet::new())
    }

    fn model_list(&self) -> Result<Vec<DeploymentRecord>, CatalogError> {
        Ok(self.deployments.clone())
    }
}
