use std::collections::BTreeSet;
use std::future::{Future, ready};

use betterlite_models::catalog_generalizations::CatalogGeneralizations;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_registration::{
    CostMapFetcher, JsonObject, ModelCostEntries, ModelInfoLookup, ModelRegistry,
    RegistrationEnvironment,
};
use betterlite_router::deployment_identity::construct::{
    Construction, ConstructionPolicy, DeploymentInput, create_deployment,
};
use betterlite_router::deployment_identity::error::InvalidDeployment;
use betterlite_router::deployment_identity::stable_id::generate_model_id;
use betterlite_router::deployment_pricing::PricingInputs;
use serde::{Deserialize, Serialize};
use tokio::runtime::Builder;

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub model_name: String,
    pub litellm_params: JsonObject,
    pub ignore_invalid_deployments: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub id: String,
    pub created: bool,
    pub raised: bool,
}

// The cases carry no access windows, supported environments or credential names, so every hook answers "nothing to flag".
struct NoHooks;

impl ConstructionPolicy for NoHooks {
    fn check_access_windows(&self, _: &JsonObject, _: &str) -> Result<(), InvalidDeployment> {
        Ok(())
    }

    fn warn_credential_mismatch(&self, _: &str, _: &JsonObject) {}

    fn is_active_for_environment(&self, _: &JsonObject) -> Result<bool, InvalidDeployment> {
        Ok(true)
    }
}

// No pricing lookup or cost-map download is involved: the probe deployments name no catalog model.
struct EmptyCatalog;

impl ModelInfoLookup for EmptyCatalog {
    fn get_model_info(
        &self,
        _model: &str,
        _model_cost: &JsonObject,
        _keys: &mut ModelCostKeys,
    ) -> impl Future<Output = Option<JsonObject>> + Send {
        ready(None)
    }

    fn clear_caches(&self) {}
}

#[derive(Debug, thiserror::Error)]
#[error("the probe has no cost map to download")]
struct NoDownload;

impl CostMapFetcher for EmptyCatalog {
    type Error = NoDownload;

    fn get_model_cost_map(
        &self,
        _url: &str,
        _max_attempts: u32,
    ) -> impl Future<Output = Result<ModelCostEntries, NoDownload>> + Send {
        ready(Err(NoDownload))
    }
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let runtime =
        Builder::new_current_thread()
            .build()
            .map_err(|error| OperationError::InvalidRequest {
                message: error.to_string(),
            })?;
    let entry = DeploymentInput {
        model_name: input.model_name.clone(),
        litellm_params: input.litellm_params.clone(),
        model_info: JsonObject::new(),
        extra: JsonObject::new(),
        declared_id: None,
        duplicate_ids: BTreeSet::new(),
    };
    let generalizations = CatalogGeneralizations::default();
    let environment = RegistrationEnvironment {
        lookup: &EmptyCatalog,
        fetcher: &EmptyCatalog,
        generalizations: &generalizations,
        bedrock_base_model: str::to_owned,
    };
    let pricing = PricingInputs {
        ptu_cost_attribution: false,
        is_strategy_router: |model| model.starts_with("auto_router/"),
    };
    let mut registry = ModelRegistry::default();
    let mut construction = Construction {
        registry: &mut registry,
        environment: &environment,
        pricing: &pricing,
        policy: &NoHooks,
        new_id: || "probe-deployment".to_owned(),
        ignore_invalid_deployments: input.ignore_invalid_deployments,
    };
    let created = runtime.block_on(create_deployment(&mut construction, &entry));
    Ok(ProbeOutput::DeploymentIdentity(Output {
        id: generate_model_id(&input.model_name, &input.litellm_params),
        created: matches!(&created, Ok(made) if made.deployment.is_some()),
        raised: created.is_err(),
    }))
}
