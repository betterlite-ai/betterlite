use std::future::{Future, ready};

use betterlite_context::bedrock_model::get_bedrock_base_model;
use betterlite_models::catalog_generalizations::CatalogGeneralizations;
use betterlite_models::decisions::Json;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_registration::error::RegistrationWarning;
use betterlite_models::model_registration::{
    CostMapFetcher, JsonObject, LiveDeploymentReplay, LiveRegistration, ModelCostEntries,
    ModelCostSource, ModelInfoLookup, ModelRegistry, RegisterOptions, RegistrationEnvironment,
};
use betterlite_models::provider_model_sets::ProviderModelSets;
use serde::Deserialize;
use serde_json::{Number, Value, json};

use super::project::project;
use crate::helpers::{HelperError, HelperResult, OrHelperError as _};

const COST_FIELDS: [&str; 2] = ["input_cost_per_token", "output_cost_per_token"];

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Registration {
    entries: ModelCostEntries,
    persist_across_reloads: bool,
    warning_display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Scenario {
    model_cost: ModelCostEntries,
    registrations: Vec<Registration>,
    live: Vec<Registration>,
    refresh: Option<ModelCostEntries>,
    show: Vec<String>,
}

struct Replay {
    registrations: Vec<Registration>,
}

impl LiveDeploymentReplay for Replay {
    fn registrations(&self) -> Vec<LiveRegistration> {
        self.registrations
            .iter()
            .map(|registration| LiveRegistration {
                entries: registration.entries.clone(),
                persist_across_reloads: registration.persist_across_reloads,
                warning_display_name: registration.warning_display_name.clone(),
            })
            .collect()
    }
}

// Only an entry that names its provider has model info, which is all the cases need from the lookup.
struct ExactLookup;

impl ModelInfoLookup for ExactLookup {
    fn get_model_info(
        &self,
        model: &str,
        model_cost: &JsonObject,
        _keys: &mut ModelCostKeys,
    ) -> impl Future<Output = Option<JsonObject>> + Send {
        ready(exact_info(model, model_cost))
    }

    fn clear_caches(&self) {}
}

fn exact_info(model: &str, model_cost: &JsonObject) -> Option<JsonObject> {
    let Some(Json::Object(entry)) = model_cost.get(model) else {
        return None;
    };
    if !matches!(entry.get("litellm_provider"), Some(Json::Text(_))) {
        return None;
    }
    let mut info = entry.clone();
    info.insert("key".to_owned(), Json::Text(model.to_owned()));
    for field in COST_FIELDS {
        let slot = info.entry(field.to_owned()).or_insert(Json::Null);
        if matches!(slot, Json::Null) {
            *slot = Json::Number(Number::from(0_u8));
        }
    }
    Some(info)
}

struct NoFetch;

#[derive(Debug, thiserror::Error)]
#[error("a registration in this helper must not fetch a cost map")]
struct NoFetchError;

impl CostMapFetcher for NoFetch {
    type Error = NoFetchError;

    fn get_model_cost_map(
        &self,
        _url: &str,
        _max_attempts: u32,
    ) -> impl Future<Output = Result<ModelCostEntries, NoFetchError>> + Send {
        ready(Err(NoFetchError))
    }
}

type Environment<'env> = RegistrationEnvironment<'env, ExactLookup, NoFetch>;

fn as_object(entries: &ModelCostEntries) -> JsonObject {
    entries
        .iter()
        .map(|(key, entry)| (key.clone(), Json::Object(entry.clone())))
        .collect()
}

fn registry_with(
    model_cost: JsonObject,
    live_replay: Option<Box<dyn LiveDeploymentReplay>>,
) -> ModelRegistry {
    ModelRegistry {
        model_cost,
        runtime_registered: ModelCostEntries::new(),
        provider_sets: ProviderModelSets::default(),
        keys: ModelCostKeys::default(),
        live_replay,
    }
}

pub(super) fn omits_token_price(model_cost: &ModelCostEntries, keys: &[String]) -> Value {
    let registry = registry_with(as_object(model_cost), None);
    let names: Vec<&str> = keys.iter().map(String::as_str).collect();
    Value::Bool(registry.cost_map_omits_token_price(&names))
}

async fn apply(
    registry: &mut ModelRegistry,
    registration: &Registration,
    environment: &Environment<'_>,
) -> Result<Vec<RegistrationWarning>, Box<HelperError>> {
    let options = RegisterOptions {
        persist_across_reloads: registration.persist_across_reloads,
        warning_display_name: registration.warning_display_name.as_deref(),
    };
    let source = ModelCostSource::Entries(registration.entries.clone());
    let registered = registry
        .register_model(source, &options, environment)
        .await
        .or_value_error()?;
    Ok(registered.warnings)
}

async fn run_scenario(
    scenario: &Scenario,
) -> Result<(ModelRegistry, Vec<RegistrationWarning>), Box<HelperError>> {
    let replay = Replay {
        registrations: scenario.live.clone(),
    };
    let mut registry = registry_with(as_object(&scenario.model_cost), Some(Box::new(replay)));
    let generalizations = CatalogGeneralizations::default();
    let environment = RegistrationEnvironment {
        lookup: &ExactLookup,
        fetcher: &NoFetch,
        generalizations: &generalizations,
        bedrock_base_model: get_bedrock_base_model,
    };
    let mut warnings = Vec::new();
    for registration in &scenario.registrations {
        warnings.extend(apply(&mut registry, registration, &environment).await?);
    }
    if let Some(refreshed) = &scenario.refresh {
        registry.model_cost = as_object(refreshed);
        let replayed = registry.reapply_runtime_registrations(&environment).await;
        warnings.extend(replayed.or_value_error()?);
    }
    Ok((registry, warnings))
}

fn shown_entries(registry: &ModelRegistry, show: &[String]) -> Json {
    let mut shown = JsonObject::new();
    for (key, entry) in &registry.model_cost {
        let mut fields = JsonObject::new();
        if let Json::Object(stored) = entry {
            for (field, value) in stored {
                if show.contains(field) && !matches!(value, Json::Null) {
                    fields.insert(field.clone(), value.clone());
                }
            }
        }
        shown.insert(key.clone(), Json::Object(fields));
    }
    Json::Object(shown)
}

pub(super) fn register(scenario: &Scenario) -> HelperResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .or_value_error()?;
    let (registry, warnings) = runtime.block_on(run_scenario(scenario))?;
    let mut sets = serde_json::Map::new();
    for (set, names) in &registry.provider_sets.sets {
        if !names.is_empty() {
            sets.insert(set.to_string(), json!(names));
        }
    }
    let warned: Vec<&str> = warnings
        .iter()
        .map(|warning| match warning {
            RegistrationWarning::MissingCachePricing { model } => model.as_str(),
        })
        .collect();
    Ok(json!({
        "model_cost": project(&shown_entries(&registry, &scenario.show)),
        "provider_sets": sets,
        "warnings": warned,
    }))
}
