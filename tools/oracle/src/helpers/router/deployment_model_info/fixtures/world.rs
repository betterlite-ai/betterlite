use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::catalog_generalizations::CatalogGeneralizations;
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::names::{ModelCostMap, NameNormalizers};
use betterlite_models::model_registration::{JsonObject, ModelCostEntries, ModelRegistry};
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::provider_model_sets::ProviderModelSets;
use betterlite_models::resolution::ResolutionContext;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::deployment_identity::stable_id::generate_model_id;
use betterlite_router::deployment_model_info::cache::DiscoveryCache;
use betterlite_router::deployment_model_info::configured::{
    get_configured_strings, get_configured_token_limits,
};
use betterlite_router::deployment_model_info::deployment_info::{get_model_group, get_model_info};
use betterlite_router::deployment_model_info::discovery::CatalogSnapshot;
use betterlite_router::deployment_model_info::listing::get_model_listing_info;
use betterlite_router::model_catalog_lookup::error::LookupError;
use betterlite_router::model_catalog_lookup::lookup::CatalogLookup;
use serde_json::{Value, json};

use super::super::input::{Arguments, Function, Row};

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(crate) struct Failure(pub(crate) String);

pub(crate) fn argument<'text>(
    value: Option<&'text str>,
    field: &str,
) -> Result<&'text str, Failure> {
    value.ok_or_else(|| Failure(format!("arguments.{field} is required")))
}

// ast-grep-ignore: no-one-line-functions
pub(crate) const fn no_inferred(_model: &str) -> Option<String> {
    None
}
// ast-grep-ignore: no-one-line-functions
const fn split_model(model: &str) -> (Option<&str>, &str) {
    (None, model)
}
pub(crate) const NORMALIZERS: NameNormalizers = NameNormalizers {
    bedrock_base_model: str::to_owned,
    bedrock_routing_prefix: str::to_owned,
    mantle_region_split: split_model,
};
pub(crate) struct Fixture {
    pub(crate) map: ModelCostMap,
    pub(crate) catalog: ModelCatalog,
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) known: BTreeSet<String>,
    pub(crate) custom: BTreeSet<String>,
    pub(crate) registry: ModelRegistry,
    pub(crate) providers: JSONProviderRegistry,
    pub(crate) generalizations: CatalogGeneralizations,
}
impl Fixture {
    pub(crate) fn new(cost: &JsonObject) -> Result<Self, Failure> {
        let bytes = serde_json::to_vec(cost).map_err(|error| Failure(error.to_string()))?;
        let (generalizations, _warnings) = CatalogGeneralizations::install(Vec::new());
        let map = ModelCostMap::parse(&bytes).map_err(|error| Failure(error.to_string()))?;
        let catalog = ModelCatalog::parse(&bytes, UnknownFields::Warn)
            .map_err(|error| Failure(error.to_string()))?;
        let registry = ModelRegistry {
            model_cost: cost.clone(),
            runtime_registered: ModelCostEntries::new(),
            provider_sets: ProviderModelSets::default(),
            keys: ModelCostKeys::default(),
            live_replay: None,
        };
        Ok(Self {
            map,
            catalog,
            environment: BTreeMap::new(),
            known: BTreeSet::new(),
            custom: BTreeSet::new(),
            registry,
            providers: JSONProviderRegistry::load(),
            generalizations,
        })
    }
    pub(crate) fn context(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.known,
            custom_providers: &self.custom,
            environment: &self.environment,
            registry: self.providers.clone(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }
}
// Router.set_model_list gives each row the stable id its name and params derive, then Deployment's defaults.
pub(crate) fn catalog(rows: &[Row]) -> Result<DeploymentCatalog, Failure> {
    let mut catalog = DeploymentCatalog::default();
    for row in rows {
        let params = row.litellm_params.clone().unwrap_or_default();
        let mut model_info = row.model_info.clone();
        model_info
            .entry("id".to_owned())
            .or_insert_with(|| Json::Text(generate_model_id(&row.model_name, &params)));
        let spec = DeploymentSpec::new(
            row.model_name.clone(),
            params,
            model_info,
            JsonObject::new(),
            String::new,
        )
        .map_err(|error| Failure(error.to_string()))?;
        catalog
            .index_patterns(&spec)
            .map_err(|error| Failure(error.to_string()))?;
        catalog.push(spec);
    }
    Ok(catalog)
}
const FIELDS: [&str; 7] = [
    "key",
    "max_tokens",
    "max_input_tokens",
    "max_output_tokens",
    "input_cost_per_token",
    "output_cost_per_token",
    "litellm_provider",
];
// The fixed field set both sides project, so fields one side injects alone never decide a case.
pub(crate) fn project(info: Option<&JsonObject>) -> Value {
    let Some(fields) = info else {
        return Value::Null;
    };
    let picked: JsonObject = FIELDS
        .iter()
        .filter_map(|name| {
            fields
                .get(*name)
                .map(|value| ((*name).to_owned(), value.clone()))
        })
        .collect();
    json!(picked)
}
pub(crate) fn summary(spec: &DeploymentSpec) -> Value {
    let model = spec
        .litellm_params
        .get("model")
        .map_or(Value::Null, |value| json!(value));
    json!({"id": spec.id(), "model_name": &spec.model_name, "model": model})
}
impl From<LookupError> for Failure {
    fn from(error: LookupError) -> Self {
        Failure(error.to_string())
    }
}
pub(crate) fn world_answer(
    engine: &Arc<DeploymentCatalog>,
    discovery: &DiscoveryCache,
    lookup: &CatalogLookup<'_>,
    arguments: &Arguments,
    function: Function,
) -> Result<Value, Failure> {
    let snapshot = CatalogSnapshot { catalog: engine };
    match function {
        Function::Listing => {
            let name = argument(arguments.model_name.as_deref(), "model_name")?;
            match get_model_listing_info(&snapshot, discovery, name)? {
                None => Ok(Value::Null),
                Some(listing_info) => Ok(json!({
                    "cost_map_keys": listing_info.cost_map_keys,
                    "max_input_tokens": listing_info.max_input_tokens,
                    "max_output_tokens": listing_info.max_output_tokens,
                })),
            }
        }
        Function::Configured => {
            let name = argument(arguments.model_name.as_deref(), "model_name")?;
            let limits = get_configured_token_limits(&snapshot, lookup, discovery, name)?;
            let strings = get_configured_strings(lookup, name)?;
            Ok(json!({
                "limits": [limits.0, limits.1],
                "mode": strings.mode,
                "display_name": strings.display_name,
            }))
        }
        Function::Raw => {
            let model_id = argument(arguments.model_id.as_deref(), "model_id")?;
            let found = get_model_info(&snapshot, model_id)?;
            let group = get_model_group(lookup, model_id)?;
            let deployment = found.map(summary);
            let rows: Option<Vec<Value>> =
                group.map(|same_name| same_name.iter().map(summary).collect());
            Ok(json!({"deployment": deployment, "group": rows}))
        }
        Function::DeploymentInfo | Function::RouterInfo | Function::AzureLogs => Err(Failure(
            "this function does not answer from the deployment list".to_owned(),
        )),
    }
}
