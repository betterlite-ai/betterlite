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
    get_configured_display_name, get_configured_mode, get_configured_token_limits,
};
use betterlite_router::deployment_model_info::deployment_info::{get_model_group, get_model_info};
use betterlite_router::deployment_model_info::discovery::CatalogSnapshot;
use betterlite_router::deployment_model_info::listing::get_model_listing_info;
use betterlite_router::model_catalog_lookup::lookup::CatalogLookup;
use serde_json::Value;
use serde_json::json;

use super::input::{Arguments, Row};

const FIELDS: [&str; 7] = [
    "key",
    "max_tokens",
    "max_input_tokens",
    "max_output_tokens",
    "input_cost_per_token",
    "output_cost_per_token",
    "litellm_provider",
];

// The ways a case stops before an answer: a refused build, a refused lookup or a missing argument.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(crate) struct Failure(String);

// ast-grep-ignore: no-one-line-functions
pub(crate) fn argument<'value>(
    value: &'value Option<String>,
    name: &'static str,
) -> Result<&'value str, Failure> {
    value
        .as_deref()
        .ok_or_else(|| Failure(format!("arguments.{name} is required")))
}

// ast-grep-ignore: no-one-line-functions
pub(crate) fn no_inferred(_model: &str) -> Option<String> {
    None
}

// ast-grep-ignore: no-one-line-functions
fn split_model(model: &str) -> (Option<&str>, &str) {
    (None, model)
}

pub(crate) const NORMALIZERS: NameNormalizers = NameNormalizers {
    bedrock_base_model: str::to_owned,
    bedrock_routing_prefix: str::to_owned,
    mantle_region_split: split_model,
};

// The static cost map, provider registry and resolution inputs a case declares.
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
        Ok(Self {
            map,
            catalog,
            environment: BTreeMap::new(),
            known: BTreeSet::new(),
            custom: BTreeSet::new(),
            registry: ModelRegistry {
                model_cost: cost.clone(),
                runtime_registered: ModelCostEntries::new(),
                provider_sets: ProviderModelSets::default(),
                keys: ModelCostKeys::default(),
                live_replay: None,
            },
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

// get_router_model_info's supplied deployment, built the way Router.set_model_list builds a row.
pub(crate) struct SuppliedRow {
    pub(crate) spec: DeploymentSpec,
    pub(crate) has_params: bool,
}

pub(crate) fn supplied(row: &Row) -> Result<SuppliedRow, Failure> {
    let params = row.litellm_params.clone().unwrap_or_default();
    let mut model_info = row.model_info.clone();
    if !model_info.contains_key("id") {
        let id = generate_model_id(&row.model_name, &params);
        model_info.insert("id".to_owned(), Json::Text(id));
    }
    let spec = DeploymentSpec::new(
        row.model_name.clone(),
        params,
        model_info,
        JsonObject::new(),
        String::new,
    )
    .map_err(|error| Failure(error.to_string()))?;
    Ok(SuppliedRow {
        spec,
        has_params: row.litellm_params.is_some(),
    })
}

// Router.set_model_list: every row under the catalog indices and wildcard patterns its name carries.
pub(crate) fn catalog(rows: &[Row]) -> Result<DeploymentCatalog, Failure> {
    let mut catalog = DeploymentCatalog::default();
    for row in rows {
        let built = supplied(row)?;
        catalog
            .index_patterns(&built.spec)
            .map_err(|error| Failure(error.to_string()))?;
        catalog.push(built.spec);
    }
    Ok(catalog)
}

// The fixed field set both sides project, so fields one side injects alone never decide a case.
pub(crate) fn project(info: Option<&JsonObject>) -> Value {
    let Some(fields) = info else {
        return Value::Null;
    };
    let mut picked = JsonObject::new();
    for name in FIELDS {
        if let Some(value) = fields.get(name) {
            picked.insert(name.to_owned(), value.clone());
        }
    }
    json!(picked)
}

pub(crate) fn summary(spec: &DeploymentSpec) -> Value {
    let model = match spec.litellm_params.get("model") {
        Some(Json::Text(model)) => Value::from(model.clone()),
        None | Some(_) => Value::Null,
    };
    json!({"id": spec.id(), "model_name": &spec.model_name, "model": model})
}

pub(crate) fn listing(
    engine: &Arc<DeploymentCatalog>,
    discovery: &DiscoveryCache,
    arguments: &Arguments,
) -> Result<Value, Failure> {
    let name = argument(&arguments.model_name, "model_name")?;
    let snapshot = CatalogSnapshot { catalog: engine };
    let info = get_model_listing_info(&snapshot, discovery, name)
        .map_err(|error| Failure(error.to_string()))?;
    Ok(match info {
        None => Value::Null,
        Some(info) => json!({
            "cost_map_keys": info.cost_map_keys,
            "max_input_tokens": info.max_input_tokens,
            "max_output_tokens": info.max_output_tokens,
        }),
    })
}

pub(crate) fn configured(
    engine: &Arc<DeploymentCatalog>,
    discovery: &DiscoveryCache,
    lookup: &CatalogLookup<'_>,
    arguments: &Arguments,
) -> Result<Value, Failure> {
    let name = argument(&arguments.model_name, "model_name")?;
    let snapshot = CatalogSnapshot { catalog: engine };
    let limits = get_configured_token_limits(&snapshot, lookup, discovery, name)
        .map_err(|error| Failure(error.to_string()))?;
    let mode = get_configured_mode(lookup, name).map_err(|error| Failure(error.to_string()))?;
    let display_name =
        get_configured_display_name(lookup, name).map_err(|error| Failure(error.to_string()))?;
    Ok(json!({
        "limits": [limits.0, limits.1],
        "mode": mode,
        "display_name": display_name,
    }))
}

pub(crate) fn raw(
    engine: &Arc<DeploymentCatalog>,
    lookup: &CatalogLookup<'_>,
    arguments: &Arguments,
) -> Result<Value, Failure> {
    let model_id = argument(&arguments.model_id, "model_id")?;
    let snapshot = CatalogSnapshot { catalog: engine };
    let deployment = get_model_info(&snapshot, model_id)
        .map_err(|error| Failure(error.to_string()))?
        .map(summary);
    let group = get_model_group(lookup, model_id)
        .map_err(|error| Failure(error.to_string()))?
        .map(|rows| rows.iter().map(summary).collect::<Vec<Value>>());
    Ok(json!({"deployment": deployment, "group": group}))
}
