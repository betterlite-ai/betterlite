use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_bedrock::mantle_inputs::region::split_region_prefix;
use betterlite_config::cost_adjustments::{
    AdjustmentEntry, AdjustmentKind, CostAdjustmentMap, resolve_adjustment_entries,
};
use betterlite_config::loader::ConfigError;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_core::cost::completion::lookup::{CatalogLookup, LookupError, LookupSettings};
use betterlite_core::cost::image_dispatch::error::ImageDispatchError;
use betterlite_core::cost::image_dispatch::options::SizePattern;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::model_metadata_resolution::names::NameNormalizers;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::provider_model_sets::ProviderModelSets;
use indexmap::IndexMap;
use jiff::tz::TimeZoneDatabase;
use serde::Deserialize;

mod tests;

const DISCOUNT_PATH: &str = "litellm_settings.cost_discount_config";
const MARGIN_PATH: &str = "litellm_settings.cost_margin_config";

// The cost settings Python holds: the price map it can edit at runtime, and the two adjustment maps.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct SettingsWire {
    pub(crate) model_cost: IndexMap<String, Json>,
    pub(crate) cost_discount_config: CostAdjustmentMap,
    pub(crate) cost_margin_config: CostAdjustmentMap,
}

// Everything one cost engine reads besides the per-call arguments, built once from the wire settings.
pub(crate) struct Settings {
    pub(crate) lookup: CatalogLookup,
    pub(crate) pattern: SizePattern,
    pub(crate) models: ProviderModelSets,
    pub(crate) database: TimeZoneDatabase,
    pub(crate) discount: BTreeMap<String, AdjustmentEntry>,
    pub(crate) margin: BTreeMap<String, AdjustmentEntry>,
}

impl Settings {
    pub(crate) fn build(wire: SettingsWire) -> Result<Self, SettingsError> {
        ensure_price_objects(&wire.model_cost)?;
        let bytes = serde_json::to_vec(&wire.model_cost)
            .map_err(|source| SettingsError::Encode { source })?;
        let mut bundled = BundledCatalog::default();
        let catalog: Arc<ModelCatalog> = bundled.get();
        let lookup = CatalogLookup::new(&bytes, Arc::clone(&catalog), lookup_settings())?;
        let mut models = ProviderModelSets::default();
        models.add_known_models(&catalog);
        Ok(Self {
            lookup,
            pattern: SizePattern::new()?,
            models,
            database: TimeZoneDatabase::from_env(),
            discount: resolve_adjustment_entries(
                AdjustmentKind::Discount,
                DISCOUNT_PATH,
                wire.cost_discount_config,
            )?,
            margin: resolve_adjustment_entries(
                AdjustmentKind::Margin,
                MARGIN_PATH,
                wire.cost_margin_config,
            )?,
        })
    }
}

// LiteLLM reads every model_cost entry as a price object, so a scalar entry stops the engine from being built.
fn ensure_price_objects(model_cost: &IndexMap<String, Json>) -> Result<(), SettingsError> {
    for (key, entry) in model_cost {
        if !matches!(entry, Json::Object(_)) {
            return Err(SettingsError::ModelEntry { key: key.clone() });
        }
    }
    Ok(())
}

// The library resolves names with no router, dashboard environment or dynamic provider sources.
fn lookup_settings() -> LookupSettings {
    LookupSettings {
        known_model_names: BTreeSet::new(),
        custom_providers: BTreeSet::new(),
        environment: BTreeMap::new(),
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
        normalizers: NameNormalizers {
            bedrock_base_model: get_bedrock_base_model,
            bedrock_routing_prefix: strip_bedrock_routing_prefix,
            mantle_region_split: split_region_prefix,
        },
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SettingsError {
    #[error("the model cost map could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "the price lookup could not be built: the model_cost entry {key} is not a price object"
    )]
    ModelEntry { key: String },
    #[error("the price lookup could not be built: {0}")]
    Lookup(#[from] LookupError),
    #[error("the image size pattern could not be built: {0}")]
    Pattern(#[from] ImageDispatchError),
    #[error("the cost adjustment settings are not supported: {0}")]
    Adjustments(#[from] ConfigError),
}
