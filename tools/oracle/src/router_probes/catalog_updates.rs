use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::atomic::{AtomicBool, Ordering};

use betterlite_models::catalog_generalizations::CatalogGeneralizations;
use betterlite_models::decisions::Json;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_registration::{
    CostMapFetcher, JsonObject, ModelCostEntries, ModelInfoLookup, ModelRegistry,
    RegistrationEnvironment,
};
use betterlite_router::catalog_updates::catalog::{CatalogHandle, DeploymentCatalog};
use betterlite_router::catalog_updates::entries::{ModelEntry, deployment_inputs};
use betterlite_router::catalog_updates::updater::{CatalogEffects, CatalogUpdater};
use betterlite_router::deployment_identity::construct::{
    Construction, ConstructionPolicy, create_deployment,
};
use betterlite_router::deployment_identity::error::InvalidDeployment;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::deployment_pricing::PricingInputs;
use serde::{Deserialize, Serialize};
use tokio::runtime::{Builder, Runtime};

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub model_name: String,
    pub model: String,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    Set { models: Vec<Entry> },
    Add { model: Entry },
    Upsert { model: Entry },
    Delete { id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub ignore_invalid_deployments: bool,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observed {
    pub raised: bool,
    pub returned: bool,
    pub names: Vec<String>,
    pub ids: Vec<String>,
    pub by_name: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub steps: Vec<Observed>,
}

// Only the provider check answers: a model outside the openai provider is refused, and openai/flaky is refused on its second admission so a restore can fail.
#[derive(Default)]
struct Probe {
    flaky_admitted: AtomicBool,
}

impl ConstructionPolicy for Probe {
    fn check_access_windows(&self, _: &JsonObject, _: &str) -> Result<(), InvalidDeployment> {
        Ok(())
    }

    fn warn_credential_mismatch(&self, _: &str, _: &JsonObject) {}

    fn is_active_for_environment(&self, _: &JsonObject) -> Result<bool, InvalidDeployment> {
        Ok(true)
    }
}

impl ModelInfoLookup for Probe {
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

impl CostMapFetcher for Probe {
    type Error = NoDownload;

    fn get_model_cost_map(
        &self,
        _url: &str,
        _max_attempts: u32,
    ) -> impl Future<Output = Result<ModelCostEntries, NoDownload>> + Send {
        ready(Err(NoDownload))
    }
}

impl CatalogEffects for Probe {
    fn admit(&self, spec: &DeploymentSpec) -> Result<(), InvalidDeployment> {
        match spec.litellm_params.get("model") {
            Some(Json::Text(model)) if model == "openai/flaky" => {
                if self.flaky_admitted.swap(true, Ordering::SeqCst) {
                    return Err(InvalidDeployment::Rejected {
                        message: "the provider stopped answering".to_owned(),
                    });
                }
                Ok(())
            }
            Some(Json::Text(model)) if model.starts_with("openai/") => Ok(()),
            _ => Err(InvalidDeployment::ProviderUnresolved {
                message: "LLM Provider NOT provided".to_owned(),
            }),
        }
    }

    fn registered(&self, _: &DeploymentSpec) {}

    fn released(&self, _: &DeploymentSpec) {}

    fn participates_in_adaptive(&self, _: &DeploymentSpec) -> bool {
        false
    }

    fn finalize_adaptive(&self, _: &DeploymentCatalog) {}

    fn invalidated(&self) {}
}

fn model_entry(entry: &Entry) -> ModelEntry {
    ModelEntry {
        model_name: entry.model_name.clone(),
        litellm_params: JsonObject::from_iter([
            ("model".to_owned(), Json::Text(entry.model.clone())),
            ("api_key".to_owned(), Json::Text("sk-probe".to_owned())),
        ]),
        model_info: JsonObject::from_iter([("id".to_owned(), Json::Text(entry.id.clone()))]),
        extra: JsonObject::new(),
    }
}

const PRICING: PricingInputs = PricingInputs {
    ptu_cost_attribution: false,
    is_strategy_router: |model| model.starts_with("auto_router/"),
};

type ProbeUpdater<'run> = CatalogUpdater<'run, Probe, Probe, Probe, Probe>;

// Runs one step and says whether it raised and whether it returned a deployment.
fn apply(
    runtime: &Runtime,
    updater: &mut ProbeUpdater<'_>,
    step: &Step,
) -> Result<(bool, bool), OperationError> {
    match step {
        Step::Set { models } => {
            let entries: Vec<ModelEntry> = models.iter().map(model_entry).collect();
            Ok((
                runtime.block_on(updater.set_model_list(&entries)).is_err(),
                false,
            ))
        }
        Step::Add { model } | Step::Upsert { model } => {
            let inputs = deployment_inputs(&[model_entry(model)]);
            let created = inputs
                .first()
                .map(|input| runtime.block_on(create_deployment(&mut updater.construction, input)));
            let Some(spec) = created
                .and_then(Result::ok)
                .and_then(|made| made.deployment)
            else {
                return Err(OperationError::InvalidRequest {
                    message: "the entry builds no deployment".to_owned(),
                });
            };
            let outcome = if matches!(step, Step::Add { .. }) {
                runtime.block_on(updater.add_deployment(spec))
            } else {
                runtime.block_on(updater.upsert_deployment(spec))
            };
            Ok((outcome.is_err(), outcome.is_ok_and(|done| done.is_some())))
        }
        Step::Delete { id } => Ok((false, updater.delete_deployment(id).is_some())),
    }
}

// The model list in order, the sorted id index, and the ids each model name indexes.
fn observed(catalog: &DeploymentCatalog) -> Observed {
    let mut ids: Vec<String> = catalog.id_index.keys().cloned().collect();
    ids.sort();
    let by_name = catalog
        .name_index
        .iter()
        .map(|(name, found)| {
            let listed = found
                .iter()
                .filter_map(|index| catalog.models.get(*index)?.id().map(str::to_owned));
            (name.clone(), listed.collect())
        })
        .collect();
    Observed {
        raised: false,
        returned: false,
        names: catalog
            .models
            .iter()
            .map(|spec| spec.model_name.clone())
            .collect(),
        ids,
        by_name,
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
    let handle = CatalogHandle::default();
    let probe = Probe::default();
    let variables: BTreeMap<String, String> = BTreeMap::new();
    let generalizations = CatalogGeneralizations::default();
    let environment = RegistrationEnvironment {
        lookup: &probe,
        fetcher: &probe,
        generalizations: &generalizations,
        bedrock_base_model: str::to_owned,
    };
    let mut registry = ModelRegistry::default();
    let mut steps = Vec::new();
    for step in &input.steps {
        let mut updater = CatalogUpdater {
            catalog: &handle,
            effects: &probe,
            env_vars: &variables,
            construction: Construction {
                registry: &mut registry,
                environment: &environment,
                pricing: &PRICING,
                policy: &probe,
                new_id: || "probe-deployment".to_owned(),
                ignore_invalid_deployments: input.ignore_invalid_deployments,
            },
        };
        let (raised, returned) = apply(&runtime, &mut updater, step)?;
        steps.push(Observed {
            raised,
            returned,
            ..observed(&handle.load_full())
        });
    }
    Ok(ProbeOutput::CatalogUpdates(Output { steps }))
}
