use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::{
    CostMapFetcher, JsonObject, ModelInfoLookup, ModelRegistry, RegistrationEnvironment,
};
use betterlite_models::tiered_pricing::Scalar;

use super::error::{DeploymentError, InvalidDeployment};
use super::params::litellm_params;
use super::ptu;
use super::spec::DeploymentSpec;
use crate::deployment_pricing::PricingInputs;
use crate::deployment_pricing::deployment::Deployment;
use crate::deployment_pricing::error::DeploymentPricingError;
use crate::deployment_pricing::payload::deployment_cost_payload;
use crate::deployment_pricing::register::register_in_cost_map;

// What one entry of the model list hands to _create_deployment.
#[derive(Clone, Debug)]
pub struct DeploymentInput {
    pub model_name: String,
    pub litellm_params: JsonObject,
    pub model_info: JsonObject,
    // the entry's other keys, which Deployment keeps as extra fields
    pub extra: JsonObject,
    pub declared_id: Option<String>,
    pub duplicate_ids: BTreeSet<String>,
}

// The checks on a deployment that other tasks own: access windows, the credential warning and the environment gate.
pub trait ConstructionPolicy: Sync {
    fn check_access_windows(
        &self,
        model_info: &JsonObject,
        model_name: &str,
    ) -> Result<(), InvalidDeployment>;

    fn warn_credential_mismatch(&self, model_name: &str, litellm_params: &JsonObject);

    fn is_active_for_environment(&self, model_info: &JsonObject)
    -> Result<bool, InvalidDeployment>;
}

pub struct Construction<'run, Lookup, Fetcher, Policy> {
    pub registry: &'run mut ModelRegistry,
    pub environment: &'run RegistrationEnvironment<'run, Lookup, Fetcher>,
    pub pricing: &'run PricingInputs,
    pub policy: &'run Policy,
    // the id a deployment gets when its model_info has none
    pub new_id: fn() -> String,
    pub ignore_invalid_deployments: bool,
}

// The deployment when built and active, and whether an ignored error is one LiteLLM queues for a later retry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Created {
    pub deployment: Option<DeploymentSpec>,
    pub provider_unresolved: bool,
}

// The except branch of _create_deployment: raise, or log and skip, answering whether the provider was unresolved.
pub fn ignore_or_raise<Fetch>(
    ignore_invalid_deployments: bool,
    error: DeploymentError<Fetch>,
) -> Result<bool, DeploymentError<Fetch>>
where
    Fetch: Error + 'static,
{
    if !ignore_invalid_deployments {
        return Err(error);
    }
    tracing::error!(
        "Error creating deployment: {error}, ignoring and continuing with other deployments."
    );
    Ok(matches!(
        error,
        DeploymentError::Invalid(InvalidDeployment::ProviderUnresolved { .. })
    ))
}

fn prepare_params<Policy>(
    policy: &Policy,
    ptu_enabled: bool,
    input: &DeploymentInput,
) -> Result<JsonObject, InvalidDeployment>
where
    Policy: ConstructionPolicy,
{
    let config_sourced = !matches!(input.model_info.get("db_model"), Some(Json::Bool(true)));
    let info: BTreeMap<String, Scalar> =
        serde_json::from_value(serde_json::to_value(&input.model_info)?)?;
    if config_sourced
        && ptu_enabled
        && let Some(message) = ptu::config_error(input, &info)
    {
        return Err(InvalidDeployment::Ptu { message });
    }
    policy.check_access_windows(&input.model_info, &input.model_name)?;
    let mut merged = input.litellm_params.clone();
    if config_sourced {
        merged.extend(ptu::zeroed_pricing(input, &info, ptu_enabled)?);
    }
    litellm_params(&merged)
}

// The cost map learns the deployment's pricing under its id and under its backend model.
async fn register_pricing<Lookup, Fetcher, Policy>(
    run: &mut Construction<'_, Lookup, Fetcher, Policy>,
    input: &DeploymentInput,
    spec: &DeploymentSpec,
) -> Result<(), DeploymentPricingError<Fetcher::Error>>
where
    Lookup: ModelInfoLookup,
    Fetcher: CostMapFetcher,
    Policy: ConstructionPolicy,
{
    let mut model_info = input.model_info.clone();
    if let Some(spec_id) = spec.id() {
        model_info.insert("id".to_owned(), Json::Text(spec_id.to_owned()));
    }
    let deployment = Deployment::read(&spec.litellm_params, &model_info)?;
    let mut payload = deployment_cost_payload(
        &spec.litellm_params,
        &model_info,
        run.registry,
        run.environment.lookup,
        run.pricing,
    )
    .await?;
    if !input.model_info.contains_key("id") {
        payload.shift_remove("id");
    }
    register_in_cost_map(
        run.registry,
        run.environment,
        run.pricing,
        &deployment.target,
        &payload,
    )
    .await
}

fn check_tag_regex(spec: &DeploymentSpec) -> Result<(), InvalidDeployment> {
    let Some(configured) = spec.litellm_params.get("tag_regex") else {
        return Ok(());
    };
    let Json::List(entries) = configured else {
        return Err(InvalidDeployment::TagRegexType);
    };
    for entry in entries {
        let Json::Text(pattern) = entry else {
            return Err(InvalidDeployment::TagRegexType);
        };
        fancy_regex::Regex::new(pattern).map_err(|source| InvalidDeployment::TagRegex {
            model_name: spec.model_name.clone(),
            pattern: pattern.clone(),
            source: Box::new(source),
        })?;
    }
    Ok(())
}

async fn build<Lookup, Fetcher, Policy>(
    run: &mut Construction<'_, Lookup, Fetcher, Policy>,
    input: &DeploymentInput,
) -> Result<Option<DeploymentSpec>, DeploymentError<Fetcher::Error>>
where
    Lookup: ModelInfoLookup,
    Fetcher: CostMapFetcher,
    Policy: ConstructionPolicy,
{
    let params = prepare_params(run.policy, run.pricing.ptu_cost_attribution, input)?;
    run.policy
        .warn_credential_mismatch(&input.model_name, &input.litellm_params);
    let spec = DeploymentSpec::new(
        input.model_name.clone(),
        params,
        input.model_info.clone(),
        input.extra.clone(),
        run.new_id,
    )?;
    register_pricing(run, input, &spec).await?;
    if !run.policy.is_active_for_environment(&spec.model_info)? {
        tracing::warn!(
            "Ignoring deployment {} as it is not active for environment {:?}",
            spec.model_name,
            spec.model_info.get("supported_environments"),
        );
        return Ok(None);
    }
    check_tag_regex(&spec)?;
    Ok(Some(spec))
}

// _create_deployment up to its add step: the built deployment, none when excluded, or the skip an invalid one takes.
pub async fn create_deployment<Lookup, Fetcher, Policy>(
    run: &mut Construction<'_, Lookup, Fetcher, Policy>,
    input: &DeploymentInput,
) -> Result<Created, DeploymentError<Fetcher::Error>>
where
    Lookup: ModelInfoLookup,
    Fetcher: CostMapFetcher,
    Policy: ConstructionPolicy,
{
    match build(run, input).await {
        Ok(deployment) => Ok(Created {
            deployment,
            provider_unresolved: false,
        }),
        Err(error) => {
            ignore_or_raise(run.ignore_invalid_deployments, error).map(|provider_unresolved| {
                Created {
                    deployment: None,
                    provider_unresolved,
                }
            })
        }
    }
}
