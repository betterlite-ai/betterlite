use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_bedrock::mantle_inputs::region::split_region_prefix;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_core::cost::completion::lookup::{CatalogLookup, LookupError, LookupSettings};
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::model_metadata_resolution::names::NameNormalizers;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use indexmap::IndexMap;
use jiff::tz::TimeZoneDatabase;
use serde::Deserialize;

// NativeCost's settings text, including the two adjustment maps this family reads nothing from.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct SettingsWire {
    pub(crate) model_cost: IndexMap<String, Json>,
    pub(crate) cost_discount_config: Option<Json>,
    pub(crate) cost_margin_config: Option<Json>,
}

// The price map and the zone database the extra calculators read, built once from that text.
pub(crate) struct Settings {
    pub(crate) lookup: CatalogLookup,
    pub(crate) database: TimeZoneDatabase,
}

impl Settings {
    pub(crate) fn build(wire: SettingsWire) -> Result<Self, SettingsError> {
        ensure_price_objects(&wire.model_cost)?;
        let bytes = serde_json::to_vec(&wire.model_cost)
            .map_err(|source| SettingsError::Encode { source })?;
        let mut bundled = BundledCatalog::default();
        let catalog: Arc<ModelCatalog> = bundled.get();
        Ok(Self {
            lookup: CatalogLookup::new(&bytes, catalog, lookup_settings())?,
            database: TimeZoneDatabase::from_env(),
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
}
